//! Scenes (docs/design/scenes.md): named, saved views of a document — the
//! camera, the hidden-object set, the hidden-tag set, the section plane, and
//! the editor's display toggles — restored in one step. Also the document's
//! single **section plane** as persisted view state (design §4), which a
//! Scene captures by value.
//!
//! Everything here is **view state, outside undo history** (design §3.1,
//! §5): the same club as [`Document::set_camera_state`],
//! [`Document::set_tag_hidden`], and [`Document::set_node_user_hidden`],
//! whose state a Scene snapshots. Unlike those, Scene mutations DO count as
//! document changes for the app's dirty flag — the caller marks dirty on
//! every `Ok` from an editing method here — because a lost Scene is a lost
//! hour, whereas a lost camera position is nothing.
//!
//! Stable ids: a Scene has its own `sid`, minted from the document's shared
//! counter (so it can never collide with an entity's), but Scenes are NOT
//! entities — they do not appear in [`Document::sids`], carry no attrs, and
//! are addressed by their own list. Hidden nodes and tags are stored as
//! entity sids, so they survive save/load and dense-id renumbering.
//!
//! Dangling references (design §3.1): a Scene keeps the sid of a deleted
//! node or tag in memory (deletion is tombstone-based, so an undo of the
//! deletion re-links it) and the writer prunes dead sids at save time
//! (`serialize.rs`, deterministic). Every read path here skips dead sids;
//! [`SceneDrift::stale_refs`] counts them for the UI.

use std::collections::{BTreeMap, BTreeSet};

use crate::camera::CameraState;
use crate::document::{Document, DocumentError, EntityRef, NodeId};
use crate::ids::{InstanceId, ObjectId};
use crate::math::{Point3, Vec3};
use crate::tol;

/// The document's section plane (design §4): one non-destructive clipping
/// plane, persisted as view state (manifest v16+), captured by value into
/// Scenes. `normal` is unit length and points at the side the cut REMOVES
/// (the app's `sectionManager.ts` convention; the renderer negates it to
/// build its clip plane).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SectionPlaneState {
    pub origin: Point3,
    pub normal: Vec3,
    /// Whether the plane is currently cutting. A placed-but-inactive plane
    /// keeps its position so it can be toggled back on.
    pub active: bool,
}

/// The editor's display toggles a Scene may capture (design §2 "Display").
/// Opaque to the kernel — stored and returned, never interpreted. Shop Mode
/// ignores this property (design §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayState {
    pub grid: bool,
    pub axes: bool,
    pub guides: bool,
}

/// Which of the five capturable properties an operation addresses —
/// SketchUp's "properties to save" checkboxes (design §1, §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SceneProps {
    pub camera: bool,
    pub hidden_nodes: bool,
    pub hidden_tags: bool,
    pub section: bool,
    pub display: bool,
}

impl SceneProps {
    /// Every property — the default for Add Scene.
    pub const ALL: SceneProps = SceneProps {
        camera: true,
        hidden_nodes: true,
        hidden_tags: true,
        section: true,
        display: true,
    };
    /// No property — the identity for `scene_drift`.
    pub const NONE: SceneProps = SceneProps {
        camera: false,
        hidden_nodes: false,
        hidden_tags: false,
        section: false,
        display: false,
    };

    /// True when at least one property is set.
    pub fn any(self) -> bool {
        self.camera || self.hidden_nodes || self.hidden_tags || self.section || self.display
    }
}

/// A saved view (design §3.1). Each captured property is `Some`; `None`
/// means "not captured — do not touch it on apply". `section`'s inner
/// `Option` distinguishes *captured, no plane* (`Some(None)`) from a
/// captured plane.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// Stable id, minted from the document's shared counter.
    pub sid: u64,
    /// Unique per document (kernel-enforced), non-empty.
    pub name: String,
    pub description: String,
    pub camera: Option<CameraState>,
    /// Stable ids of user-hidden objects/groups/instances — a FULL set, not
    /// a delta: everything absent is visible, so geometry created after the
    /// Scene is visible in it (design §3.1).
    pub hidden_nodes: Option<BTreeSet<u64>>,
    /// Stable ids of hidden tag-registry entries (same full-set posture).
    pub hidden_tags: Option<BTreeSet<u64>>,
    pub section: Option<Option<SectionPlaneState>>,
    pub display: Option<DisplayState>,
}

impl Scene {
    /// Which properties this Scene captures (`Some` fields).
    pub fn props(&self) -> SceneProps {
        SceneProps {
            camera: self.camera.is_some(),
            hidden_nodes: self.hidden_nodes.is_some(),
            hidden_tags: self.hidden_tags.is_some(),
            section: self.section.is_some(),
            display: self.display.is_some(),
        }
    }
}

/// What applying a Scene means for the app (design §3.1): the camera and
/// display to apply (`None` = not captured), the renderer-level hidden leaf
/// sets already leaf-expanded through groups (the union of hidden tags and
/// hidden nodes, the same walk the app's own hide toggles perform), the
/// panel-state inputs (hidden tag paths, hidden node handles), and the
/// section plane. Produced by [`Document::resolve_scene`] (pure) and
/// [`Document::apply_scene`] (after writing the kernel-side state).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedScene {
    pub camera: Option<CameraState>,
    pub display: Option<DisplayState>,
    /// `None` when neither hidden property is captured (nothing to push).
    pub hidden_object_ids: Option<Vec<ObjectId>>,
    pub hidden_instance_ids: Option<Vec<InstanceId>>,
    /// The hidden tag paths, for the app's tag-panel state. `None` when
    /// `hidden_tags` is not captured.
    pub hidden_tag_paths: Option<Vec<Vec<String>>>,
    /// The user-hidden node handles, for the app's outliner state. `None`
    /// when `hidden_nodes` is not captured.
    pub hidden_nodes: Option<Vec<NodeId>>,
    pub section: Option<Option<SectionPlaneState>>,
}

/// [`Document::scene_drift`]'s answer: which captured properties no longer
/// match the live document, plus how many captured references point at
/// entities that no longer exist (design §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneDrift {
    pub props: SceneProps,
    pub stale_refs: usize,
}

impl SceneDrift {
    /// True when anything drifted or any reference is stale.
    pub fn any(self) -> bool {
        self.props.any() || self.stale_refs > 0
    }
}

/// True when the two cameras agree within the named tolerances
/// (`tol::SCENE_CAMERA_POSITION` / `SCENE_CAMERA_DIRECTION` /
/// `SCENE_CAMERA_FOV_DEG`). Raw equality would flag "drifted" after every
/// tween landing or orbit-and-return (design §3.1).
pub fn cameras_match(a: &CameraState, b: &CameraState) -> bool {
    a.projection == b.projection
        && a.eye.approx_eq(b.eye, tol::SCENE_CAMERA_POSITION)
        && a.target.approx_eq(b.target, tol::SCENE_CAMERA_POSITION)
        && a.up.approx_eq(b.up, tol::SCENE_CAMERA_DIRECTION)
        && (a.fov_deg - b.fov_deg).abs() <= tol::SCENE_CAMERA_FOV_DEG
}

/// True when the two section states agree within the camera tolerances
/// (`None` matches only `None`).
pub fn sections_match(a: Option<SectionPlaneState>, b: Option<SectionPlaneState>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.active == b.active
                && a.origin.approx_eq(b.origin, tol::SCENE_CAMERA_POSITION)
                && a.normal.approx_eq(b.normal, tol::SCENE_CAMERA_DIRECTION)
        }
        _ => false,
    }
}

impl Document {
    // ------------------------------------------------------- section plane

    /// The document's section plane (design §4), or `None` when none is
    /// placed. View state: persisted (manifest v16+), never undoable.
    pub fn section_plane(&self) -> Option<SectionPlaneState> {
        self.section_plane
    }

    /// Places or moves the section plane. Refuses a non-unit or non-finite
    /// normal / non-finite origin (rule 4: typed refusal, no silent
    /// normalization). View state, NOT undoable, dirties nothing by itself
    /// (the app treats it like the camera).
    pub fn set_section_plane(&mut self, plane: SectionPlaneState) -> Result<(), DocumentError> {
        validate_section_plane(&plane)?;
        self.section_plane = Some(plane);
        Ok(())
    }

    /// Removes the section plane.
    pub fn clear_section_plane(&mut self) {
        self.section_plane = None;
    }

    // -------------------------------------------------------------- scenes

    /// Every Scene, in tab order.
    pub fn scenes(&self) -> &[Scene] {
        &self.scenes
    }

    /// The Scene with stable id `sid`.
    pub fn scene(&self, sid: u64) -> Result<&Scene, DocumentError> {
        self.scenes
            .iter()
            .find(|s| s.sid == sid)
            .ok_or(DocumentError::UnknownScene)
    }

    fn scene_index(&self, sid: u64) -> Result<usize, DocumentError> {
        self.scenes
            .iter()
            .position(|s| s.sid == sid)
            .ok_or(DocumentError::UnknownScene)
    }

    /// The lowest "Scene N" (N ≥ 1) no Scene is using — Add Scene's
    /// auto-name (design §3.1).
    pub fn next_scene_name(&self) -> String {
        let mut n = 1usize;
        loop {
            let candidate = format!("Scene {n}");
            if !self.scenes.iter().any(|s| s.name == candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    fn check_scene_name(&self, name: &str, except: Option<u64>) -> Result<(), DocumentError> {
        if name.trim().is_empty() {
            return Err(DocumentError::EmptySceneName);
        }
        if self
            .scenes
            .iter()
            .any(|s| s.name == name && Some(s.sid) != except)
        {
            return Err(DocumentError::DuplicateSceneName);
        }
        Ok(())
    }

    /// Adds a Scene capturing the document's current view state under
    /// `props` (design §5 "Add"): the kernel snapshots its own hidden-node
    /// set, hidden-tag set, and section plane; `camera` and `display` come
    /// from the app and are stored only when their flag is set. `name =
    /// None` auto-names; a given name must be non-empty and unused. The new
    /// Scene is inserted after `after` (append when `None`). Returns its
    /// stable id. Not undoable; the caller marks the document dirty.
    pub fn add_scene(
        &mut self,
        name: Option<String>,
        props: SceneProps,
        camera: Option<CameraState>,
        display: Option<DisplayState>,
        after: Option<u64>,
    ) -> Result<u64, DocumentError> {
        let name = match name {
            Some(n) => {
                self.check_scene_name(&n, None)?;
                n
            }
            None => self.next_scene_name(),
        };
        let index = match after {
            Some(a) => self.scene_index(a)? + 1,
            None => self.scenes.len(),
        };
        let sid = self.mint_sid();
        let mut scene = Scene {
            sid,
            name,
            description: String::new(),
            camera: None,
            hidden_nodes: None,
            hidden_tags: None,
            section: None,
            display: None,
        };
        self.capture_into(&mut scene, props, camera, display);
        self.scenes.insert(index, scene);
        Ok(sid)
    }

    /// Re-captures the properties in `props` into an existing Scene
    /// (design §5 "Update"): each flagged property is overwritten with the
    /// document's current state (kernel-side) or the supplied `camera` /
    /// `display` (app-side); unflagged properties are left exactly as they
    /// were, captured or not. A flagged `camera`/`display` with `None`
    /// supplied leaves that property untouched (nothing to capture).
    pub fn update_scene(
        &mut self,
        sid: u64,
        props: SceneProps,
        camera: Option<CameraState>,
        display: Option<DisplayState>,
    ) -> Result<(), DocumentError> {
        let index = self.scene_index(sid)?;
        let mut scene = self.scenes[index].clone();
        self.capture_into(&mut scene, props, camera, display);
        self.scenes[index] = scene;
        Ok(())
    }

    /// Sets which properties a Scene captures (the checkboxes). Turning a
    /// property ON captures it now (with `camera`/`display` supplied by the
    /// app; a missing one leaves it uncaptured); turning one OFF drops its
    /// data (design §5 "Uncapture").
    pub fn set_scene_props(
        &mut self,
        sid: u64,
        props: SceneProps,
        camera: Option<CameraState>,
        display: Option<DisplayState>,
    ) -> Result<(), DocumentError> {
        let index = self.scene_index(sid)?;
        let mut scene = self.scenes[index].clone();
        let had = scene.props();
        let newly = SceneProps {
            camera: props.camera && !had.camera,
            hidden_nodes: props.hidden_nodes && !had.hidden_nodes,
            hidden_tags: props.hidden_tags && !had.hidden_tags,
            section: props.section && !had.section,
            display: props.display && !had.display,
        };
        self.capture_into(&mut scene, newly, camera, display);
        if !props.camera {
            scene.camera = None;
        }
        if !props.hidden_nodes {
            scene.hidden_nodes = None;
        }
        if !props.hidden_tags {
            scene.hidden_tags = None;
        }
        if !props.section {
            scene.section = None;
        }
        if !props.display {
            scene.display = None;
        }
        self.scenes[index] = scene;
        Ok(())
    }

    /// Renames a Scene; the new name must be non-empty and unused by any
    /// OTHER Scene (renaming to its own name is a no-op).
    pub fn rename_scene(&mut self, sid: u64, name: String) -> Result<(), DocumentError> {
        let index = self.scene_index(sid)?;
        self.check_scene_name(&name, Some(sid))?;
        self.scenes[index].name = name;
        Ok(())
    }

    /// Sets a Scene's free-text description.
    pub fn set_scene_description(&mut self, sid: u64, text: String) -> Result<(), DocumentError> {
        let index = self.scene_index(sid)?;
        self.scenes[index].description = text;
        Ok(())
    }

    /// Moves a Scene to position `index` in tab order (clamped to the end).
    pub fn move_scene(&mut self, sid: u64, index: usize) -> Result<(), DocumentError> {
        let from = self.scene_index(sid)?;
        let scene = self.scenes.remove(from);
        let to = index.min(self.scenes.len());
        self.scenes.insert(to, scene);
        Ok(())
    }

    /// Deletes a Scene. Not undoable — the UI confirms first (design §5).
    pub fn remove_scene(&mut self, sid: u64) -> Result<(), DocumentError> {
        let index = self.scene_index(sid)?;
        self.scenes.remove(index);
        Ok(())
    }

    /// Resolves what applying a Scene would do, without mutating anything
    /// (design §3.1) — Shop Mode's path, and the headless renderer's.
    pub fn resolve_scene(&self, sid: u64) -> Result<ResolvedScene, DocumentError> {
        let scene = self.scene(sid)?;
        Ok(self.resolve(scene))
    }

    /// Writes a Scene's captured kernel-side state into the document — the
    /// hidden-tag registry flags, the user-hidden node set, the section
    /// plane — and returns the resolution for the app to finish (camera,
    /// display, renderer leaf sets). Validates and resolves everything
    /// FIRST and skips dead sids, so the write phase cannot fail part-way
    /// (design §3.1). Non-undoable, like the state it writes; not a dirtying
    /// change (activation is view state, design §5).
    pub fn apply_scene(&mut self, sid: u64) -> Result<ResolvedScene, DocumentError> {
        let scene = self.scene(sid)?.clone();
        let resolved = self.resolve(&scene);

        if let Some(paths) = &resolved.hidden_tag_paths {
            let want: BTreeSet<&Vec<String>> = paths.iter().collect();
            let known: Vec<Vec<String>> = self.tag_meta().map(|(p, _)| p.to_vec()).collect();
            for path in known {
                let hidden = want.contains(&path);
                if self.tag_hidden(&path) != hidden {
                    self.set_tag_hidden(path, hidden);
                }
            }
            // A captured tag the registry forgot (deleted, then not
            // re-created) is skipped by `resolve`; nothing to register.
        }
        if let Some(nodes) = &resolved.hidden_nodes {
            let want: BTreeSet<NodeId> = nodes.iter().copied().collect();
            for node in self.user_hidden_nodes() {
                if !want.contains(&node) {
                    self.set_node_user_hidden(node, false);
                }
            }
            for &node in &want {
                self.set_node_user_hidden(node, true);
            }
        }
        if let Some(section) = resolved.section {
            self.section_plane = section;
        }
        Ok(resolved)
    }

    /// Which captured properties no longer match the live document, and how
    /// many captured references are dead (design §3.1, §5 "Active + drift").
    /// `live_camera` / `live_display` are the app's current values; when
    /// `None`, that property is reported as not drifted (nothing to compare).
    pub fn scene_drift(
        &self,
        sid: u64,
        live_camera: Option<&CameraState>,
        live_display: Option<&DisplayState>,
    ) -> Result<SceneDrift, DocumentError> {
        let scene = self.scene(sid)?;
        let mut props = SceneProps::NONE;
        let mut stale = 0usize;

        if let (Some(captured), Some(live)) = (&scene.camera, live_camera) {
            props.camera = !cameras_match(captured, live);
        }
        if let (Some(captured), Some(live)) = (&scene.display, live_display) {
            props.display = captured != live;
        }
        if let Some(captured) = &scene.hidden_tags {
            let live_hidden: BTreeSet<u64> = self.hidden_tag_sids();
            let mut live_only = live_hidden.clone();
            let mut captured_live = BTreeSet::new();
            for &s in captured {
                if self.tag_sid_live(s) {
                    captured_live.insert(s);
                } else {
                    stale += 1;
                }
            }
            for s in &captured_live {
                live_only.remove(s);
            }
            props.hidden_tags =
                !live_only.is_empty() || captured_live.iter().any(|s| !live_hidden.contains(s));
        }
        if let Some(captured) = &scene.hidden_nodes {
            let live_hidden: BTreeSet<u64> = self.hidden_node_sids();
            let mut captured_live = BTreeSet::new();
            for &s in captured {
                if self.node_sid_live(s) {
                    captured_live.insert(s);
                } else {
                    stale += 1;
                }
            }
            props.hidden_nodes = captured_live != live_hidden;
        }
        if let Some(captured) = scene.section {
            props.section = !sections_match(captured, self.section_plane);
        }
        Ok(SceneDrift {
            props,
            stale_refs: stale,
        })
    }

    /// The renderer-level hidden leaf sets of the document AS IT STANDS —
    /// live user-hidden nodes expanded through groups, plus every world node
    /// under a hidden tag — deduplicated, in deterministic order. What a
    /// headless render (`softrender::document_items`) skips so it matches
    /// the app's own view; the Scene-specific counterpart is
    /// [`ResolvedScene::hidden_object_ids`].
    pub fn hidden_leaves(&self) -> (Vec<ObjectId>, Vec<InstanceId>) {
        let nodes: Vec<NodeId> = self
            .user_hidden_nodes()
            .into_iter()
            .filter(|&n| self.node_live(n))
            .collect();
        let tags: Vec<Vec<String>> = self
            .tag_meta()
            .filter(|(_, hidden)| *hidden)
            .map(|(p, _)| p.to_vec())
            .collect();
        self.union_hidden_leaves(&nodes, &tags)
    }

    // ------------------------------------------------------------ internals

    /// Snapshot the flagged properties into `scene`.
    fn capture_into(
        &self,
        scene: &mut Scene,
        props: SceneProps,
        camera: Option<CameraState>,
        display: Option<DisplayState>,
    ) {
        if props.camera
            && let Some(c) = camera
        {
            scene.camera = Some(c);
        }
        if props.display
            && let Some(d) = display
        {
            scene.display = Some(d);
        }
        if props.hidden_nodes {
            scene.hidden_nodes = Some(self.hidden_node_sids());
        }
        if props.hidden_tags {
            scene.hidden_tags = Some(self.hidden_tag_sids());
        }
        if props.section {
            scene.section = Some(self.section_plane);
        }
    }

    /// Stable ids of every user-hidden node that is live.
    fn hidden_node_sids(&self) -> BTreeSet<u64> {
        self.user_hidden_nodes()
            .into_iter()
            .filter(|&n| self.node_live(n))
            .filter_map(|n| self.sid_of(&entity_of(n)))
            .collect()
    }

    /// Stable ids of every registered tag whose hidden flag is set.
    fn hidden_tag_sids(&self) -> BTreeSet<u64> {
        self.tag_meta()
            .filter(|(_, hidden)| *hidden)
            .filter_map(|(path, _)| self.sid_of(&EntityRef::Tag(path.to_vec())))
            .collect()
    }

    /// Reverse map: node sid → live node handle.
    fn live_node_by_sid(&self) -> BTreeMap<u64, NodeId> {
        self.sids()
            .filter_map(|(entity, sid)| match entity {
                EntityRef::Object(id) => Some((sid, NodeId::Object(*id))),
                EntityRef::Group(id) => Some((sid, NodeId::Group(*id))),
                EntityRef::Instance(id) => Some((sid, NodeId::Instance(*id))),
                _ => None,
            })
            .filter(|(_, node)| self.node_live(*node))
            .collect()
    }

    /// Reverse map: tag sid → registered path.
    fn live_tag_by_sid(&self) -> BTreeMap<u64, Vec<String>> {
        let registered: BTreeSet<Vec<String>> = self.tag_meta().map(|(p, _)| p.to_vec()).collect();
        self.sids()
            .filter_map(|(entity, sid)| match entity {
                EntityRef::Tag(path) if registered.contains(path) => Some((sid, path.clone())),
                _ => None,
            })
            .collect()
    }

    fn node_sid_live(&self, sid: u64) -> bool {
        self.live_node_by_sid().contains_key(&sid)
    }

    fn tag_sid_live(&self, sid: u64) -> bool {
        self.live_tag_by_sid().contains_key(&sid)
    }

    fn resolve(&self, scene: &Scene) -> ResolvedScene {
        let mut hidden_nodes: Option<Vec<NodeId>> = None;
        let mut hidden_tag_paths: Option<Vec<Vec<String>>> = None;

        if let Some(captured) = &scene.hidden_nodes {
            let by_sid = self.live_node_by_sid();
            hidden_nodes = Some(
                captured
                    .iter()
                    .filter_map(|s| by_sid.get(s).copied())
                    .collect(),
            );
        }
        if let Some(captured) = &scene.hidden_tags {
            let by_sid = self.live_tag_by_sid();
            hidden_tag_paths = Some(
                captured
                    .iter()
                    .filter_map(|s| by_sid.get(s).cloned())
                    .collect(),
            );
        }

        // The renderer leaf sets describe the document AS IT WILL BE after
        // apply: the captured half comes from the Scene, the uncaptured half
        // from the live document — otherwise a tags-only Scene would un-hide
        // the user's manually hidden nodes in the renderer while the kernel
        // still holds them hidden.
        let (hidden_object_ids, hidden_instance_ids) =
            if hidden_nodes.is_some() || hidden_tag_paths.is_some() {
                let live_nodes: Vec<NodeId>;
                let live_tags: Vec<Vec<String>>;
                let nodes: &[NodeId] = match &hidden_nodes {
                    Some(n) => n,
                    None => {
                        live_nodes = self
                            .user_hidden_nodes()
                            .into_iter()
                            .filter(|&n| self.node_live(n))
                            .collect();
                        &live_nodes
                    }
                };
                let tags: &[Vec<String>] = match &hidden_tag_paths {
                    Some(t) => t,
                    None => {
                        live_tags = self
                            .tag_meta()
                            .filter(|(_, hidden)| *hidden)
                            .map(|(p, _)| p.to_vec())
                            .collect();
                        &live_tags
                    }
                };
                let (o, i) = self.union_hidden_leaves(nodes, tags);
                (Some(o), Some(i))
            } else {
                (None, None)
            };

        ResolvedScene {
            camera: scene.camera,
            display: scene.display,
            hidden_object_ids,
            hidden_instance_ids,
            hidden_tag_paths,
            hidden_nodes,
            section: scene.section,
        }
    }

    /// The renderer-level hidden leaf sets implied by a hidden-node list
    /// plus a hidden-tag-path list — the same walk the app's
    /// `unionHiddenLeafIds` performs on every hide toggle (design §5
    /// "Activate"): a hidden group expands to its leaf objects/instances;
    /// a hidden tag covers every WORLD node whose tag path is at or under
    /// it. Deduplicated, in deterministic order.
    fn union_hidden_leaves(
        &self,
        hidden_nodes: &[NodeId],
        hidden_tag_paths: &[Vec<String>],
    ) -> (Vec<ObjectId>, Vec<InstanceId>) {
        let mut objects: BTreeSet<ObjectId> = BTreeSet::new();
        let mut instances: BTreeSet<InstanceId> = BTreeSet::new();

        for &node in hidden_nodes {
            self.collect_scene_leaves(node, &mut objects, &mut instances);
        }

        if !hidden_tag_paths.is_empty() {
            let mut all_nodes: Vec<NodeId> = Vec::new();
            all_nodes.extend(self.visible_object_ids().into_iter().map(NodeId::Object));
            all_nodes.extend(self.group_ids().into_iter().map(NodeId::Group));
            all_nodes.extend(self.instance_ids().into_iter().map(NodeId::Instance));
            for node in all_nodes {
                let covered = self.node_tags(node).iter().any(|path| {
                    hidden_tag_paths.iter().any(|anchor| {
                        path.len() >= anchor.len() && path[..anchor.len()] == anchor[..]
                    })
                });
                if covered {
                    self.collect_scene_leaves(node, &mut objects, &mut instances);
                }
            }
        }

        (
            objects.into_iter().collect(),
            instances.into_iter().collect(),
        )
    }

    fn collect_scene_leaves(
        &self,
        node: NodeId,
        objects: &mut BTreeSet<ObjectId>,
        instances: &mut BTreeSet<InstanceId>,
    ) {
        match node {
            NodeId::Object(id) => {
                objects.insert(id);
            }
            NodeId::Instance(id) => {
                instances.insert(id);
            }
            NodeId::Group(id) => {
                if let Some(members) = self.group_members(id) {
                    for m in members {
                        self.collect_scene_leaves(m, objects, instances);
                    }
                }
            }
        }
    }
}

/// The [`EntityRef`] a tree node's stable id hangs on.
pub(crate) fn entity_of(node: NodeId) -> EntityRef {
    match node {
        NodeId::Object(id) => EntityRef::Object(id),
        NodeId::Group(id) => EntityRef::Group(id),
        NodeId::Instance(id) => EntityRef::Instance(id),
    }
}

/// Refuses a section plane whose normal is not unit length or whose
/// coordinates are not finite (rule 4).
pub(crate) fn validate_section_plane(plane: &SectionPlaneState) -> Result<(), DocumentError> {
    let finite = plane.origin.x.is_finite()
        && plane.origin.y.is_finite()
        && plane.origin.z.is_finite()
        && plane.normal.x.is_finite()
        && plane.normal.y.is_finite()
        && plane.normal.z.is_finite();
    if !finite {
        return Err(DocumentError::InvalidSectionPlane);
    }
    if (plane.normal.length() - 1.0).abs() > tol::SECTION_NORMAL_UNIT {
        return Err(DocumentError::InvalidSectionPlane);
    }
    Ok(())
}
