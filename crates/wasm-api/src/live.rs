//! The live desktop transport's WASM half (docs/agents/HEW_API.md §11.2): a
//! per-connection [`api::Connection`] dispatched against the SAME live
//! [`kernel::Document`] the viewport renders. The Tauri shell
//! (`shells/tauri`) owns the socket, the discovery file, and the token
//! check; it never sees the document. This module owns dispatch and the
//! [`api::Host`] effects that are cleanly reachable from inside the
//! webview's WASM sandbox — nothing more.
//!
//! [`Scene::api_dispatch`](crate::Scene::api_dispatch) is the entry
//! point; it lives in `lib.rs` because it needs `&mut Scene` (to
//! reconcile derived caches after a mutation), not just `&mut
//! kernel::Document`.

use api::{Host, Refusal, SnapshotProjection, StandardView, ViewCameraSpec};
use serde::Serialize;

/// A viewport/app-settings effect requested by `hew.view.camera`,
/// `hew.view.zoom_extents`, or `hew.view.units` that `LiveHost` cannot
/// perform itself — this WASM module has no DOM access to move the
/// viewport's three.js camera or call `app/src/settings/units.ts`'s
/// setter directly. `LiveHost`'s matching trait method (below) just
/// records the intent here and answers success; `Scene` (`lib.rs`)
/// captures it off `LiveHost` right after dispatch and hands it to the
/// webview's live bridge (`app/src/api/liveBridge.ts`) through
/// [`crate::Scene::take_pending_view_directive`] — a second, JS-side step
/// exactly like how `Scene::api_method_mutates` already tells the bridge
/// "should you resync" outside the JSON-RPC reply itself.
///
/// Deliberately NOT folded into the JSON-RPC reply `api_dispatch` returns:
/// that string is the actual wire response sent back over the socket to
/// whatever dialed in (`hew-cli --live`, an MCP agent) per docs/agents/HEW_API.md
/// §4 — those clients have no viewport or Settings window of their own to
/// act on a directive with, and JSON-RPC responses are contractually
/// `{jsonrpc, id, result|error}`, not a place to smuggle side-channel
/// instructions. This type instead travels a WASM-internal side channel
/// local to the one process that owns both the dispatcher and the UI the
/// directive is for — the desktop app itself — and is read out
/// synchronously in the same JS task that called `api_dispatch`, so there
/// is never a JS round trip for `LiveHost`'s trait methods to block on.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ViewDirective {
    /// `hew.view.camera`'s effect: the live bridge applies it through the
    /// SAME `Viewport.setCamera`/`Viewport.setStandardView` the app's own
    /// Camera menu and toolbar already call (`app/src/viewport/
    /// Viewport.tsx`) — there is no second camera-control path.
    Camera {
        #[serde(skip_serializing_if = "Option::is_none")]
        camera: Option<CameraJson>,
        #[serde(skip_serializing_if = "Option::is_none")]
        view: Option<String>,
    },
    /// `hew.view.zoom_extents`'s effect: the bridge calls
    /// `Viewport.zoomExtents()`.
    ZoomExtents,
    /// `hew.view.units`'s effect: the bridge calls
    /// `app/src/settings/units.ts`'s `setLengthUnit(format)` directly —
    /// NOT a viewport call, since display units are an app-level setting,
    /// not a camera property. Going through that module's own setter
    /// (rather than writing `localStorage` directly) is what keeps the
    /// cross-window `'settings-changed'` broadcast firing, so a live
    /// desktop's separate Settings window stays in sync.
    Units { format: String },
    /// `hew.scenes.apply`'s host notification (`Host::scene_applied`):
    /// the kernel-side state (tag/node hidden flags, section plane) is
    /// already written by the time this fires — `sid` alone is enough
    /// for the bridge to drive whatever app-side activation logic
    /// `useScenesController`'s `scenes.activate` path already has
    /// (camera tween, panel/outliner sync), rather than duplicating that
    /// logic here from raw numbers. See `applyPendingViewDirective`
    /// (`app/src/api/liveBridge.ts`) for the JS side.
    ActivateScene { sid: u64 },
}

/// The wire shape `ViewDirective::Camera` hands the live bridge for an
/// explicit camera — mirrors `hew.view.snapshot`/`hew.view.camera`'s own
/// `camera` params shape (docs/agents/HEW_API.md §7) so the bridge's JSON
/// parsing on the JS side needs exactly one shape to know, matching what
/// it already receives on the wire from an actual `--live` client.
#[derive(Debug, Clone, Serialize)]
pub struct CameraJson {
    pub eye: [f64; 3],
    pub target: [f64; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub up: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fov_deg: Option<f64>,
}

fn standard_view_name(view: StandardView) -> String {
    match view {
        StandardView::Iso => "iso",
        StandardView::Front => "front",
        StandardView::Back => "back",
        StandardView::Left => "left",
        StandardView::Right => "right",
        StandardView::Top => "top",
        StandardView::Bottom => "bottom",
    }
    .to_string()
}

impl From<&ViewCameraSpec> for ViewDirective {
    fn from(spec: &ViewCameraSpec) -> ViewDirective {
        match spec {
            ViewCameraSpec::Explicit(cam) => ViewDirective::Camera {
                camera: Some(CameraJson {
                    eye: cam.eye,
                    target: cam.target,
                    up: cam.up,
                    projection: cam.projection.map(|p| match p {
                        SnapshotProjection::Perspective => "perspective".to_string(),
                        SnapshotProjection::Parallel => "parallel".to_string(),
                    }),
                    fov_deg: cam.fov_deg,
                }),
                view: None,
            },
            ViewCameraSpec::Standard(view) => ViewDirective::Camera {
                camera: None,
                view: Some(standard_view_name(*view)),
            },
        }
    }
}

/// The `api::Host` for a live desktop connection. The webview's WASM
/// runtime has no filesystem, no native renderer, and no viewport handle
/// to give back to `crates/api`, so most host-implemented commands
/// (docs/agents/HEW_API.md §3: `hew.doc.new/open/import`, `hew.view.snapshot`)
/// are honestly refused here with `host_capability_missing` rather than
/// faked (the registry declares that exact refusal name for every one of
/// these — see `crates/api/src/registry.rs`'s Wave D block).
/// `hew.doc.save` and `hew.doc.export` are the two exceptions: neither
/// needs a filesystem to produce its bytes, only the final write does, so
/// both hand the bytes back for the caller to write and refuse only a
/// `path` specifically (see [`save_document`](Self::save_document) and
/// [`export_document`](Self::export_document) below). The much larger
/// kernel-served command surface — sketch, solid, structure, entity,
/// style, attrs, history: everything that is NOT one of the above —
/// never touches this trait at all and runs for real against the live
/// document (`registry.rs`'s `Served::Kernel` commands execute directly
/// inside `crates/api`), which is what makes `--live` actually useful
/// today even before counting `save`/`export`.
///
/// `hew.doc.new`/`hew.doc.open` are a deliberate design call, not a
/// missing feature: docs/agents/HEW_API.md §10 names exactly this posture — "a
/// live host that keeps document lifecycle user-driven simply withholds
/// `hew.doc.new`/`open` from its `app` grant" — because a remote
/// connection silently replacing the user's live, unsaved document is
/// precisely the "astonishing" case §12 warns `--live` must never be.
/// `Profile::App` grants unconditionally today (`registry.rs`'s
/// `Profile::grants` — not this crate's to narrow, and doing so is a
/// `crates/api` change outside this effort's ownership), so the
/// equivalent is enforced here instead: both refuse typed, pointing at
/// the app's own File menu, rather than either faking the swap or
/// reaching past this crate's boundary to narrow the profile.
///
/// `set_camera`/`zoom_extents`/`set_display_units`, unlike every method
/// above, are cleanly reachable from inside the WASM sandbox — not
/// through this trait's own effect (which still has no DOM access), but
/// by recording a [`ViewDirective`] into `directive` and answering
/// success; see that type's doc comment for the full JS hand-off story.
#[derive(Debug, Default)]
pub struct LiveHost {
    /// Set by the last `set_camera`/`zoom_extents`/`set_display_units`
    /// call on this `LiveHost`. `Scene::api_dispatch` (`lib.rs`)
    /// constructs a fresh `LiveHost` per dispatch, so this never leaks a
    /// stale directive from an earlier command into a later one.
    pub directive: Option<ViewDirective>,
}

/// Builds the `host_capability_missing` refusal every method below
/// answers with — same refusal name the registry declares for these
/// commands on any host lacking the capability (`crates/api/src/host.rs`'s
/// own `unsupported()` helper mints the identical name; this is a local
/// copy so the explanation can be specific to what a *live* WASM
/// connection actually lacks, not the generic "connect through a host
/// that can" text, which is misleading here — this IS the desktop app).
fn refuse(what: &str, use_instead: &str) -> Refusal {
    Refusal::api(
        "host_capability_missing",
        &format!("The live desktop connection cannot {what} yet. {use_instead}"),
    )
    .with_detail(serde_json::json!({ "capability": what }))
}

/// Maps a [`mesh_export::ExportError`] to an [`api::Refusal`] through the
/// error's own name/message accessors, so the two can never drift apart —
/// `mesh-export` has no dependency on `crates/api` (DEVELOPMENT.md rule
/// 1), so this boundary-crossing step lives here instead, the same way
/// `crates/hew-cli`'s `CliHost` maps it on its own side.
fn export_refusal(e: mesh_export::ExportError) -> Refusal {
    Refusal::api(e.name(), &e.message())
}

impl Host for LiveHost {
    fn new_document(&mut self, _doc: &mut kernel::Document) -> Result<(), Refusal> {
        Err(refuse(
            "create a new document",
            "Document lifecycle stays user-driven on a live connection — use the app's File > New.",
        ))
    }

    fn open_document(&mut self, _doc: &mut kernel::Document, _path: &str) -> Result<(), Refusal> {
        Err(refuse(
            "open a document",
            "Document lifecycle stays user-driven on a live connection — use the app's File > Open.",
        ))
    }

    /// Serializing the document needs no filesystem — only WRITING the
    /// result does. So a live save produces the bytes and hands them
    /// back; whoever asked (a CLI, an MCP client) writes them on the
    /// side that actually has a disk. A `path` is the one part this host
    /// genuinely cannot honor, and it refuses that specifically rather
    /// than refusing the whole command.
    fn save_document(
        &mut self,
        doc: &kernel::Document,
        path: Option<&str>,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        if path.is_some() {
            return Err(refuse(
                "write a file",
                "The WASM boundary has no filesystem of its own. Omit `path` to receive the \
                 bytes and write them yourself, or use the app's File > Save.",
            ));
        }
        Ok(Some(doc.save_for_persistence()))
    }

    /// Mirrors [`save_document`](Self::save_document)'s posture exactly:
    /// the STL/3MF/glTF/USDZ writers (`crates/mesh-export`, shared with
    /// `crates/hew-cli`'s `CliHost`) need no filesystem to produce bytes,
    /// only a `path` would — so a live export produces the bytes and
    /// hands them back for the caller to write, and refuses only that one
    /// specific thing this boundary genuinely cannot do.
    fn export_document(
        &mut self,
        doc: &kernel::Document,
        format: &str,
        path: Option<&str>,
        segments_per_turn: u32,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        if path.is_some() {
            return Err(refuse(
                "write a file",
                "The WASM boundary has no filesystem of its own. Omit `path` to receive the \
                 bytes and write them yourself, or use the app's File > Export.",
            ));
        }
        // `true`: this is the `hew.doc.export` dispatch command, the same
        // non-interactive posture `hew-cli`'s `CliHost` has — a leaky
        // object is dropped rather than silently handed back
        // (crates/mesh-export's module doc, "Non-solid inclusion"). The
        // desktop app's own interactive File > Export goes through
        // `Scene::export` instead, which passes `false`.
        let bytes =
            mesh_export::export(doc, format, segments_per_turn, true).map_err(export_refusal)?;
        Ok(Some(bytes))
    }

    fn import_document(
        &mut self,
        _doc: &mut kernel::Document,
        _path: &str,
        _options: &serde_json::Value,
    ) -> Result<serde_json::Value, Refusal> {
        Err(refuse(
            "import",
            "The WASM boundary has no filesystem access of its own — use the app's File > Import.",
        ))
    }

    fn snapshot(
        &mut self,
        _doc: &kernel::Document,
        _params: &api::SnapshotParams,
    ) -> Result<api::SnapshotResult, Refusal> {
        Err(refuse(
            "render a snapshot",
            "The live viewport isn't wired to the API host yet; a headless hew-cli connection renders via its software rasterizer instead.",
        ))
    }

    fn write_snapshot(&mut self, _path: &str, _bytes: &[u8]) -> Result<(), Refusal> {
        Err(refuse(
            "write files",
            "The WASM boundary has no filesystem access of its own.",
        ))
    }

    fn set_camera(&mut self, spec: &ViewCameraSpec) -> Result<(), Refusal> {
        self.directive = Some(ViewDirective::from(spec));
        Ok(())
    }

    fn zoom_extents(&mut self) -> Result<(), Refusal> {
        self.directive = Some(ViewDirective::ZoomExtents);
        Ok(())
    }

    fn set_display_units(&mut self, format: &str) -> Result<(), Refusal> {
        self.directive = Some(ViewDirective::Units {
            format: format.to_string(),
        });
        Ok(())
    }

    /// `hew.scenes.apply`'s host notification: unlike the three effects
    /// above, this one is reachable through the SAME kernel-served
    /// dispatch path (`Document::apply_scene` already wrote the kernel
    /// state before `crates/api`'s handler calls this) — it never itself
    /// refuses, it only records the directive for the bridge to pick up.
    fn scene_applied(&mut self, sid: u64) -> Result<(), Refusal> {
        self.directive = Some(ViewDirective::ActivateScene { sid });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{Connection, DispatchOutcome, Profile, Request, RequestId};

    fn req(id: i64, method: &str, params: serde_json::Value) -> Request {
        Request {
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::Number(id)),
            method: method.to_string(),
            params: Some(params),
        }
    }

    /// A document with exactly one watertight box object, built through
    /// `Document::ingest` — the same fixture `crates/mesh-export`'s and
    /// `crates/hew-cli`'s own tests use.
    fn box_document() -> kernel::Document {
        let positions = vec![
            kernel::Point3::new(0.0, 0.0, 0.0),
            kernel::Point3::new(1.0, 0.0, 0.0),
            kernel::Point3::new(1.0, 1.0, 0.0),
            kernel::Point3::new(0.0, 1.0, 0.0),
            kernel::Point3::new(0.0, 0.0, 1.0),
            kernel::Point3::new(1.0, 0.0, 1.0),
            kernel::Point3::new(1.0, 1.0, 1.0),
            kernel::Point3::new(0.0, 1.0, 1.0),
        ];
        let faces = vec![
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ];
        let face_count = faces.len();
        let mesh = kernel::MeshRecipe {
            name: "Box".to_string(),
            positions,
            faces,
            face_materials: vec![kernel::serialize::NO_MATERIAL; face_count],
            face_uv_frames: vec![None; face_count],
            face_holes: vec![Vec::new(); face_count],
            base_material: kernel::serialize::NO_MATERIAL,
            tags: Vec::new(),
        };
        let mut doc = kernel::Document::new();
        let scene = kernel::ImportScene {
            materials: Vec::new(),
            defs: Vec::new(),
            roots: vec![kernel::ImportNode::Mesh(mesh)],
            guides: Vec::new(),
            tags: Vec::new(),
        };
        doc.ingest(scene, Vec::new()).expect("ingest a plain box");
        doc
    }

    /// A live export needs no filesystem to produce bytes, only the write
    /// does — mirrors `save_document`'s already-covered posture (see
    /// `crates/hew-cli/tests/cli.rs`'s `cli_host_exports_a_box_to_...`
    /// tests for the headless side of the same contract) and confirms it
    /// reaches the exact same shared writer `crates/mesh-export` gives
    /// every host, byte for byte.
    #[test]
    fn export_document_hands_back_bytes_matching_the_shared_writer() {
        let doc = box_document();
        let mut host = LiveHost::default();
        let bytes = host
            .export_document(&doc, "stl", None, 0)
            .expect("a solid box exports live")
            .expect("bytes come back since there is no filesystem to write to");
        let direct =
            mesh_export::export_stl(&doc, 0, true).expect("the same writer, called directly");
        assert_eq!(
            bytes, direct,
            "live export matches mesh-export byte for byte"
        );
    }

    /// A `path` is the one part of export this boundary genuinely cannot
    /// honor — refused specifically, not the whole command.
    #[test]
    fn export_document_refuses_a_path_since_the_wasm_boundary_has_no_filesystem() {
        let doc = box_document();
        let mut host = LiveHost::default();
        let err = host
            .export_document(&doc, "stl", Some("/tmp/out.stl"), 0)
            .unwrap_err();
        assert_eq!(err.name, "host_capability_missing");
    }

    #[test]
    fn export_document_refuses_nothing_to_export_on_an_empty_document() {
        let doc = kernel::Document::new();
        let mut host = LiveHost::default();
        let err = host.export_document(&doc, "stl", None, 0).unwrap_err();
        assert_eq!(err.name, "nothing_to_export");
    }

    /// A live connection's `hew.doc.new` refuses typed instead of wiping
    /// the document out from under the app — the whole point of
    /// `LiveHost` withholding document lifecycle.
    #[test]
    fn live_host_refuses_new_document_typed() {
        let mut conn = Connection::new(Profile::App, "test");
        let mut doc = kernel::Document::new();
        let mut host = LiveHost::default();
        conn.dispatch(
            &mut doc,
            &mut host,
            req(0, "hew.meta.hello", serde_json::json!({"protocol": 1})),
        );
        conn.dispatch(
            &mut doc,
            &mut host,
            req(1, "hew.doc.attach", serde_json::json!({})),
        );
        let DispatchOutcome::Reply(r) = conn.dispatch(
            &mut doc,
            &mut host,
            req(2, "hew.doc.new", serde_json::json!({})),
        ) else {
            panic!("dispatch replies")
        };
        let err = r.error.expect("hew.doc.new refuses on a live host");
        let data = err.data.expect("refusal carries data");
        assert_eq!(data["refusal"], "host_capability_missing");
    }

    /// `hew.view.snapshot` is core-granted (docs/agents/HEW_API.md §10) but
    /// still refuses on `LiveHost` — the live viewport isn't wired to
    /// the API host yet, so this must not silently return a blank image.
    #[test]
    fn live_host_refuses_snapshot_typed() {
        let mut conn = Connection::new(Profile::App, "test");
        let mut doc = kernel::Document::new();
        let mut host = LiveHost::default();
        conn.dispatch(
            &mut doc,
            &mut host,
            req(0, "hew.meta.hello", serde_json::json!({"protocol": 1})),
        );
        conn.dispatch(
            &mut doc,
            &mut host,
            req(1, "hew.doc.attach", serde_json::json!({})),
        );
        let DispatchOutcome::Reply(r) = conn.dispatch(
            &mut doc,
            &mut host,
            req(2, "hew.view.snapshot", serde_json::json!({})),
        ) else {
            panic!("dispatch replies")
        };
        let err = r.error.expect("hew.view.snapshot refuses on a live host");
        assert_eq!(err.data.unwrap()["refusal"], "host_capability_missing");
    }

    /// A kernel-served command (not a `Host` effect at all) actually runs
    /// against the document — this is the whole reason `--live` is
    /// useful despite every `Host` method above refusing.
    #[test]
    fn kernel_served_commands_run_for_real_on_a_live_host() {
        let mut conn = Connection::new(Profile::App, "test");
        let mut doc = kernel::Document::new();
        let mut host = LiveHost::default();
        conn.dispatch(
            &mut doc,
            &mut host,
            req(0, "hew.meta.hello", serde_json::json!({"protocol": 1})),
        );
        conn.dispatch(
            &mut doc,
            &mut host,
            req(1, "hew.doc.attach", serde_json::json!({})),
        );
        let DispatchOutcome::Reply(r) = conn.dispatch(
            &mut doc,
            &mut host,
            req(2, "hew.query.scene", serde_json::json!({})),
        ) else {
            panic!("dispatch replies")
        };
        assert!(
            r.error.is_none(),
            "a read-only kernel command succeeds live: {:?}",
            r.error
        );
    }

    /// `hew.scenes.apply` — a kernel-served command, unlike `hew.view.*` —
    /// still leaves a `ViewDirective::ActivateScene` for the bridge, via
    /// `LiveHost::scene_applied` (`Host::scene_applied`'s live impl):
    /// `crates/api`'s handler writes the kernel-side state for real AND
    /// notifies the host, both in the same dispatch.
    #[test]
    fn scene_apply_leaves_an_activate_scene_directive() {
        let mut conn = Connection::new(Profile::App, "test");
        let mut doc = kernel::Document::new();
        conn.dispatch(
            &mut doc,
            &mut LiveHost::default(),
            req(0, "hew.meta.hello", serde_json::json!({"protocol": 1})),
        );
        conn.dispatch(
            &mut doc,
            &mut LiveHost::default(),
            req(1, "hew.doc.attach", serde_json::json!({})),
        );

        // Add a Scene through the same live connection first.
        let mut host = LiveHost::default();
        let DispatchOutcome::Reply(add_reply) = conn.dispatch(
            &mut doc,
            &mut host,
            req(2, "hew.scenes.add", serde_json::json!({ "name": "Front" })),
        ) else {
            panic!("dispatch replies")
        };
        assert!(
            add_reply.error.is_none(),
            "add succeeds: {:?}",
            add_reply.error
        );
        // A plain (non-`hew.doc.transact`) mutating dispatch is exactly a
        // one-command transaction (docs/agents/HEW_API.md §6.1): the reply is
        // `{"results": [...], "label": ...}`, the add's own result at
        // `results[0]`, not the bare command result.
        let add_body = add_reply.result.expect("add succeeds");
        let add_result = &add_body["results"][0];
        let sid = add_result["sid"].as_u64().expect("sid is a number");
        let id = add_result["id"]
            .as_str()
            .expect("id is a string")
            .to_string();

        // A directive from `add` (registry-state, ModelMutating, no
        // Host effect) must not leak into the NEXT dispatch's directive.
        assert!(host.directive.is_none());

        let mut host = LiveHost::default();
        let DispatchOutcome::Reply(apply_reply) = conn.dispatch(
            &mut doc,
            &mut host,
            req(3, "hew.scenes.apply", serde_json::json!({ "id": id })),
        ) else {
            panic!("dispatch replies")
        };
        assert!(
            apply_reply.error.is_none(),
            "apply succeeds: {:?}",
            apply_reply.error
        );
        match host.directive {
            Some(ViewDirective::ActivateScene { sid: got }) => assert_eq!(got, sid),
            other => panic!("expected an ActivateScene directive, got {other:?}"),
        }
    }
}
