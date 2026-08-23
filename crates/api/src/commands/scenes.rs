//! Scenes: `hew.scenes.*` (docs/agents/HEW_API.md's Scenes section;
//! docs/design/scenes.md §3, §7). A Scene is a named, saved view of the
//! document — the camera, the hidden-object set, the hidden-tag set, the
//! section plane, and the editor's display toggles — captured and
//! restored in one step.
//!
//! Like `hew.tag.create`/`hew.tag.set_visible` (structure.rs's sibling
//! `hew.material`/`hew.tag` block), every command here except `list` is
//! `CommandClass::ModelMutating` — so it may ride a transaction and the
//! registry marks `mutates_document = true` — but NONE of it is
//! *undoable*: `kernel::scenes` never calls a recorded kernel op for any
//! of this (`Document::add_scene`/`update_scene`/`apply_scene`/… all
//! write plain fields), so the compound entry `transact.rs`'s bracket
//! commits ends up empty and no undo entry lands, exactly like a tag
//! registration or a visibility toggle. This is a kernel-side fact, not
//! something this module has to enforce — it just calls the kernel API,
//! which already has the right posture (see `crates/kernel/src/
//! scenes.rs`'s own module doc: "Everything here is view state, outside
//! undo history").
//!
//! `hew.scenes.apply` is the one command that also touches
//! [`crate::host::Host`]: after `Document::apply_scene` has already
//! written the kernel-side state (tag/node hidden flags, section plane),
//! it calls [`crate::host::Host::scene_applied`] so a live host can react
//! (drive the camera, tell its own Scene-tray UI which Scene is now
//! active) — a best-effort notification, not a second source of truth:
//! `NoHost`/`CliHost` do nothing extra, and the document mutation already
//! happened whether or not any host cares.

use super::camera::{RawCamera, parse_camera_or_view, to_kernel_camera};
use super::{CmdError, Ctx, Handler};
use crate::ids;
use crate::refusal::Refusal;
use kernel::EntityRef;
use serde::Deserialize;
use serde_json::{Value, json};

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    match name {
        "hew.scenes.list" => Some(list),
        "hew.scenes.add" => Some(add),
        "hew.scenes.update" => Some(update),
        "hew.scenes.rename" => Some(rename),
        "hew.scenes.describe" => Some(describe),
        "hew.scenes.remove" => Some(remove),
        "hew.scenes.reorder" => Some(reorder),
        "hew.scenes.apply" => Some(apply),
        _ => None,
    }
}

fn parse<T: for<'de> Deserialize<'de>>(params: &Value) -> Result<T, CmdError> {
    serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))
}

/// A Scene id that fails to parse, or does not name a live Scene in this
/// document — the same machine name either way (`unknown_scene`), snake-
/// cased identically to the kernel's own `DocumentError::UnknownScene`
/// (`refusal.rs`'s automatic variant-name derivation), so a client never
/// sees two different names for "that Scene doesn't exist."
pub(crate) fn unknown_scene(id: &str) -> CmdError {
    CmdError::Refusal(
        Refusal::api(
            "unknown_scene",
            &format!("'{id}' does not name a Scene in this document."),
        )
        .with_detail(json!({ "id": id })),
    )
}

/// Parses a Scene public id into its stable id. Whether that sid actually
/// names a *live* Scene is left to the kernel call the caller makes next
/// (`Document::scene`/`add_scene`/… all answer `UnknownScene`, which maps
/// to the identical `unknown_scene` refusal `unknown_scene` above mints
/// for a parse failure) — one refusal name for both cases.
pub(crate) fn resolve_scene_id(id: &str) -> Result<u64, CmdError> {
    ids::resolve_scene_id(id).ok_or_else(|| unknown_scene(id))
}

// -------------------------------------------------------------- wire shapes

/// Which of the five capturable properties a command addresses — the
/// wire form of `kernel::SceneProps`, field-for-field, so there is no
/// second vocabulary for "camera/hidden nodes/hidden tags/section/
/// display" to keep in sync with the kernel's own names.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct PropsParams {
    #[serde(default = "true_")]
    camera: bool,
    #[serde(default = "true_")]
    hidden_nodes: bool,
    #[serde(default = "true_")]
    hidden_tags: bool,
    #[serde(default = "true_")]
    section: bool,
    #[serde(default = "true_")]
    display: bool,
}

fn true_() -> bool {
    true
}

impl Default for PropsParams {
    /// `hew.scenes.add`'s default when `properties` is omitted entirely —
    /// every property, `kernel::SceneProps::ALL`'s wire mirror.
    fn default() -> PropsParams {
        PropsParams {
            camera: true,
            hidden_nodes: true,
            hidden_tags: true,
            section: true,
            display: true,
        }
    }
}

impl PropsParams {
    fn into_scene_props(self) -> kernel::SceneProps {
        kernel::SceneProps {
            camera: self.camera,
            hidden_nodes: self.hidden_nodes,
            hidden_tags: self.hidden_tags,
            section: self.section,
            display: self.display,
        }
    }
}

fn props_to_json(props: kernel::SceneProps) -> Value {
    json!({
        "camera": props.camera,
        "hidden_nodes": props.hidden_nodes,
        "hidden_tags": props.hidden_tags,
        "section": props.section,
        "display": props.display,
    })
}

/// The wire form of `kernel::DisplayState` — opaque booleans the kernel
/// stores and returns, never interprets (docs/design/scenes.md §2
/// "Display"). All three are required together: a Scene's display
/// capture is all-or-nothing, there is no partial display state to merge
/// with what a Scene already had.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct DisplayParams {
    grid: bool,
    axes: bool,
    guides: bool,
}

impl DisplayParams {
    fn into_kernel(self) -> kernel::DisplayState {
        kernel::DisplayState {
            grid: self.grid,
            axes: self.axes,
            guides: self.guides,
        }
    }
}

fn display_to_json(d: kernel::DisplayState) -> Value {
    json!({ "grid": d.grid, "axes": d.axes, "guides": d.guides })
}

fn camera_to_json(cam: &kernel::CameraState) -> Value {
    json!({
        "eye": [cam.eye.x, cam.eye.y, cam.eye.z],
        "target": [cam.target.x, cam.target.y, cam.target.z],
        "up": [cam.up.x, cam.up.y, cam.up.z],
        "projection": match cam.projection {
            kernel::CameraProjection::Perspective => "perspective",
            kernel::CameraProjection::Parallel => "parallel",
        },
        "fov_deg": cam.fov_deg,
    })
}

fn section_to_json(section: Option<kernel::SectionPlaneState>) -> Value {
    match section {
        Some(s) => json!({
            "origin": [s.origin.x, s.origin.y, s.origin.z],
            "normal": [s.normal.x, s.normal.y, s.normal.z],
            "active": s.active,
        }),
        None => Value::Null,
    }
}

fn scene_to_json(scene: &kernel::Scene) -> Value {
    let mut out = json!({
        "id": ids::scene_id(scene.sid),
        "sid": scene.sid,
        "name": scene.name,
        "description": scene.description,
        "props": props_to_json(scene.props()),
    });
    if let Some(cam) = &scene.camera {
        out["camera"] = camera_to_json(cam);
    }
    if let Some(section) = scene.section {
        out["section"] = section_to_json(section);
    }
    if let Some(d) = scene.display {
        out["display"] = display_to_json(d);
    }
    out
}

/// Resolves an explicit `camera` param (§7's shared RawCamera vocabulary
/// — no `view` shorthand here, since a Scene captures a concrete
/// eye/target, not a fitted named view) to a kernel `CameraState`, or —
/// when `props.camera` is set and no explicit camera was given — falls
/// back to the document's own saved working camera
/// (`Document::camera_state`), the same "capture what's already there"
/// default `hew.view.snapshot`'s cameraless path uses. `None` when
/// neither applies: nothing to capture, and `capture_into` (kernel side)
/// only reads this when `props.camera` is set anyway.
fn resolve_capture_camera(
    ctx: &Ctx,
    raw: Option<RawCamera>,
    props_camera: bool,
) -> Result<Option<kernel::CameraState>, CmdError> {
    let (camera, _view) = parse_camera_or_view(raw, None)?;
    Ok(match camera {
        Some(c) => Some(to_kernel_camera(&c)),
        None if props_camera => ctx.doc.camera_state(),
        None => None,
    })
}

// ------------------------------------------------------------- hew.scenes.list

fn list(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {}
    let P {} = parse(params)?;
    let scenes: Vec<Value> = ctx.doc.scenes().iter().map(scene_to_json).collect();
    Ok(json!({ "scenes": scenes }))
}

// -------------------------------------------------------------- hew.scenes.add

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AddParams {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    camera: Option<RawCamera>,
    #[serde(default)]
    display: Option<DisplayParams>,
    #[serde(default)]
    properties: PropsParams,
    #[serde(default)]
    after: Option<String>,
}

fn add(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: AddParams = parse(params)?;
    let props = p.properties.into_scene_props();
    let camera_state = resolve_capture_camera(ctx, p.camera, props.camera)?;
    let display_state = p.display.map(DisplayParams::into_kernel);
    let after = p.after.as_deref().map(resolve_scene_id).transpose()?;

    let sid = ctx
        .doc
        .add_scene(p.name, props, camera_state, display_state, after)?;
    // `set_scene_description` is a separate kernel call from `add_scene`
    // (which has no description parameter of its own) — cheap and never
    // refuses beyond `UnknownScene`, unreachable immediately after a
    // successful `add_scene` with the sid it just minted.
    if let Some(description) = p.description {
        ctx.doc.set_scene_description(sid, description)?;
    }
    let name = ctx.doc.scene(sid)?.name.clone();
    Ok(json!({ "id": ids::scene_id(sid), "sid": sid, "name": name }))
}

// ----------------------------------------------------------- hew.scenes.update

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateParams {
    id: String,
    #[serde(default)]
    properties: Option<PropsParams>,
    #[serde(default)]
    camera: Option<RawCamera>,
    #[serde(default)]
    display: Option<DisplayParams>,
}

fn update(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: UpdateParams = parse(params)?;
    let sid = resolve_scene_id(&p.id)?;
    // No `properties` given: re-capture exactly what this Scene already
    // captures (docs/agents/HEW_API.md's Scenes section) rather than defaulting
    // to ALL — `update` re-snapshots the current set, it does not widen
    // it the way `add`'s from-scratch default does.
    let props = match p.properties {
        Some(pp) => pp.into_scene_props(),
        None => ctx.doc.scene(sid)?.props(),
    };
    let camera_state = resolve_capture_camera(ctx, p.camera, props.camera)?;
    let display_state = p.display.map(DisplayParams::into_kernel);
    ctx.doc
        .update_scene(sid, props, camera_state, display_state)?;
    Ok(json!({}))
}

// ----------------------------------------------------------- hew.scenes.rename

fn rename(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        id: String,
        name: String,
    }
    let p: P = parse(params)?;
    let sid = resolve_scene_id(&p.id)?;
    ctx.doc.rename_scene(sid, p.name)?;
    Ok(json!({}))
}

// --------------------------------------------------------- hew.scenes.describe

fn describe(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        id: String,
        description: String,
    }
    let p: P = parse(params)?;
    let sid = resolve_scene_id(&p.id)?;
    ctx.doc.set_scene_description(sid, p.description)?;
    Ok(json!({}))
}

// ----------------------------------------------------------- hew.scenes.remove

fn remove(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        id: String,
    }
    let p: P = parse(params)?;
    let sid = resolve_scene_id(&p.id)?;
    ctx.doc.remove_scene(sid)?;
    Ok(json!({}))
}

// ---------------------------------------------------------- hew.scenes.reorder

fn reorder(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        id: String,
        index: usize,
    }
    let p: P = parse(params)?;
    let sid = resolve_scene_id(&p.id)?;
    ctx.doc.move_scene(sid, p.index)?;
    Ok(json!({}))
}

// ------------------------------------------------------------ hew.scenes.apply

fn apply(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        id: String,
    }
    let p: P = parse(params)?;
    let sid = resolve_scene_id(&p.id)?;
    let resolved = ctx.doc.apply_scene(sid)?;
    // Best-effort host notification — see the module doc comment. Never a
    // second veto point: the kernel state above is already written.
    ctx.host.scene_applied(sid).map_err(CmdError::Refusal)?;

    let resolver = ctx.resolver();
    let mut out = json!({});
    if let Some(cam) = resolved.camera {
        out["camera"] = camera_to_json(&cam);
    }
    if let Some(section) = resolved.section {
        out["section"] = section_to_json(section);
    }
    // `None` means "hidden state wasn't captured — nothing to push", NOT
    // "push an empty set": a Scene that captures only, say, the camera
    // must never read as "show everything," silently un-hiding whatever
    // the user currently has hidden. So these keys are OMITTED rather
    // than empty-arrayed when uncaptured — the caller's contract is the
    // same as `ResolvedScene`'s own `Option` (kernel/src/scenes.rs).
    if let Some(objects) = &resolved.hidden_object_ids {
        let ids: Vec<String> = objects
            .iter()
            .filter_map(|&o| resolver.public_of(ctx.doc, &EntityRef::Object(o)))
            .collect();
        out["hidden_object_ids"] = json!(ids);
    }
    if let Some(instances) = &resolved.hidden_instance_ids {
        let ids: Vec<String> = instances
            .iter()
            .filter_map(|&i| resolver.public_of(ctx.doc, &EntityRef::Instance(i)))
            .collect();
        out["hidden_instance_ids"] = json!(ids);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::FaceTokens;
    use crate::host::NoHost;

    fn ctx<'a>(
        doc: &'a mut kernel::Document,
        host: &'a mut NoHost,
        tokens: &'a mut FaceTokens,
    ) -> Ctx<'a> {
        Ctx {
            doc,
            host,
            face_tokens: tokens,
            current_label: None,
        }
    }

    fn call(ctx: &mut Ctx, name: &str, params: Value) -> Result<Value, CmdError> {
        handler(name).expect("declared above")(ctx, &params)
    }

    #[test]
    fn add_then_list_round_trips_defaults() {
        let mut doc = kernel::Document::new();
        let mut host = NoHost;
        let mut tokens = FaceTokens::new();
        let mut c = ctx(&mut doc, &mut host, &mut tokens);

        let added = call(&mut c, "hew.scenes.add", json!({})).expect("add succeeds");
        assert_eq!(added["name"], "Scene 1");
        let id = added["id"].as_str().expect("id string").to_string();
        assert!(id.starts_with("scene_"));

        let listed = call(&mut c, "hew.scenes.list", json!({})).expect("list succeeds");
        let scenes = listed["scenes"].as_array().expect("array");
        assert_eq!(scenes.len(), 1);
        assert_eq!(scenes[0]["id"], id);
        assert_eq!(scenes[0]["name"], "Scene 1");
        assert_eq!(scenes[0]["description"], "");
        // Default properties request ALL five, but `props` (both here and
        // kernel-side, `Scene::props`) reports whether there is actually
        // something captured, not merely what was asked for
        // (`kernel::scenes::Scene::props`'s own doc comment: "Some
        // fields"). A brand new document has no working camera and no
        // display was passed, so `camera`/`display` end up NOT captured
        // even though `properties` defaulted to requesting them — nothing
        // existed to capture. hidden_nodes/hidden_tags/section always
        // capture something (an empty set, or captured-no-plane), so
        // those three stay true.
        assert_eq!(scenes[0]["props"]["camera"], false);
        assert_eq!(scenes[0]["props"]["hidden_nodes"], true);
        assert_eq!(scenes[0]["props"]["hidden_tags"], true);
        assert_eq!(scenes[0]["props"]["section"], true);
        assert_eq!(scenes[0]["props"]["display"], false);
        assert!(scenes[0].get("camera").is_none());
        assert!(scenes[0].get("display").is_none());
        // Section defaults to captured-no-plane: `Some(None)` kernel side,
        // JSON `null` on the wire.
        assert_eq!(scenes[0]["section"], Value::Null);
    }

    #[test]
    fn add_captures_an_explicit_camera_and_display() {
        let mut doc = kernel::Document::new();
        let mut host = NoHost;
        let mut tokens = FaceTokens::new();
        let mut c = ctx(&mut doc, &mut host, &mut tokens);

        let added = call(
            &mut c,
            "hew.scenes.add",
            json!({
                "name": "Front",
                "camera": {"eye": [0.0, -5.0, 2.0], "target": [0.0, 0.0, 0.0]},
                "display": {"grid": false, "axes": true, "guides": false},
            }),
        )
        .expect("add succeeds");
        let id = added["id"].as_str().unwrap().to_string();

        let listed = call(&mut c, "hew.scenes.list", json!({})).unwrap();
        let scene = &listed["scenes"][0];
        assert_eq!(scene["id"], id);
        assert_eq!(scene["camera"]["eye"], json!([0.0, -5.0, 2.0]));
        assert_eq!(scene["camera"]["projection"], "perspective");
        assert_eq!(scene["camera"]["fov_deg"], 35.0);
        assert_eq!(
            scene["display"],
            json!({"grid": false, "axes": true, "guides": false})
        );
    }

    #[test]
    fn add_refuses_a_duplicate_name() {
        let mut doc = kernel::Document::new();
        let mut host = NoHost;
        let mut tokens = FaceTokens::new();
        let mut c = ctx(&mut doc, &mut host, &mut tokens);

        call(&mut c, "hew.scenes.add", json!({"name": "Alpha"})).expect("first add succeeds");
        let err = call(&mut c, "hew.scenes.add", json!({"name": "Alpha"})).unwrap_err();
        match err {
            CmdError::Refusal(r) => assert_eq!(r.name, "duplicate_scene_name"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn add_refuses_an_empty_name() {
        let mut doc = kernel::Document::new();
        let mut host = NoHost;
        let mut tokens = FaceTokens::new();
        let mut c = ctx(&mut doc, &mut host, &mut tokens);

        let err = call(&mut c, "hew.scenes.add", json!({"name": "   "})).unwrap_err();
        match err {
            CmdError::Refusal(r) => assert_eq!(r.name, "empty_scene_name"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn rename_describe_reorder_and_remove_round_trip() {
        let mut doc = kernel::Document::new();
        let mut host = NoHost;
        let mut tokens = FaceTokens::new();
        let mut c = ctx(&mut doc, &mut host, &mut tokens);

        let a = call(&mut c, "hew.scenes.add", json!({"name": "A"})).unwrap();
        let a_id = a["id"].as_str().unwrap().to_string();
        call(&mut c, "hew.scenes.add", json!({"name": "B"})).unwrap();

        call(
            &mut c,
            "hew.scenes.rename",
            json!({"id": a_id, "name": "Alpha"}),
        )
        .expect("rename succeeds");
        call(
            &mut c,
            "hew.scenes.describe",
            json!({"id": a_id, "description": "the first one"}),
        )
        .expect("describe succeeds");

        let listed = call(&mut c, "hew.scenes.list", json!({})).unwrap();
        let scenes = listed["scenes"].as_array().unwrap();
        assert_eq!(scenes[0]["name"], "Alpha");
        assert_eq!(scenes[0]["description"], "the first one");
        assert_eq!(scenes[1]["name"], "B");

        // Move "Alpha" (index 0) after "B" (index 1).
        call(
            &mut c,
            "hew.scenes.reorder",
            json!({"id": a_id, "index": 1}),
        )
        .expect("reorder succeeds");
        let listed = call(&mut c, "hew.scenes.list", json!({})).unwrap();
        let scenes = listed["scenes"].as_array().unwrap();
        assert_eq!(scenes[0]["name"], "B");
        assert_eq!(scenes[1]["name"], "Alpha");

        call(&mut c, "hew.scenes.remove", json!({"id": a_id})).expect("remove succeeds");
        let listed = call(&mut c, "hew.scenes.list", json!({})).unwrap();
        assert_eq!(listed["scenes"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_malformed_or_unknown_id_refuses_unknown_scene_everywhere() {
        let mut doc = kernel::Document::new();
        let mut host = NoHost;
        let mut tokens = FaceTokens::new();
        let mut c = ctx(&mut doc, &mut host, &mut tokens);

        for (method, params) in [
            (
                "hew.scenes.update",
                json!({"id": "scene_ff", "properties": {}}),
            ),
            ("hew.scenes.rename", json!({"id": "scene_ff", "name": "x"})),
            (
                "hew.scenes.describe",
                json!({"id": "scene_ff", "description": "x"}),
            ),
            ("hew.scenes.remove", json!({"id": "scene_ff"})),
            ("hew.scenes.reorder", json!({"id": "scene_ff", "index": 0})),
            ("hew.scenes.apply", json!({"id": "scene_ff"})),
            // Not even hex-shaped: caught by the id parser itself.
            ("hew.scenes.remove", json!({"id": "not-a-scene-id"})),
        ] {
            let err = call(&mut c, method, params).unwrap_err();
            match err {
                CmdError::Refusal(r) => assert_eq!(r.name, "unknown_scene", "method {method}"),
                other => panic!("{method}: expected a refusal, got {other:?}"),
            }
        }
    }

    #[test]
    fn update_with_no_properties_recaptures_the_currently_captured_set() {
        let mut doc = kernel::Document::new();
        let mut host = NoHost;
        let mut tokens = FaceTokens::new();
        let mut c = ctx(&mut doc, &mut host, &mut tokens);

        let added = call(
            &mut c,
            "hew.scenes.add",
            json!({
                "properties": {
                    "camera": true, "hidden_nodes": false, "hidden_tags": false,
                    "section": false, "display": false
                },
                "camera": {"eye": [1.0, 0.0, 0.0], "target": [0.0, 0.0, 0.0]},
            }),
        )
        .unwrap();
        let id = added["id"].as_str().unwrap().to_string();

        // Update with a NEW camera and no `properties` — must re-capture
        // only `camera` (what was already captured), not widen to ALL.
        call(
            &mut c,
            "hew.scenes.update",
            json!({
                "id": id,
                "camera": {"eye": [2.0, 0.0, 0.0], "target": [0.0, 0.0, 0.0]},
            }),
        )
        .expect("update succeeds");

        let listed = call(&mut c, "hew.scenes.list", json!({})).unwrap();
        let scene = &listed["scenes"][0];
        assert_eq!(scene["camera"]["eye"], json!([2.0, 0.0, 0.0]));
        assert_eq!(scene["props"]["camera"], true);
        assert_eq!(scene["props"]["hidden_nodes"], false);
        assert_eq!(scene["props"]["section"], false);
        assert!(scene.get("section").is_none());
    }

    #[test]
    fn apply_writes_kernel_state_and_omits_uncaptured_hidden_sets() {
        let mut doc = kernel::Document::new();
        let mut host = NoHost;
        let mut tokens = FaceTokens::new();
        let mut c = ctx(&mut doc, &mut host, &mut tokens);

        // A camera-only Scene: applying it must not report
        // hidden_object_ids/hidden_instance_ids at all.
        let added = call(
            &mut c,
            "hew.scenes.add",
            json!({
                "properties": {
                    "camera": true, "hidden_nodes": false, "hidden_tags": false,
                    "section": false, "display": false
                },
                "camera": {"eye": [3.0, 0.0, 0.0], "target": [0.0, 0.0, 0.0]},
            }),
        )
        .unwrap();
        let id = added["id"].as_str().unwrap().to_string();

        let applied = call(&mut c, "hew.scenes.apply", json!({"id": id})).expect("apply succeeds");
        assert_eq!(applied["camera"]["eye"], json!([3.0, 0.0, 0.0]));
        assert!(applied.get("hidden_object_ids").is_none());
        assert!(applied.get("hidden_instance_ids").is_none());
        assert!(applied.get("section").is_none());
    }

    #[test]
    fn apply_reports_empty_hidden_sets_when_hidden_nodes_is_captured_but_empty() {
        let mut doc = kernel::Document::new();
        let mut host = NoHost;
        let mut tokens = FaceTokens::new();
        let mut c = ctx(&mut doc, &mut host, &mut tokens);

        let added = call(
            &mut c,
            "hew.scenes.add",
            json!({
                "properties": {
                    "camera": false, "hidden_nodes": true, "hidden_tags": true,
                    "section": false, "display": false
                },
            }),
        )
        .unwrap();
        let id = added["id"].as_str().unwrap().to_string();

        let applied = call(&mut c, "hew.scenes.apply", json!({"id": id})).expect("apply succeeds");
        assert_eq!(applied["hidden_object_ids"], json!([]));
        assert_eq!(applied["hidden_instance_ids"], json!([]));
    }
}
