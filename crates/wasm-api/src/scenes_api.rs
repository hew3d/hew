//! Scenes and the section plane across the wasm boundary
//! (docs/design/scenes.md §3.3). A second `#[wasm_bindgen] impl Scene`
//! block, kept out of `lib.rs` for size; it reaches the handle's private
//! `doc` like any child module.
//!
//! Boundary shapes: structured values cross as JSON strings (the Library
//! precedent — `scenes_json`, camera/display in and out, drift), because
//! the app already speaks these exact JSON shapes (`getCameraState()`
//! returns `{projection, fovDeg, eye, target, up}`); the renderer-level
//! hidden LEAF ids cross as `Vec<u64>` (BigUint64Array) like every other
//! handle list — a u64 slotmap handle does not survive JSON. Scene stable
//! ids are small counter values and ride JSON as numbers.
//!
//! Property masks: bit 1 camera, 2 hidden objects, 4 visible tags,
//! 8 section plane, 16 display (`PROP_*` below).

use kernel::{
    CameraProjection, CameraState, DisplayState, NodeId, Point3, ResolvedScene, SceneProps,
    SectionPlaneState, Vec3,
};
use slotmap::Key;
use wasm_bindgen::prelude::*;

use crate::{ApiError, Scene, doc_err, recording};

pub(crate) const PROP_CAMERA: u8 = 1;
pub(crate) const PROP_HIDDEN_NODES: u8 = 2;
pub(crate) const PROP_HIDDEN_TAGS: u8 = 4;
pub(crate) const PROP_SECTION: u8 = 8;
pub(crate) const PROP_DISPLAY: u8 = 16;

pub(crate) fn props_from_mask(mask: u8) -> SceneProps {
    SceneProps {
        camera: mask & PROP_CAMERA != 0,
        hidden_nodes: mask & PROP_HIDDEN_NODES != 0,
        hidden_tags: mask & PROP_HIDDEN_TAGS != 0,
        section: mask & PROP_SECTION != 0,
        display: mask & PROP_DISPLAY != 0,
    }
}

pub(crate) fn mask_from_props(p: SceneProps) -> u8 {
    (if p.camera { PROP_CAMERA } else { 0 })
        | (if p.hidden_nodes { PROP_HIDDEN_NODES } else { 0 })
        | (if p.hidden_tags { PROP_HIDDEN_TAGS } else { 0 })
        | (if p.section { PROP_SECTION } else { 0 })
        | (if p.display { PROP_DISPLAY } else { 0 })
}

fn xyz(v: &serde_json::Value, what: &str) -> Result<[f64; 3], ApiError> {
    let arr = v
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or_else(|| ApiError(format!("BadVector: {what} must be an xyz triple")))?;
    let mut out = [0.0; 3];
    for (i, x) in arr.iter().enumerate() {
        out[i] = x
            .as_f64()
            .filter(|f| f.is_finite())
            .ok_or_else(|| ApiError(format!("BadVector: {what} must be finite numbers")))?;
    }
    Ok(out)
}

/// `{projection, fovDeg, eye, target, up}` → [`CameraState`].
pub(crate) fn camera_from_json(json: &str) -> Result<CameraState, ApiError> {
    let v: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| ApiError(format!("BadCamera: malformed camera json: {e}")))?;
    let projection = match v["projection"].as_str() {
        Some("perspective") => CameraProjection::Perspective,
        Some("parallel") => CameraProjection::Parallel,
        other => {
            return Err(ApiError(format!(
                "BadProjection: unknown projection {other:?}"
            )));
        }
    };
    let fov_deg = v["fovDeg"]
        .as_f64()
        .filter(|f| f.is_finite())
        .ok_or_else(|| ApiError("BadCamera: fovDeg must be a finite number".to_string()))?;
    let eye = xyz(&v["eye"], "eye")?;
    let target = xyz(&v["target"], "target")?;
    let up = xyz(&v["up"], "up")?;
    let (eye, target, up) = (
        Point3::new(eye[0], eye[1], eye[2]),
        Point3::new(target[0], target[1], target[2]),
        Vec3::new(up[0], up[1], up[2]),
    );
    // The parallel height: as given, else what the perspective view spans
    // at the target (a camera recorded before the field existed).
    let parallel_height = match v.get("parallelHeight") {
        None | Some(serde_json::Value::Null) => CameraState::height_at_target(eye, target, fov_deg),
        Some(h) => h
            .as_f64()
            .filter(|h| h.is_finite() && *h > 0.0)
            .ok_or_else(|| {
                ApiError("BadCamera: parallelHeight must be a finite positive number".to_string())
            })?,
    };
    // The Match Photo image width (meters) and ratio, when the camera
    // holds them; absent or null is unset.
    let positive = |key: &str| -> Result<Option<f64>, ApiError> {
        match v.get(key) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(x) => x
                .as_f64()
                .filter(|x| x.is_finite() && *x > 0.0)
                .map(Some)
                .ok_or_else(|| {
                    ApiError(format!("BadCamera: {key} must be a finite positive number"))
                }),
        }
    };
    Ok(CameraState {
        projection,
        fov_deg,
        parallel_height,
        eye,
        target,
        up,
        image_width: positive("imageWidth")?,
        aspect_ratio: positive("aspectRatio")?,
    })
}

/// `{background, sky, ground, axes}` → [`kernel::ViewStyle`].
pub(crate) fn view_style_from_json(json: &str) -> Result<kernel::ViewStyle, ApiError> {
    let v: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| ApiError(format!("BadViewStyle: malformed view style json: {e}")))?;
    let rgb = |x: &serde_json::Value, what: &str| -> Result<[u8; 3], ApiError> {
        let a = x
            .as_array()
            .filter(|a| a.len() == 3)
            .ok_or_else(|| ApiError(format!("BadViewStyle: {what} must be [r, g, b]")))?;
        let mut out = [0u8; 3];
        for (o, c) in out.iter_mut().zip(a) {
            *o = c
                .as_u64()
                .and_then(|c| u8::try_from(c).ok())
                .ok_or_else(|| ApiError(format!("BadViewStyle: {what} must be 0..=255 each")))?;
        }
        Ok(out)
    };
    let opt = |x: &serde_json::Value, what: &str| -> Result<Option<[u8; 3]>, ApiError> {
        match x {
            serde_json::Value::Null => Ok(None),
            x => rgb(x, what).map(Some),
        }
    };
    // The optional section and guide settings; absent or null is unset.
    let flag = |key: &str| -> Result<Option<bool>, ApiError> {
        match &v[key] {
            serde_json::Value::Null => Ok(None),
            x => x
                .as_bool()
                .map(Some)
                .ok_or_else(|| ApiError(format!("BadViewStyle: {key} must be a boolean"))),
        }
    };
    let width = match &v["sectionCutWidth"] {
        serde_json::Value::Null => None,
        x => Some(
            x.as_u64()
                .and_then(|w| u32::try_from(w).ok())
                .ok_or_else(|| {
                    ApiError(
                        "BadViewStyle: sectionCutWidth must be a non-negative integer".to_string(),
                    )
                })?,
        ),
    };
    Ok(kernel::ViewStyle {
        background: rgb(&v["background"], "background")?,
        sky: opt(&v["sky"], "sky")?,
        ground: opt(&v["ground"], "ground")?,
        axes: v["axes"]
            .as_bool()
            .ok_or_else(|| ApiError("BadViewStyle: axes must be a boolean".to_string()))?,
        section: kernel::SectionLook {
            cut_color: opt(&v["sectionCutColor"], "sectionCutColor")?,
            cut_width: width,
            fill: flag("sectionFill")?,
            fill_color: opt(&v["sectionFillColor"], "sectionFillColor")?,
            active_color: opt(&v["activeSectionColor"], "activeSectionColor")?,
        },
        guides_color: opt(&v["guidesColor"], "guidesColor")?,
        guides_hidden: flag("guidesHidden")?,
    })
}

pub(crate) fn view_style_to_json(v: &kernel::ViewStyle) -> serde_json::Value {
    let mut out = serde_json::json!({
        "background": v.background,
        "sky": v.sky,
        "ground": v.ground,
        "axes": v.axes,
    });
    // The section and guide settings, present only when set.
    let s = &v.section;
    for (key, value) in [
        ("sectionCutColor", s.cut_color.map(|c| serde_json::json!(c))),
        ("sectionCutWidth", s.cut_width.map(|w| serde_json::json!(w))),
        ("sectionFill", s.fill.map(|f| serde_json::json!(f))),
        (
            "sectionFillColor",
            s.fill_color.map(|c| serde_json::json!(c)),
        ),
        (
            "activeSectionColor",
            s.active_color.map(|c| serde_json::json!(c)),
        ),
        ("guidesColor", v.guides_color.map(|c| serde_json::json!(c))),
        (
            "guidesHidden",
            v.guides_hidden.map(|h| serde_json::json!(h)),
        ),
    ] {
        if let Some(value) = value {
            out[key] = value;
        }
    }
    out
}

pub(crate) fn camera_to_json(c: &CameraState) -> serde_json::Value {
    let mut v = serde_json::json!({
        "projection": match c.projection {
            CameraProjection::Perspective => "perspective",
            CameraProjection::Parallel => "parallel",
        },
        "fovDeg": c.fov_deg,
        "parallelHeight": c.parallel_height,
        "eye": [c.eye.x, c.eye.y, c.eye.z],
        "target": [c.target.x, c.target.y, c.target.z],
        "up": [c.up.x, c.up.y, c.up.z],
    });
    // Present only when set, so a camera without them reads as before.
    if let Some(w) = c.image_width {
        v["imageWidth"] = w.into();
    }
    if let Some(r) = c.aspect_ratio {
        v["aspectRatio"] = r.into();
    }
    v
}

/// `{grid, axes, guides}` → [`DisplayState`].
pub(crate) fn display_from_json(json: &str) -> Result<DisplayState, ApiError> {
    let v: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| ApiError(format!("BadDisplay: malformed display json: {e}")))?;
    let flag = |k: &str| {
        v[k].as_bool()
            .ok_or_else(|| ApiError(format!("BadDisplay: {k} must be a boolean")))
    };
    Ok(DisplayState {
        grid: flag("grid")?,
        axes: flag("axes")?,
        guides: flag("guides")?,
    })
}

pub(crate) fn display_to_json(d: &DisplayState) -> serde_json::Value {
    serde_json::json!({ "grid": d.grid, "axes": d.axes, "guides": d.guides })
}

pub(crate) fn section_to_json(p: &SectionPlaneState) -> serde_json::Value {
    serde_json::json!({
        "origin": [p.origin.x, p.origin.y, p.origin.z],
        "normal": [p.normal.x, p.normal.y, p.normal.z],
        "active": p.active,
    })
}

fn scene_to_json(s: &kernel::Scene) -> serde_json::Value {
    let mut v = serde_json::json!({
        "sid": s.sid,
        "name": s.name,
        "description": s.description,
        "props": mask_from_props(s.props()),
        "transitionS": s.transition_s,
        "delayS": s.delay_s,
    });
    if let Some(c) = &s.camera {
        v["camera"] = camera_to_json(c);
    }
    if let Some(d) = &s.display {
        v["display"] = display_to_json(d);
    }
    if let Some(style) = &s.view_style {
        v["viewStyle"] = view_style_to_json(style);
    }
    if let Some(section) = &s.section {
        v["section"] = match section {
            None => serde_json::Value::Null,
            Some(p) => section_to_json(p),
        };
    }
    v
}

fn opt_camera(json: Option<String>) -> Result<Option<CameraState>, ApiError> {
    json.as_deref().map(camera_from_json).transpose()
}

fn opt_display(json: Option<String>) -> Result<Option<DisplayState>, ApiError> {
    json.as_deref().map(display_from_json).transpose()
}

/// What applying (or resolving) a Scene means for the app — the JS view of
/// `kernel::ResolvedScene` (docs/design/scenes.md §3.1). Getters instead of
/// one JSON blob because the leaf-id lists must cross as BigUint64Array.
#[wasm_bindgen]
pub struct ResolvedSceneJs {
    inner: ResolvedScene,
}

#[wasm_bindgen]
impl ResolvedSceneJs {
    /// `{projection, fovDeg, eye, target, up}` JSON, or `undefined` when the
    /// Scene does not capture the camera.
    pub fn camera_json(&self) -> Option<String> {
        self.inner
            .camera
            .as_ref()
            .map(|c| camera_to_json(c).to_string())
    }

    /// `{grid, axes, guides}` JSON, or `undefined` when not captured.
    pub fn display_json(&self) -> Option<String> {
        self.inner
            .display
            .as_ref()
            .map(|d| display_to_json(d).to_string())
    }

    /// The view style to show (`Scene::view_style_json`'s shape), or
    /// `undefined` when the Scene captures none; the document's own is
    /// already set to it.
    pub fn view_style_json(&self) -> Option<String> {
        self.inner
            .view_style
            .as_ref()
            .map(|v| view_style_to_json(v).to_string())
    }

    /// True when the Scene captures either hidden property — the leaf-id
    /// lists below are then authoritative (an empty list means "hide
    /// nothing"). False → leave the current hidden state alone.
    pub fn has_hidden(&self) -> bool {
        self.inner.hidden_object_ids.is_some()
    }

    /// Renderer-level hidden object handles (leaf-expanded union of hidden
    /// tags and hidden nodes) — feed straight to `ViewportApi.setHidden`.
    pub fn hidden_object_ids(&self) -> Vec<u64> {
        self.inner
            .hidden_object_ids
            .as_ref()
            .map(|v| v.iter().map(|id| id.data().as_ffi()).collect())
            .unwrap_or_default()
    }

    /// Renderer-level hidden instance handles.
    pub fn hidden_instance_ids(&self) -> Vec<u64> {
        self.inner
            .hidden_instance_ids
            .as_ref()
            .map(|v| v.iter().map(|id| id.data().as_ffi()).collect())
            .unwrap_or_default()
    }

    /// True when the Scene captures the visible-tags property.
    pub fn has_hidden_tags(&self) -> bool {
        self.inner.hidden_tag_paths.is_some()
    }

    /// Hidden tag paths, `/`-joined (the app's tag-panel state).
    pub fn hidden_tag_paths(&self) -> Vec<String> {
        self.inner
            .hidden_tag_paths
            .as_ref()
            .map(|v| v.iter().map(|p| p.join("/")).collect())
            .unwrap_or_default()
    }

    /// True when the Scene captures the hidden-objects property.
    pub fn has_hidden_nodes(&self) -> bool {
        self.inner.hidden_nodes.is_some()
    }

    /// User-hidden node kinds (0 object, 1 group, 2 instance), parallel to
    /// [`ResolvedSceneJs::hidden_node_ids`] — the app's outliner state.
    pub fn hidden_node_kinds(&self) -> Vec<u8> {
        self.inner
            .hidden_nodes
            .as_ref()
            .map(|v| {
                v.iter()
                    .map(|n| match n {
                        NodeId::Object(_) => 0,
                        NodeId::Group(_) => 1,
                        NodeId::Instance(_) => 2,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn hidden_node_ids(&self) -> Vec<u64> {
        self.inner
            .hidden_nodes
            .as_ref()
            .map(|v| {
                v.iter()
                    .map(|n| match n {
                        NodeId::Object(id) => id.data().as_ffi(),
                        NodeId::Group(id) => id.data().as_ffi(),
                        NodeId::Instance(id) => id.data().as_ffi(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// True when the Scene captures the section property; then
    /// [`ResolvedSceneJs::section_json`] is authoritative (`undefined` =
    /// captured with no plane).
    pub fn has_section(&self) -> bool {
        self.inner.section.is_some()
    }

    /// `{origin, normal, active}` JSON when the Scene captures a plane.
    pub fn section_json(&self) -> Option<String> {
        match &self.inner.section {
            Some(Some(p)) => Some(section_to_json(p).to_string()),
            _ => None,
        }
    }
}

#[wasm_bindgen]
impl Scene {
    // -------------------------------------------------------- section plane

    /// The document's section plane as `{origin, normal, active}` JSON, or
    /// `undefined` when none is placed (docs/design/scenes.md §4). View
    /// state, not undoable, persisted (manifest v16).
    pub fn section_plane_json(&self) -> Option<String> {
        self.doc
            .section_plane()
            .map(|p| section_to_json(&p).to_string())
    }

    /// Places or moves the section plane. `normal` must be unit length and
    /// point at the side the cut removes (`sectionManager.ts` convention);
    /// a non-unit normal is a typed refusal, never normalized.
    #[allow(clippy::too_many_arguments)]
    pub fn set_section_plane(
        &mut self,
        ox: f64,
        oy: f64,
        oz: f64,
        nx: f64,
        ny: f64,
        nz: f64,
        active: bool,
    ) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        self.doc
            .set_section_plane(SectionPlaneState {
                origin: Point3::new(ox, oy, oz),
                normal: Vec3::new(nx, ny, nz),
                active,
            })
            .map_err(doc_err)?;
        recording::record(recording::RecordedCall::SetSectionPlane {
            origin: [ox, oy, oz],
            normal: [nx, ny, nz],
            active,
        });
        Ok(())
    }

    /// Removes the section plane.
    pub fn clear_section_plane(&mut self) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        self.doc.clear_section_plane();
        recording::record(recording::RecordedCall::ClearSectionPlane);
        Ok(())
    }

    // ----------------------------------------------------------- view style

    /// How the document asks to be shown (`kernel::ViewStyle`, manifest
    /// v19): `{background: [r, g, b], sky: [r, g, b] | null, ground:
    /// [r, g, b] | null, axes: bool}` as JSON, or `undefined` for Hew's own
    /// look. A `.skp` opened from SketchUp carries its style's backdrop
    /// and axes here; the app applies it on load.
    pub fn view_style_json(&self) -> Option<String> {
        self.doc
            .view_style()
            .map(|v| view_style_to_json(&v).to_string())
    }

    /// Sets the document's view style from the JSON `view_style_json`
    /// returns, or clears it (`undefined`/`null`) for Hew's own look. View
    /// state, not undoable, like `set_camera_state`.
    pub fn set_view_style_json(&mut self, json: Option<String>) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        let style = json.as_deref().map(view_style_from_json).transpose()?;
        self.doc.set_view_style(style);
        recording::record(recording::RecordedCall::SetViewStyle { json });
        Ok(())
    }

    /// Records whether the model axes are shown in the document's view
    /// style, when it has one (View ▸ Axes on a document opened from a
    /// `.skp`, so a save back writes the choice); a document shown Hew's
    /// own way keeps none. Not undoable.
    pub fn set_view_style_axes(&mut self, axes: bool) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        if let Some(mut v) = self.doc.view_style()
            && v.axes != axes
        {
            v.axes = axes;
            self.doc.set_view_style(Some(v));
            recording::record(recording::RecordedCall::SetViewStyle {
                json: Some(view_style_to_json(&v).to_string()),
            });
        }
        Ok(())
    }

    // --------------------------------------------------------------- scenes

    /// Every Scene in tab order as a JSON array of
    /// `{sid, name, description, props, camera?, display?, section?}` —
    /// `props` is the capture bitmask (1 camera, 2 hidden objects, 4 visible
    /// tags, 8 section, 16 display); `camera`/`display`/`section` are present
    /// only when captured, and `section` is `null` for "captured, no plane".
    /// The hidden sets themselves stay kernel-side (resolve them via
    /// [`Scene::resolve_scene`]).
    pub fn scenes_json(&self) -> String {
        let list: Vec<serde_json::Value> = self.doc.scenes().iter().map(scene_to_json).collect();
        serde_json::Value::Array(list).to_string()
    }

    /// The auto-name Add Scene would use next ("Scene N").
    pub fn next_scene_name(&self) -> String {
        self.doc.next_scene_name()
    }

    /// Adds a Scene capturing the current view state under `props`
    /// (bitmask); `camera_json` / `display_json` are the app's live values
    /// (`{projection, fovDeg, eye, target, up}` / `{grid, axes, guides}`),
    /// stored only when their bit is set. `name` `undefined` auto-names;
    /// `after` `undefined` appends. Returns the new Scene's stable id. Not
    /// undoable; the caller marks the document dirty.
    pub fn add_scene(
        &mut self,
        name: Option<String>,
        props: u8,
        camera_json: Option<String>,
        display_json: Option<String>,
        after: Option<u64>,
    ) -> Result<u64, ApiError> {
        self.ensure_not_busy()?;
        let camera = opt_camera(camera_json.clone())?;
        let display = opt_display(display_json.clone())?;
        let sid = self
            .doc
            .add_scene(name.clone(), props_from_mask(props), camera, display, after)
            .map_err(doc_err)?;
        recording::record(recording::RecordedCall::AddScene {
            name,
            props,
            camera_json,
            display_json,
            after,
        });
        Ok(sid)
    }

    /// Re-captures the properties in `props` (bitmask) into an existing
    /// Scene; unflagged properties are left as they were.
    pub fn update_scene(
        &mut self,
        sid: u64,
        props: u8,
        camera_json: Option<String>,
        display_json: Option<String>,
    ) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        let camera = opt_camera(camera_json.clone())?;
        let display = opt_display(display_json.clone())?;
        self.doc
            .update_scene(sid, props_from_mask(props), camera, display)
            .map_err(doc_err)?;
        recording::record(recording::RecordedCall::UpdateScene {
            sid,
            props,
            camera_json,
            display_json,
        });
        Ok(())
    }

    /// Sets which properties a Scene captures (bitmask): newly-on ones are
    /// captured now, newly-off ones drop their data.
    pub fn set_scene_props(
        &mut self,
        sid: u64,
        props: u8,
        camera_json: Option<String>,
        display_json: Option<String>,
    ) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        let camera = opt_camera(camera_json.clone())?;
        let display = opt_display(display_json.clone())?;
        self.doc
            .set_scene_props(sid, props_from_mask(props), camera, display)
            .map_err(doc_err)?;
        recording::record(recording::RecordedCall::SetSceneProps {
            sid,
            props,
            camera_json,
            display_json,
        });
        Ok(())
    }

    /// Renames a Scene (`DuplicateSceneName` / `EmptySceneName` are typed
    /// refusals surfaced as `ApiError`).
    pub fn rename_scene(&mut self, sid: u64, name: String) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        self.doc.rename_scene(sid, name.clone()).map_err(doc_err)?;
        recording::record(recording::RecordedCall::RenameScene { sid, name });
        Ok(())
    }

    pub fn set_scene_description(&mut self, sid: u64, text: String) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        self.doc
            .set_scene_description(sid, text.clone())
            .map_err(doc_err)?;
        recording::record(recording::RecordedCall::SetSceneDescription { sid, text });
        Ok(())
    }

    /// Sets a Scene's own transition time in seconds (finite, not
    /// negative); `undefined` returns it to the document's own. Not
    /// undoable, like the Scene's other fields.
    pub fn set_scene_transition(&mut self, sid: u64, seconds: Option<f64>) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        self.doc
            .set_scene_transition(sid, seconds)
            .map_err(doc_err)?;
        recording::record(recording::RecordedCall::SetSceneTransition { sid, seconds });
        Ok(())
    }

    /// Sets a Scene's own delay in seconds; as [`Scene::set_scene_transition`].
    pub fn set_scene_delay(&mut self, sid: u64, seconds: Option<f64>) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        self.doc.set_scene_delay(sid, seconds).map_err(doc_err)?;
        recording::record(recording::RecordedCall::SetSceneDelay { sid, seconds });
        Ok(())
    }

    /// Moves a Scene to `index` in tab order (clamped).
    pub fn move_scene(&mut self, sid: u64, index: u32) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        self.doc.move_scene(sid, index as usize).map_err(doc_err)?;
        recording::record(recording::RecordedCall::MoveScene { sid, index });
        Ok(())
    }

    /// Deletes a Scene (not undoable — the UI confirms first).
    pub fn remove_scene(&mut self, sid: u64) -> Result<(), ApiError> {
        self.ensure_not_busy()?;
        self.doc.remove_scene(sid).map_err(doc_err)?;
        recording::record(recording::RecordedCall::RemoveScene { sid });
        Ok(())
    }

    /// What applying a Scene would do, without mutating anything — Shop
    /// Mode's path (docs/design/scenes.md §6).
    pub fn resolve_scene(&self, sid: u64) -> Result<ResolvedSceneJs, ApiError> {
        let inner = self.doc.resolve_scene(sid).map_err(doc_err)?;
        Ok(ResolvedSceneJs { inner })
    }

    /// Applies a Scene's captured kernel-side state (tag visibility, user-
    /// hidden nodes, section plane) and returns the resolution for the app
    /// to finish (camera, display, renderer leaf sets). Non-undoable, not a
    /// dirtying change. The app pushes the returned leaf ids into
    /// [`Scene::set_hidden`] itself, exactly as its hide toggles do.
    pub fn apply_scene(&mut self, sid: u64) -> Result<ResolvedSceneJs, ApiError> {
        self.ensure_not_busy()?;
        let inner = self.doc.apply_scene(sid).map_err(doc_err)?;
        // The next event drain reports it as the active Scene.
        self.last_active_scene = Some(sid);
        // A Scene sets tag visibility: a guide hidden or shown by its
        // tags snaps accordingly at once.
        self.reregister_guides();
        recording::record(recording::RecordedCall::ApplyScene { sid });
        Ok(ResolvedSceneJs { inner })
    }

    /// Which captured properties no longer match the live document, as JSON
    /// `{camera, hiddenNodes, hiddenTags, section, display, staleRefs}` —
    /// the first five booleans, `staleRefs` a count. `camera_json` /
    /// `display_json` are the app's live values (`undefined` = don't compare
    /// that property).
    pub fn scene_drift(
        &self,
        sid: u64,
        camera_json: Option<String>,
        display_json: Option<String>,
    ) -> Result<String, ApiError> {
        let camera = opt_camera(camera_json)?;
        let display = opt_display(display_json)?;
        let d = self
            .doc
            .scene_drift(sid, camera.as_ref(), display.as_ref())
            .map_err(doc_err)?;
        Ok(serde_json::json!({
            "camera": d.props.camera,
            "hiddenNodes": d.props.hidden_nodes,
            "hiddenTags": d.props.hidden_tags,
            "section": d.props.section,
            "display": d.props.display,
            "staleRefs": d.stale_refs,
        })
        .to_string())
    }
}

#[cfg(test)]
mod display_json_tests {
    use super::*;

    /// A style's section and guide settings cross the JSON boundary both
    /// ways, present only when set, so a style without them reads as before.
    #[test]
    fn a_view_styles_section_and_guide_settings_round_trip_through_json() {
        let json = r#"{"background":[1,2,3],"sky":null,"ground":null,"axes":true,
            "sectionCutColor":[200,0,10],"sectionCutWidth":5,"sectionFill":true,
            "sectionFillColor":[4,5,6],"activeSectionColor":[0,200,0],
            "guidesColor":[0,0,255],"guidesHidden":false}"#;
        let style = view_style_from_json(json).unwrap();
        assert_eq!(style.section.cut_color, Some([200, 0, 10]));
        assert_eq!(style.section.cut_width, Some(5));
        assert_eq!(style.section.fill, Some(true));
        assert_eq!(style.guides_color, Some([0, 0, 255]));
        assert_eq!(style.guides_hidden, Some(false));
        let back = view_style_to_json(&style);
        assert_eq!(view_style_from_json(&back.to_string()).unwrap(), style);

        let plain =
            view_style_from_json(r#"{"background":[1,2,3],"sky":null,"ground":null,"axes":true}"#)
                .unwrap();
        assert!(plain.section.is_unset() && plain.guides_color.is_none());
        let out = view_style_to_json(&plain);
        assert!(out.get("sectionCutColor").is_none() && out.get("guidesHidden").is_none());

        assert!(
            view_style_from_json(
                r#"{"background":[1,2,3],"sky":null,"ground":null,"axes":true,"sectionCutWidth":-1}"#
            )
            .is_err(),
            "a negative width is refused"
        );
    }

    /// A camera's image width and ratio cross as `imageWidth` and
    /// `aspectRatio`, present only when set.
    #[test]
    fn a_cameras_image_width_and_ratio_round_trip_through_json() {
        let json = r#"{"projection":"perspective","fovDeg":35,"eye":[4,-6,3],
            "target":[0,0,0],"up":[0,0,1],"imageWidth":25.4,"aspectRatio":1.5}"#;
        let cam = camera_from_json(json).unwrap();
        assert_eq!((cam.image_width, cam.aspect_ratio), (Some(25.4), Some(1.5)));
        let back = camera_to_json(&cam);
        assert_eq!(camera_from_json(&back.to_string()).unwrap(), cam);
        let plain = camera_from_json(
            r#"{"projection":"parallel","fovDeg":35,"eye":[4,-6,3],"target":[0,0,0],"up":[0,0,1]}"#,
        )
        .unwrap();
        assert_eq!((plain.image_width, plain.aspect_ratio), (None, None));
        let out = camera_to_json(&plain);
        assert!(out.get("imageWidth").is_none() && out.get("aspectRatio").is_none());
        assert!(
            camera_from_json(
                r#"{"projection":"parallel","fovDeg":35,"eye":[4,-6,3],"target":[0,0,0],"up":[0,0,1],"imageWidth":0}"#
            )
            .is_err()
        );
    }
}
