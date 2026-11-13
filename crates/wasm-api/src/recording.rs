//! Replayable command recording (docs/dev/DEVELOPMENT.md,  +).
//!
//! A recording is the ordered stream of committed `Scene` mutations as **typed,
//! replayable calls**. Replaying them verbatim into a fresh [`Scene`] reproduces
//! the session, and the final [`state_hash`](crate::Scene::state_hash) is
//! asserted against the recorded golden — the regression guarantee (docs/dev/DEVELOPMENT.md).
//!
//! ## Why verbatim replay needs no handle-remapping
//!
//! Recorded calls carry literal slotmap handles (a `sketch`/`object`/`region`
//! id). Naively those wouldn't survive into a fresh document — except the kernel
//! is now **deterministic**: the same op sequence does the same
//! insert/remove sequence, and `slotmap` assigns keys deterministically from
//! that, so a replay that re-issues the identical calls reproduces the identical
//! handles. The recorded ids are therefore valid verbatim — no remap table. This
//! is a direct payoff of the determinism lane, and the replay test empirically
//! confirms it (a divergence would fail to resolve a handle, or break the hash).
//!
//! A recording replays into a **fresh** `Scene` (`golden_hash` is captured
//! relative to the empty document the recording began on). The artifact JSON
//! shape is frozen in `docs/dev/DIAGNOSTICS.md` — the handshake for the
//! Node runner and the M17 bug-report bundle.

use std::cell::{Cell, RefCell};

use serde::{Deserialize, Serialize};

/// Bump on any breaking change to the [`Recording`] JSON shape. v2 =
/// typed replayable calls (v1 was the log-tap hash-chain). See
/// `docs/dev/DIAGNOSTICS.md`.
///
/// [`RecordedCall::ApiDispatch`] did NOT bump this, matching the posture
/// every other additive variant documents: an old recording replays on a
/// new build unchanged, and a new recording using the variant fails to
/// parse on an old build — loudly, never silently divergent. Bumping
/// instead would strand every frozen reproducer, since replay gates on
/// exact version equality.
pub const RECORDING_FORMAT_VERSION: u32 = 2;

fn default_true() -> bool {
    true
}

/// One committed `Scene` mutation, captured with the exact arguments needed to
/// re-issue it. `#[serde(tag = "method")]` gives a self-describing JSON object
/// per call (`{"method":"extrude_region","sketch":…,"region":…,"distance":…}`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum RecordedCall {
    /// `begin_ground_sketch()`.
    BeginGroundSketch,
    /// `begin_sketch_on_plane(px, py, pz, nx, ny, nz)` — mints a sketch on an
    /// arbitrary plane (sketches-on-any-plane design §5: the idle-lock draw
    /// path). Additive variant (the [`RecordedCall::SketchBeginCurveWith`]
    /// posture): recordings that never draw off the ground plane replay on
    /// older builds unchanged; one that does fails to parse there — loudly,
    /// never silently divergent.
    BeginSketchOnPlane {
        px: f64,
        py: f64,
        pz: f64,
        nx: f64,
        ny: f64,
        nz: f64,
    },
    /// `sketch_add_segment(sketch, a, b)`.
    SketchAddSegment {
        sketch: u64,
        a: [f64; 3],
        b: [f64; 3],
    },
    /// `sketch_remove_edge(sketch, edge)` — the eraser tool's commit.
    /// Additive variant (like [`RecordedCall::SketchBeginCurveWith`]): a
    /// recording that never erases replays on older builds unchanged; one
    /// that does fails to parse there — loudly, never silently divergent.
    SketchRemoveEdge { sketch: u64, edge: u64 },
    /// `sketch_begin_gesture(sketch)`.
    SketchBeginGesture { sketch: u64 },
    /// `sketch_begin_curve(sketch)`.
    SketchBeginCurve { sketch: u64 },
    /// `sketch_begin_curve_with(sketch, center, radius)` — a curve bracket
    /// carrying the chain's analytic circle. Additive variant: recordings
    /// that never use it replay on older builds unchanged; one that does
    /// fails to parse there (loudly, never silently divergent).
    SketchBeginCurveWith {
        sketch: u64,
        center: [f64; 3],
        radius: f64,
    },
    /// `sketch_begin_polygon_with(sketch, center, radius)` — a curve bracket
    /// whose circle is a regular polygon's CIRCUMcircle, not a curve the
    /// facets approximate. Additive variant, same posture as
    /// [`RecordedCall::SketchBeginCurveWith`].
    SketchBeginPolygonWith {
        sketch: u64,
        center: [f64; 3],
        radius: f64,
    },
    /// `sketch_refacet_curve(sketch, curve, segments)` — rebuild a drawn
    /// circle's facets at a new density, in place. Additive variant, same
    /// posture as [`RecordedCall::SketchBeginCurveWith`].
    SketchRefacetCurve {
        sketch: u64,
        curve: u64,
        segments: u32,
    },
    /// `sketch_end_curve(sketch)`.
    SketchEndCurve { sketch: u64 },
    /// `sketch_end_gesture(sketch)`.
    SketchEndGesture { sketch: u64 },
    /// `sketch_cancel_gesture()` (recorded only when a gesture was open).
    SketchCancelGesture,
    /// `extrude_region(sketch, region, distance)`.
    ExtrudeRegion {
        sketch: u64,
        region: u64,
        distance: f64,
    },
    /// `follow_me_along_edges(sketch, region, path_sketch, path_edges)` —
    /// sweep a profile region along a chain of sketch edges. Additive
    /// variant (the [`RecordedCall::SketchBeginCurveWith`] posture): old
    /// recordings replay unchanged; one that sweeps fails to parse on older
    /// builds — loudly, never silently divergent.
    FollowMeAlongEdges {
        sketch: u64,
        region: u64,
        path_sketch: u64,
        path_edges: Vec<u64>,
        /// Group-context birth (design §2f), absent for top-level. Same
        /// additive posture as `stop_len`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group: Option<u64>,
        /// Partial-sweep stop (arc length from the seam), absent for a
        /// full sweep. `skip_serializing_if` keeps a full sweep's record
        /// byte-identical to what it was before the field existed, so no
        /// golden moves; an absent field deserializes to `None` (default),
        /// so old recordings replay unchanged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_len: Option<f64>,
    },
    /// `follow_me_around_face(sketch, region, path_object, path_face)` —
    /// sweep a profile region around a solid face's outer boundary loop.
    /// Additive variant, same posture as
    /// [`RecordedCall::FollowMeAlongEdges`].
    FollowMeAroundFace {
        sketch: u64,
        region: u64,
        path_object: u64,
        path_face: u64,
        /// Partial-sweep stop, exactly as on
        /// [`RecordedCall::FollowMeAlongEdges`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_len: Option<f64>,
        /// Group-context birth (design §2f), absent for top-level.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group: Option<u64>,
    },
    /// `follow_me_around_instance_face(...)` — a face loop reached through
    /// a component instance (design §2e). Additive variant.
    FollowMeAroundInstanceFace {
        sketch: u64,
        region: u64,
        instance: u64,
        path_object: u64,
        path_face: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_len: Option<f64>,
    },
    /// `follow_me_merged_around_face(...)` — the merged molding gesture
    /// (design §3b). Additive variant, same posture as
    /// [`RecordedCall::FollowMeAlongEdges`].
    FollowMeMergedAroundFace {
        sketch: u64,
        region: u64,
        path_object: u64,
        path_face: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_len: Option<f64>,
    },
    /// `follow_me_face_along_edges(...)` — a solid face as the profile
    /// (design §3a). Additive variant.
    FollowMeFaceAlongEdges {
        profile_object: u64,
        profile_face: u64,
        path_sketch: u64,
        path_edges: Vec<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_len: Option<f64>,
    },
    /// `follow_me_face_around_face(...)` — a solid face as the profile,
    /// swept around a face loop. Additive variant.
    FollowMeFaceAroundFace {
        profile_object: u64,
        profile_face: u64,
        path_object: u64,
        path_face: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_len: Option<f64>,
    },
    /// `sketch_offset_region(sketch, region, distance)` — the Offset tool's
    /// sketch commit: the region's whole boundary offset by a uniform
    /// distance, inserted as new sketch geometry. Additive variant (the
    /// [`RecordedCall::SketchBeginCurveWith`] posture): recordings that never
    /// offset replay on older builds unchanged; one that does fails to parse
    /// there — loudly, never silently divergent.
    SketchOffsetRegion {
        sketch: u64,
        region: u64,
        distance: f64,
    },
    /// `boolean(op, a, b)`.
    Boolean { op: u8, a: u64, b: u64 },
    /// `boolean_nodes(op, a_kind, a, b_kind, b)` — the node-operand boolean
    /// (plain solids or whole groups; the group-ops design) the UI routes
    /// every boolean command through. Additive variant (the
    /// [`RecordedCall::SketchBeginCurveWith`] posture): old recordings replay
    /// unchanged; a recording that uses it fails to parse on older builds —
    /// loudly, never silently divergent.
    BooleanNodes {
        op: u8,
        a_kind: u8,
        a: u64,
        b_kind: u8,
        b: u64,
    },
    /// `boolean_nodes_many(op, kinds, ids)` — the same combine over any
    /// number of nodes as one undo step. Additive variant: a recording that
    /// uses it fails to parse on older builds, loudly.
    BooleanNodesMany {
        op: u8,
        kinds: Vec<u8>,
        ids: Vec<u64>,
    },
    /// `group_nodes(kinds, ids)` — form a merge group. Additive variant;
    /// closes a pre-existing structural-op recording gap alongside
    /// [`RecordedCall::BooleanNodes`] (a group boolean is unreplayable
    /// without the grouping that built its operand).
    GroupNodes { kinds: Vec<u8>, ids: Vec<u64> },
    /// `duplicate_node(kind, id, affine)` — the Move+Alt deep copy. Additive
    /// variant; closes the same pre-existing gap as
    /// [`RecordedCall::GroupNodes`].
    DuplicateNode {
        kind: u8,
        id: u64,
        affine: [f64; 12],
    },
    /// `slice_object(object, plane)`.
    SliceObject { object: u64, plane: [f64; 6] },
    /// `transform_object(object, affine)`.
    TransformObject { object: u64, affine: [f64; 12] },
    /// `transform_selection(kinds, ids, sketches, affine)` — a whole
    /// multi-selection moved/rotated/scaled as one undo step.
    TransformSelection {
        kinds: Vec<u8>,
        ids: Vec<u64>,
        sketches: Vec<u64>,
        affine: [f64; 12],
    },
    /// `rescale_document(factor)` — the Tape Measure "resize the model"
    /// flow (tool-parity §3): the whole document uniformly scaled about the
    /// world origin in one undo step. Additive variant (the
    /// [`RecordedCall::SketchBeginCurveWith`] posture): old recordings
    /// replay unchanged; one that rescales fails to parse on older builds —
    /// loudly, never silently divergent.
    RescaleDocument { factor: f64 },
    /// `delete_node(kind, id)`.
    DeleteNode { kind: u8, id: u64 },
    /// `duplicate_selection_array(kinds, ids, affine, count)` — the Move
    /// tool's array copy (a copy commit, or its ×N / /N refinement): every
    /// listed node cloned `count` times along `affine`, one undo step.
    /// Additive variant (same posture as [`RecordedCall::DuplicateNode`]).
    DuplicateSelectionArray {
        kinds: Vec<u8>,
        ids: Vec<u64>,
        affine: [f64; 12],
        count: u32,
        /// Linked copies ([`kernel::LinkPolicy::Linked`]). Missing in a
        /// recording made before linked copies, which replays as
        /// independent; written only when true, so an independent copy
        /// records exactly as it always did.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        linked: bool,
    },
    /// `wrap_linked(kind, id)` — one Object or Group wrapped as a linked
    /// set. Additive variant (same posture as the others).
    WrapLinked { kind: u8, id: u64 },
    /// `set_linked(component, linked)` — "Make component" removing a linked
    /// set's `hew` / `linked` (`linked = false`). Additive variant.
    SetLinked { component: u64, linked: bool },
    /// `push_pull(object, face, distance)` — the user-level push/pull of a
    /// solid face. Replay re-issues it and the kernel re-derives the routing
    /// (translate, coplanar-aware, whole-wall radial offset, boss/recess, or
    /// through-cut), so recording the intent alone reproduces the result.
    /// Additive variant (same posture as the others); enables a draw-on-face
    /// imprint to be pushed through in replay (the true-curves design, C3).
    PushPull {
        object: u64,
        face: u64,
        distance: f64,
    },
    /// `extrude_face_as_new_object(object, face, distance)` — Push/Pull's
    /// Ctrl/Cmd modifier (tool-parity §2): straight-extrudes a solid face's
    /// boundary into a NEW top-level object, leaving the source untouched.
    /// Additive variant (the [`RecordedCall::SketchBeginCurveWith`] posture):
    /// old recordings replay unchanged; one that uses the modifier fails to
    /// parse on older builds — loudly, never silently divergent.
    ExtrudeFaceAsNewObject {
        object: u64,
        face: u64,
        distance: f64,
    },
    /// `scene_undo()` — recorded only when it succeeded (a refused undo
    /// commits nothing). Additive variant (the
    /// [`RecordedCall::SketchBeginCurveWith`] posture): old recordings
    /// replay unchanged; a recording that uses it fails to parse on older
    /// builds — loudly, never silently divergent. Undo/redo are where
    /// Model D's subtle behavior lives (extrusion undo re-inserts
    /// scaffolding, merging with later edits), so a bug reproducer must
    /// carry them.
    SceneUndo,
    /// `scene_redo()` — recorded only when it succeeded.
    SceneRedo,

    // -------------------------------------------------------------------
    // Coverage-audit variants: EVERY `Scene` method that pushes the
    // document undo stack or mutates state included in `Document::save`
    // records itself, so a recorded `scene_undo`/`scene_redo` replays
    // against an identically-shaped undo stack and identical persisted
    // state. (The proven divergence: a session's `delete_tag` + undo
    // replayed as an undo of the *previous* op off a shorter stack.)
    // All additive — the [`RecordedCall::SketchBeginCurveWith`] posture:
    // old recordings replay unchanged; a recording that uses one of these
    // fails to parse on older builds loudly, never silently divergent.
    // Session-only state (inference hides, transient segments, snappable
    // toggles, torture mode) is deliberately NOT recorded: it is neither
    // undoable nor saved, so it cannot reshape the stack or the document.
    // -------------------------------------------------------------------
    /// `transform_sketch(sketch, affine)`.
    TransformSketch { sketch: u64, affine: [f64; 12] },
    /// `transform_sketch_island(sketch, island, affine)`.
    TransformSketchIsland {
        sketch: u64,
        island: u64,
        affine: [f64; 12],
    },
    /// `transform_sketch_islands(sketch, islands, affine)` — additive
    /// variant (the [`RecordedCall::SketchBeginCurveWith`] posture): the
    /// multi-island move that arranges like a draw.
    TransformSketchIslands {
        sketch: u64,
        islands: Vec<u64>,
        affine: [f64; 12],
    },
    /// `copy_sketch_islands(sketch, islands, affine)` — additive variant (the
    /// [`RecordedCall::SketchBeginCurveWith`] posture): the returned copy
    /// handle is deterministic (the next minted `SketchId`), so replay
    /// re-derives it without recording it.
    CopySketchIslands {
        sketch: u64,
        islands: Vec<u64>,
        affine: [f64; 12],
    },
    /// `move_sketch_vertex(sketch, vertex, p)`.
    MoveSketchVertex {
        sketch: u64,
        vertex: u64,
        p: [f64; 3],
    },
    /// `ungroup(group)`.
    Ungroup { group: u64 },
    /// `delete_sketch(sketch)`.
    DeleteSketch { sketch: u64 },
    /// `transform_group(group, affine)`.
    TransformGroup { group: u64, affine: [f64; 12] },
    /// `make_component(kinds, ids)`.
    MakeComponent { kinds: Vec<u8>, ids: Vec<u64> },
    /// `place_instance(component, affine)`.
    PlaceInstance { component: u64, affine: [f64; 12] },
    /// `place_text(sketch, regions, distance, name, group)` — the 3D Text
    /// placement's atomic tail (extrude the app-selected fill regions,
    /// fold into one component, place one instance), folding the
    /// immediately-preceding glyph-injection gesture into the same undo
    /// step. Additive variant (the [`RecordedCall::SketchBeginCurveWith`]
    /// posture): recordings that never place 3D text replay on older
    /// builds unchanged; one that does fails to parse there — loudly,
    /// never silently divergent.
    PlaceText {
        sketch: u64,
        regions: Vec<u64>,
        distance: f64,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group: Option<u64>,
    },
    /// `transform_instance(instance, affine)`.
    TransformInstance { instance: u64, affine: [f64; 12] },
    /// `explode_instance(instance)`.
    ExplodeInstance { instance: u64 },
    /// `make_unique(instance)`.
    MakeUnique { instance: u64 },
    /// `make_unique_by_path_json(path, cascade, deep)`. `MakeUnique` stays
    /// for the one-instance entry point and keeps replaying old recordings.
    MakeUniqueByPath {
        path: Vec<u64>,
        /// Absent in a hand-written recording: the default, `true`.
        #[serde(default = "default_true")]
        cascade: bool,
        #[serde(default)]
        deep: bool,
    },
    /// `open_explode_session(instance)` (docs/design/explode-session-
    /// prototype.md, a `proto/explode-session` prototype).
    OpenExplodeSession { instance: u64 },
    /// `close_explode_session()`.
    CloseExplodeSession,
    /// `set_pick_scope(objects, instances, frame_sketches)` — the editing
    /// scope the picks favor. Session-only view state, neither undoable
    /// nor saved, and recorded anyway (unlike the inference hide sets) so
    /// a replayed session ends up with the same scope as the original (a
    /// probe after replay then sees the same scoped picks). A recording
    /// stores no pick results and replay issues no picks. Changes nothing
    /// the state hash covers. Additive variant (the
    /// [`RecordedCall::SketchBeginCurveWith`] posture).
    SetPickScope {
        objects: Vec<u64>,
        instances: Vec<u64>,
        /// Whether the open session frame's own drawing is in scope.
        /// Additive field, always written by a current recorder: a
        /// recording made before it has none and replays with that call's
        /// original meaning (both lists empty cleared the scope; otherwise
        /// the frame's drawing was in it). The format version is unchanged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        frame_sketches: Option<bool>,
    },
    /// `clear_pick_scope()` — same posture as [`RecordedCall::SetPickScope`].
    ClearPickScope,
    /// `open_group_session(group)` (docs/design/group-session.md).
    OpenGroupSession { group: u64 },
    /// `close_group_session()`.
    CloseGroupSession,
    /// `rescale_session(factor, anchor)` — the Tape Measure's in-context
    /// resize of the innermost open session frame about a measured anchor
    /// (docs/design/group-session.md).
    RescaleSession { factor: f64, anchor: [f64; 3] },
    /// `push_pull_in_component(instance, object, face, distance)` —
    /// `instance` (not `component`) since `distance` is a WORLD-space length
    /// mapped through that specific instance's pose (delta-review fix on
    /// component-edit-parity.md phase A2: the raw distance previously
    /// committed diverged from the ghost preview's world-space sweep on a
    /// scaled instance).
    PushPullInComponent {
        instance: u64,
        object: u64,
        face: u64,
        distance: f64,
    },
    /// `set_node_name(kind, id, name)`.
    SetNodeName {
        kind: u8,
        id: u64,
        name: Option<String>,
    },
    /// `add_node_tag(kind, id, path)`.
    AddNodeTag {
        kind: u8,
        id: u64,
        path: Vec<String>,
    },
    /// `remove_node_tag(kind, id, path)`.
    RemoveNodeTag {
        kind: u8,
        id: u64,
        path: Vec<String>,
    },
    /// `add_node_tag_many(kinds, ids, path)` — one labeled compound over
    /// several nodes (multi-selection tagging). Additive variant.
    AddNodeTagMany {
        kinds: Vec<u8>,
        ids: Vec<u64>,
        path: Vec<String>,
    },
    /// `remove_node_tag_many(kinds, ids, path)`.
    RemoveNodeTagMany {
        kinds: Vec<u8>,
        ids: Vec<u64>,
        path: Vec<String>,
    },
    /// `set_tag_hidden(path, hidden)` — not undoable, but persisted with
    /// the document (manifest v5), so it must replay for the saved bytes
    /// and state hash to match.
    SetTagHidden { path: String, hidden: bool },
    /// `delete_tag(path)`.
    DeleteTag { path: String },
    /// `rename_tag(path, new_path)` — undoable, identity-preserving.
    RenameTag { path: String, new_path: String },
    /// `set_node_user_hidden(kind, id, hidden)` — persisted view state
    /// (manifest v6), same rationale as [`RecordedCall::SetTagHidden`].
    SetNodeUserHidden { kind: u8, id: u64, hidden: bool },
    /// `set_camera_state(projection, fov_deg, parallel_height, eye, target, up)` — the
    /// camera's working view (docs/design/camera.md §5): not undoable, but
    /// persisted with the document (manifest v13), same rationale as
    /// [`RecordedCall::SetTagHidden`]. Additive variant: a recording that
    /// never sets a camera state (every recording captured before this
    /// shipped) replays on older builds unchanged; one that does fails to
    /// parse there — loudly, never silently divergent. `parallel_height`
    /// came later (manifest v19): a recording without it replays with the
    /// height the perspective view spans at the target, as the app showed
    /// it then.
    SetCameraState {
        projection: String,
        fov_deg: f64,
        #[serde(default)]
        parallel_height: Option<f64>,
        eye: [f64; 3],
        target: [f64; 3],
        up: [f64; 3],
    },
    /// `set_section_plane(ox, oy, oz, nx, ny, nz, active)` — the document's
    /// section plane (docs/design/scenes.md §4): persisted view state
    /// (manifest v16), same rationale as [`RecordedCall::SetCameraState`].
    SetSectionPlane {
        origin: [f64; 3],
        normal: [f64; 3],
        active: bool,
    },
    /// `clear_section_plane()`.
    ClearSectionPlane,
    /// `set_view_style_json(json)` — the document's view style
    /// (`kernel::ViewStyle`, manifest v19): persisted view state, same
    /// rationale as [`RecordedCall::SetCameraState`]. `None` clears it.
    SetViewStyle { json: Option<String> },
    /// `add_scene(name, props, camera_json, display_json, after)` — Scenes
    /// (docs/design/scenes.md) are persisted view state; every Scene edit
    /// replays so saved bytes and the state hash match. Additive variants,
    /// same posture as [`RecordedCall::SetCameraState`].
    AddScene {
        name: Option<String>,
        props: u8,
        camera_json: Option<String>,
        display_json: Option<String>,
        after: Option<u64>,
    },
    /// `update_scene(sid, props, camera_json, display_json)`.
    UpdateScene {
        sid: u64,
        props: u8,
        camera_json: Option<String>,
        display_json: Option<String>,
    },
    /// `set_scene_props(sid, props, camera_json, display_json)`.
    SetSceneProps {
        sid: u64,
        props: u8,
        camera_json: Option<String>,
        display_json: Option<String>,
    },
    /// `rename_scene(sid, name)`.
    RenameScene { sid: u64, name: String },
    /// `set_scene_description(sid, text)`.
    SetSceneDescription { sid: u64, text: String },
    /// `set_scene_transition(sid, seconds)`.
    SetSceneTransition { sid: u64, seconds: Option<f64> },
    /// `set_scene_delay(sid, seconds)`.
    SetSceneDelay { sid: u64, seconds: Option<f64> },
    /// `move_scene(sid, index)`.
    MoveScene { sid: u64, index: u32 },
    /// `remove_scene(sid)`.
    RemoveScene { sid: u64 },
    /// `apply_scene(sid)` — writes tag visibility, user-hidden nodes, and
    /// the section plane, all persisted, so it must replay.
    ApplyScene { sid: u64 },
    /// `add_material(name, r, g, b, a)` — palette additions are not
    /// undoable but are saved, and later recorded paint calls reference
    /// the handle this call deterministically produces.
    AddMaterial {
        name: String,
        r: u8,
        g: u8,
        b: u8,
        a: u8,
    },
    /// `add_texture_material(...)` — embeds the encoded image bytes, so a
    /// recording with textures is self-contained (and correspondingly
    /// larger).
    #[allow(clippy::too_many_arguments)]
    AddTextureMaterial {
        name: String,
        r: u8,
        g: u8,
        b: u8,
        a: u8,
        image: Vec<u8>,
        format: u8,
        world_w: f64,
        world_h: f64,
    },
    /// `set_material_alpha(material, alpha)`.
    SetMaterialAlpha { material: u64, alpha: u8 },
    /// `set_material_pbr(material, metallic, roughness)`. Additive variant:
    /// old recordings replay unchanged.
    SetMaterialPbr {
        material: u64,
        metallic: f32,
        roughness: f32,
    },
    /// `set_material_name(material, name)` — palette material rename (v1.1
    /// assets lane). Additive variant (the [`RecordedCall::SketchBeginCurveWith`]
    /// posture): old recordings replay unchanged; one that renames a
    /// material fails to parse on older builds — loudly, never silently
    /// divergent.
    SetMaterialName { material: u64, name: String },
    /// `delete_material(material)` — palette material delete/tombstone
    /// (v1.1 assets lane). Additive variant, same posture as
    /// [`RecordedCall::SetMaterialName`].
    DeleteMaterial { material: u64 },
    /// `paint_face(object, face, material)` — `u64::MAX` = unpaint.
    PaintFace {
        object: u64,
        face: u64,
        material: u64,
    },
    /// `set_object_material(object, material)` — `u64::MAX` = clear.
    SetObjectMaterial { object: u64, material: u64 },
    /// `set_instance_material(instance, material)` — `u64::MAX` = clear.
    /// Additive variant: recordings that never paint a placement replay on
    /// older builds unchanged.
    SetInstanceMaterial { instance: u64, material: u64 },
    /// `set_definition_material(component, material)` — `u64::MAX` =
    /// clear. Additive, as [`RecordedCall::SetInstanceMaterial`].
    SetDefinitionMaterial { component: u64, material: u64 },
    /// `replace_material(document_wide, scope_object, from, to)` — the
    /// Shift-click "replace everywhere" gesture (paint-tool design §2);
    /// `scope_object` is meaningless when `document_wide` is true. `u64::MAX`
    /// on `from`/`to` = the unpainted sentinel, same convention as
    /// `paint_face`. Additive variant (the [`RecordedCall::SketchBeginCurveWith`]
    /// posture): recordings that never replace-everywhere replay on older
    /// builds unchanged; one that does fails to parse there — loudly, never
    /// silently divergent.
    ReplaceMaterial {
        document_wide: bool,
        scope_object: u64,
        from: u64,
        to: u64,
    },
    /// `set_face_uv_frame(object, face, frame)` — Position Texture's kernel
    /// commit (paint-tool design §3: the whole drag/pin gesture collapses to
    /// one call at commit). `frame: None` resets the face to the planar
    /// projection default. Additive variant (the
    /// [`RecordedCall::SketchBeginCurveWith`] posture): recordings that never
    /// position a texture replay on older builds unchanged; one that does
    /// fails to parse there — loudly, never silently divergent.
    SetFaceUvFrame {
        object: u64,
        face: u64,
        frame: Option<UvFrameRecorded>,
    },
    /// `add_guide_line(origin, dir)`.
    AddGuideLine { origin: [f64; 3], dir: [f64; 3] },
    /// `add_guide_point(p)`.
    AddGuidePoint { p: [f64; 3] },
    /// `delete_guide(guide)`.
    DeleteGuide { guide: u64 },
    /// `delete_all_guides()`.
    DeleteAllGuides,
    /// `transform_guide(guide, affine)` — Move on a guide. Additive
    /// variant (the [`RecordedCall::SketchBeginCurveWith`] posture): a
    /// recording that never moves a guide replays on older builds
    /// unchanged; one that does fails to parse there — loudly, never
    /// silently divergent.
    TransformGuide { guide: u64, affine: [f64; 12] },
    /// `duplicate_guide_array(guide, affine, count)` — Move's copy and
    /// ×N / /N array on a guide. Additive variant, as
    /// [`RecordedCall::TransformGuide`].
    DuplicateGuideArray {
        guide: u64,
        affine: [f64; 12],
        count: u32,
    },
    /// `add_linear_dimension(a_node_kind, a_node_id, a_point, b_node_kind,
    /// b_node_id, b_point, offset, plane, text_override)`. Additive variant
    /// (the [`RecordedCall::SketchBeginCurveWith`] posture): a recording
    /// that never dimensions replays on older builds unchanged; one that
    /// does fails to parse there — loudly, never silently divergent.
    #[allow(clippy::too_many_arguments)]
    AddLinearDimension {
        a_node_kind: i8,
        a_node_id: u64,
        a_point: [f64; 3],
        b_node_kind: i8,
        b_node_id: u64,
        b_point: [f64; 3],
        offset: [f64; 3],
        plane: [f64; 6],
        text_override: Option<String>,
    },
    /// `add_radial_dimension(anchor_node_kind, anchor_node_id, anchor_point,
    /// kind, curve_center, curve_radius, curve_plane, leader_dir,
    /// text_override)`. Additive variant, same posture as
    /// [`RecordedCall::AddLinearDimension`].
    #[allow(clippy::too_many_arguments)]
    AddRadialDimension {
        anchor_node_kind: i8,
        anchor_node_id: u64,
        anchor_point: [f64; 3],
        kind: String,
        curve_center: [f64; 3],
        curve_radius: f64,
        curve_plane: [f64; 6],
        leader_dir: [f64; 3],
        text_override: Option<String>,
    },
    /// `add_leader_text(anchor_node_kind, anchor_node_id, anchor_point,
    /// offset, text)`. Additive variant, same posture as
    /// [`RecordedCall::AddLinearDimension`].
    AddLeaderText {
        anchor_node_kind: i8,
        anchor_node_id: u64,
        anchor_point: [f64; 3],
        offset: [f64; 3],
        text: String,
    },
    /// `update_linear_dimension(id, ...)`. Additive variant, same posture as
    /// [`RecordedCall::AddLinearDimension`].
    #[allow(clippy::too_many_arguments)]
    UpdateLinearDimension {
        id: u64,
        a_node_kind: i8,
        a_node_id: u64,
        a_point: [f64; 3],
        b_node_kind: i8,
        b_node_id: u64,
        b_point: [f64; 3],
        offset: [f64; 3],
        plane: [f64; 6],
        text_override: Option<String>,
    },
    /// `update_radial_dimension(id, ...)`. Additive variant, same posture as
    /// [`RecordedCall::AddLinearDimension`].
    #[allow(clippy::too_many_arguments)]
    UpdateRadialDimension {
        id: u64,
        anchor_node_kind: i8,
        anchor_node_id: u64,
        anchor_point: [f64; 3],
        kind: String,
        curve_center: [f64; 3],
        curve_radius: f64,
        curve_plane: [f64; 6],
        leader_dir: [f64; 3],
        text_override: Option<String>,
    },
    /// `update_leader_text(id, ...)`. Additive variant, same posture as
    /// [`RecordedCall::AddLinearDimension`].
    UpdateLeaderText {
        id: u64,
        anchor_node_kind: i8,
        anchor_node_id: u64,
        anchor_point: [f64; 3],
        offset: [f64; 3],
        text: String,
    },
    /// `delete_annotation(id)`. Additive variant, same posture as
    /// [`RecordedCall::AddLinearDimension`].
    DeleteAnnotation { id: u64 },
    /// `insert_item(bytes, affine, source_id, content_hash, origin)` — a
    /// library insert or a paste: embeds the item file, the placement pose,
    /// and the provenance that drives idempotent definition reuse, so a
    /// session with an insert replays self-contained AND reproduces the
    /// same reuse-vs-copy decision. Additive variant (the
    /// [`RecordedCall::SketchBeginCurveWith`] posture): old recordings
    /// replay unchanged; one that inserts a library item fails to parse on
    /// older builds — loudly, never silently divergent.
    InsertItem {
        bytes: Vec<u8>,
        affine: [f64; 12],
        source_id: Option<String>,
        content_hash: Option<String>,
        /// The composed clipboard origin of a same-document paste
        /// (`Scene::clipboard_origin`), absent for a library placement.
        /// Recorded as composed, so replay compares it with the stamps
        /// inside `bytes` exactly as the live session did, whatever the
        /// replaying scene's own epoch is. Same additive posture as
        /// `stop_len` above: absent in older recordings, which replay
        /// unchanged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin: Option<String>,
    },
    /// `insert_item_palette(bytes)` — a material item's Add-to-palette:
    /// embeds the item file. Additive variant, same posture as
    /// [`RecordedCall::InsertItem`].
    InsertItemPalette { bytes: Vec<u8> },
    /// `stamp_library_source(kinds, ids, source_id, content_hash, def_sid)`
    /// — marks a saved item's source selection. Serialized document state
    /// changes (attribute dictionaries), so replay must re-issue it or the
    /// state hash diverges. Additive variant, same posture as
    /// [`RecordedCall::InsertItem`].
    StampLibrarySource {
        kinds: Vec<u8>,
        ids: Vec<u64>,
        source_id: String,
        content_hash: String,
        def_sid: Option<String>,
    },
    /// `import_dae(bytes, images)` — embeds the COLLADA file and its image
    /// map, so a session with an import replays self-contained.
    ImportDae {
        bytes: Vec<u8>,
        images: Vec<RecordedImage>,
    },
    /// `import_gltf(bytes)` — embeds the glTF/GLB file.
    ImportGltf { bytes: Vec<u8> },
    /// `import_skp(bytes)` — embeds the .skp file.
    ImportSkp { bytes: Vec<u8> },
    /// An import whose source file was too large to embed
    /// ([`EMBED_CAP`]): the recording keeps the fact and the size, so a
    /// replay fails loudly at this step instead of diverging silently, and
    /// a bug report is not doubled by a copy of a several-hundred-megabyte
    /// model the browser kernel is already holding. Additive variant (the
    /// [`RecordedCall::SketchBeginCurveWith`] posture).
    ImportOmitted { of: String, len: u64 },
    /// `import_stl(bytes, unit_scale, name)` — embeds the STL file, the
    /// units-chooser scale it was imported with (STL carries no units of its
    /// own, so replaying without the scale would reproduce the wrong geometry
    /// size), and the file-stem name the Objects were given (STL has no
    /// internal names, so replaying without it would rename the Objects and
    /// diverge the state hash). Additive variant (the
    /// [`RecordedCall::SketchBeginCurveWith`] posture): old recordings replay
    /// unchanged; one that imports an STL fails to parse on older builds —
    /// loudly, never silently divergent.
    ImportStl {
        bytes: Vec<u8>,
        unit_scale: f64,
        name: Option<String>,
    },
    /// `place_image(image, format, name, origin, u, v, width)` — Place
    /// Image's commit (and `hew.doc.import` of a picture), embedding the
    /// picture's bytes under the same size cap as the other imports
    /// (`ImportOmitted` past it). Additive variant (the
    /// [`RecordedCall::SketchBeginCurveWith`] posture): old recordings replay
    /// unchanged; one that places a picture fails to parse on older builds —
    /// loudly, never silently divergent.
    PlaceImage {
        image: Vec<u8>,
        format: u8,
        name: String,
        origin: [f64; 3],
        u: [f64; 3],
        v: [f64; 3],
        width: f64,
    },
    /// `set_axes(origin, x, y)` — the Axes tool's commit, or Reset Axes
    /// passing world identity's own components (tool-parity design §4).
    /// `z` is not recorded — it is always `x × y`, re-derived on replay
    /// exactly as the kernel derives it live. Additive variant (the
    /// [`RecordedCall::SketchBeginCurveWith`] posture): old recordings
    /// replay unchanged; one that moves the axes fails to parse on older
    /// builds — loudly, never silently divergent.
    SetAxes {
        origin: [f64; 3],
        x: [f64; 3],
        y: [f64; 3],
    },
    /// `load(bytes)` — a mid-session File ▸ Open/New replaces the whole
    /// document; embedding the `.hew` bytes keeps everything after it
    /// replayable from a fresh `Scene`.
    Load { bytes: Vec<u8> },
    // ---------------------------------------------------------------------
    // component-edit-parity.md phase K1 — additive variants (the
    // `SketchBeginCurveWith` posture): a recording that never draws/extrudes
    // inside a component replays unchanged on an older build; one that does
    // fails to parse there — loudly, never silently divergent.
    // ---------------------------------------------------------------------
    /// `begin_sketch_on_plane_in_instance(instance, px, py, pz, nx, ny, nz)`.
    BeginSketchOnPlaneInInstance {
        instance: u64,
        px: f64,
        py: f64,
        pz: f64,
        nx: f64,
        ny: f64,
        nz: f64,
    },
    /// `extrude_region_in_instance(instance, sketch, region, distance)`.
    ExtrudeRegionInInstance {
        instance: u64,
        sketch: u64,
        region: u64,
        distance: f64,
    },
    /// `delete_def_member(component, object)`.
    DeleteDefMember { component: u64, object: u64 },
    /// `cap_rims(object, rims)` — Model Health's cap (the import-health
    /// design R1). Additive variant (the
    /// [`RecordedCall::SketchBeginCurveWith`] posture): old recordings
    /// replay unchanged; one that caps fails to parse on older builds —
    /// loudly, never silently divergent. The same holds for the two below.
    CapRims {
        object: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rims: Option<Vec<u32>>,
    },
    /// `mend(object, plan_json)` — Model Health's Mend (the mesh-repair
    /// design). Additive variant: old recordings replay unchanged; one
    /// that mends fails to parse on older builds — loudly.
    Mend { object: u64, plan_json: String },
    /// `delete_faces(object, faces)` — Model Health's face removal (R2).
    DeleteFaces { object: u64, faces: Vec<u64> },
    /// `set_accepted_surface(object, accepted)` — Model Health's mark (R3).
    SetAcceptedSurface { object: u64, accepted: bool },
    /// `thicken_sheet(object, start, end)` — Model Health's pull of a flat
    /// sheet into a solid (R4).
    ThickenSheet { object: u64, start: f64, end: f64 },
    /// `thicken_sheet_in_instance(instance, object, distance)` — the Push/Pull
    /// tool pulling a flat surface member picked through an instance.
    ThickenSheetInInstance {
        instance: u64,
        object: u64,
        distance: f64,
    },
    /// `delete_definition(component)` — deletes a component definition and
    /// every instance that places it (v1.1 assets lane). Additive variant
    /// (the [`RecordedCall::SketchBeginCurveWith`] posture): old recordings
    /// replay unchanged; one that deletes a definition fails to parse on
    /// older builds — loudly, never silently divergent.
    DeleteDefinition { component: u64 },
    /// `purge_unused()` — deletes every unused definition and material as
    /// one undo entry (v1.1 assets lane). Recorded only when the call
    /// actually purged something (the kernel records no undo entry, and this
    /// mirrors that: an idle purge is a no-op replay would gain nothing
    /// from). Additive variant, same posture as
    /// [`RecordedCall::DeleteDefinition`].
    PurgeUnused,

    // ---------------------------------------------------------------------
    // component-edit-parity.md phase K2 — additive variants (the
    // `SketchBeginCurveWith` posture): a recording that never sweeps/
    // combines/slices/transforms inside a component replays unchanged on an
    // older build; one that does fails to parse there — loudly, never
    // silently divergent.
    // ---------------------------------------------------------------------
    /// `follow_me_along_edges_in_instance(instance, sketch, region,
    /// path_sketch, path_edges, stop_len)`.
    FollowMeAlongEdgesInInstance {
        instance: u64,
        sketch: u64,
        region: u64,
        path_sketch: u64,
        path_edges: Vec<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_len: Option<f64>,
    },
    /// `follow_me_around_face_in_instance(instance, sketch, region,
    /// path_object, path_face, stop_len)`.
    FollowMeAroundFaceInInstance {
        instance: u64,
        sketch: u64,
        region: u64,
        path_object: u64,
        path_face: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_len: Option<f64>,
    },
    /// `follow_me_merged_around_face_in_instance(instance, sketch, region,
    /// path_object, path_face, stop_len)`.
    FollowMeMergedAroundFaceInInstance {
        instance: u64,
        sketch: u64,
        region: u64,
        path_object: u64,
        path_face: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_len: Option<f64>,
    },
    /// `follow_me_face_along_edges_in_instance(instance, profile_object,
    /// profile_face, path_sketch, path_edges, stop_len)`.
    FollowMeFaceAlongEdgesInInstance {
        instance: u64,
        profile_object: u64,
        profile_face: u64,
        path_sketch: u64,
        path_edges: Vec<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_len: Option<f64>,
    },
    /// `follow_me_face_around_face_in_instance(instance, profile_object,
    /// profile_face, path_object, path_face, stop_len)`.
    FollowMeFaceAroundFaceInInstance {
        instance: u64,
        profile_object: u64,
        profile_face: u64,
        path_object: u64,
        path_face: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_len: Option<f64>,
    },
    /// `boolean_in_component(component, op, a, b)`.
    BooleanInComponent {
        component: u64,
        op: u8,
        a: u64,
        b: u64,
    },
    /// `boolean_in_component_many(component, op, ids)`: the same over any
    /// number of members of one definition. Additive variant.
    BooleanInComponentMany {
        component: u64,
        op: u8,
        ids: Vec<u64>,
    },
    /// `boolean_through_placement_json(path, host, tool, op)`.
    BooleanThroughPlacement {
        path: Vec<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host: Option<u64>,
        tool: u64,
        op: u8,
    },
    /// `slice_def_member(instance, object, plane)`.
    SliceDefMember {
        instance: u64,
        object: u64,
        plane: [f64; 6],
    },
    /// `transform_def_member(instance, object, affine)`.
    TransformDefMember {
        instance: u64,
        object: u64,
        affine: [f64; 12],
    },
    /// `transform_def_sketch(instance, sketch, affine)`.
    TransformDefSketch {
        instance: u64,
        sketch: u64,
        affine: [f64; 12],
    },
    /// `transform_def_sketch_island(instance, sketch, island, affine)`.
    TransformDefSketchIsland {
        instance: u64,
        sketch: u64,
        island: u64,
        affine: [f64; 12],
    },
    /// `transform_def_selection(instance, objects, sketches,
    /// island_sketches, islands, affine)`.
    TransformDefSelection {
        instance: u64,
        objects: Vec<u64>,
        sketches: Vec<u64>,
        island_sketches: Vec<u64>,
        islands: Vec<u64>,
        affine: [f64; 12],
    },
    /// One document-mutating envelope that arrived over the live API
    /// (`Scene::api_dispatch` — an agent or script driving the open
    /// document, docs/agents/HEW_API.md §11.2), stored as the raw JSON-RPC
    /// request text.
    ///
    /// Recorded at API altitude rather than expanded into the kernel
    /// calls it performs, deliberately. The API is the versioned public
    /// contract (§14's evolution rules), so a recorded envelope stays
    /// replayable across kernel refactors that would invalidate a
    /// kernel-shaped capture; and one envelope may be a whole
    /// transaction, whose label, origin, and single-compound-undo-entry
    /// grouping are exactly what a faithful reproducer has to preserve.
    /// Expanding it would reproduce the geometry while silently losing
    /// the history structure the user sees.
    ///
    /// The stored frame's public ids resolve on replay for the same
    /// reason recorded slotmap handles do (see this module's header): the
    /// kernel is deterministic, public ids derive from stable ids minted
    /// in op order, so re-issuing the identical sequence into a fresh
    /// document mints the identical ids.
    ///
    /// Only frames that actually mutated are captured, on the same
    /// condition that triggers the post-dispatch resync — so a recording
    /// and the viewport can never disagree about what changed. Read-only
    /// traffic (an agent polling `hew.query.scene`) is not recorded, and
    /// no captured frame can carry a discovery token: the handshake that
    /// bears one is read-only.
    ApiDispatch {
        /// The JSON-RPC request text, verbatim.
        frame: String,
    },
    /// `delete_selection(kinds, ids)` — the Delete-key/Cut batch delete
    /// (Lane D, v1.1-cycle.md): every listed node removed as ONE undo step,
    /// `add_node_tag_many`'s labeled-compound shape. Additive variant: a
    /// recording that never batch-deletes (every recording captured before
    /// this shipped, including the individual `DeleteNode` calls it
    /// replaces going forward) replays on older builds unchanged; one that
    /// does fails to parse there — loudly, never silently divergent.
    DeleteSelection { kinds: Vec<u8>, ids: Vec<u64> },
    /// `reparent_nodes(kinds, ids, group)` — the Outliner drag-and-drop
    /// "Add to Group"/"Move to top level" (v1.1-cycle.md grouping lane):
    /// every listed node moved as ONE undo step, `DeleteSelection`'s
    /// labeled-compound shape. Additive variant: a recording that never
    /// reparents (every recording captured before this shipped) replays
    /// on older builds unchanged; one that does fails to parse there —
    /// loudly, never silently divergent.
    ReparentNodes {
        kinds: Vec<u8>,
        ids: Vec<u64>,
        group: Option<u64>,
    },
    // ---------------------------------------------------------------------
    // Unified drafting (docs/design/unified-drafting.md §4/§6). Additive
    // variants (the `SketchBeginCurveWith` posture): a recording that never
    // drafts replays on older builds unchanged; one that does fails to
    // parse there, loudly. Every point is WORLD space as called; `draft` is
    // the handle the call carried (a begin variant re-mints exactly it).
    // ---------------------------------------------------------------------
    /// `draft_begin_plane(px..nz, label)`.
    DraftBeginPlane {
        draft: u64,
        px: f64,
        py: f64,
        pz: f64,
        nx: f64,
        ny: f64,
        nz: f64,
        label: String,
    },
    /// `draft_begin_plane_in_instance(instance, px..nz, label)`.
    DraftBeginPlaneInInstance {
        draft: u64,
        instance: u64,
        px: f64,
        py: f64,
        pz: f64,
        nx: f64,
        ny: f64,
        nz: f64,
        label: String,
    },
    /// `draft_begin_face(object, face, label)`.
    DraftBeginFace {
        draft: u64,
        object: u64,
        face: u64,
        label: String,
    },
    /// `draft_begin_face_in_instance(instance, object, face, label)`.
    DraftBeginFaceInInstance {
        draft: u64,
        instance: u64,
        object: u64,
        face: u64,
        label: String,
    },
    /// `draft_begin_sketch(sketch, label)`.
    DraftBeginSketch {
        draft: u64,
        sketch: u64,
        label: String,
    },
    /// `draft_begin_ctx(ctx_json, label)`.
    DraftBeginCtx {
        draft: u64,
        ctx: String,
        label: String,
    },
    /// `draft_begin_detached(ctx_json, label)`: a draft that draws one new
    /// detached shape (detached shapes §3.7). Additive (the posture above):
    /// a recording that never draws a detached shape replays on older builds
    /// unchanged; one that does fails to parse there, loudly.
    DraftBeginDetached {
        draft: u64,
        ctx: String,
        label: String,
    },
    /// `draft_end(draft)`.
    DraftEnd { draft: u64 },
    /// `draft_cancel(draft)`.
    DraftCancel { draft: u64 },
    /// `draft_add_segment(draft, a, b)`.
    DraftAddSegment {
        draft: u64,
        a: [f64; 3],
        b: [f64; 3],
    },
    /// `draft_begin_curve_with(draft, center, radius)`.
    DraftBeginCurveWith {
        draft: u64,
        center: [f64; 3],
        radius: f64,
    },
    /// `draft_begin_polygon_with(draft, center, radius)`.
    DraftBeginPolygonWith {
        draft: u64,
        center: [f64; 3],
        radius: f64,
    },
    /// `draft_end_curve(draft)`.
    DraftEndCurve { draft: u64 },
    /// `draft_begin_curve(draft)`.
    DraftBeginCurve { draft: u64 },
    /// `history_bracket_open()`.
    HistoryBracketOpen,
    /// `history_bracket_commit(label)`.
    HistoryBracketCommit { label: String },
    /// `history_bracket_abort()`.
    HistoryBracketAbort,
    /// `draft_remove_edges(draft, keys)`.
    DraftRemoveEdges { draft: u64, keys: Vec<u64> },
    /// `draft_transform_edges(draft, keys, affine)`.
    DraftTransformEdges {
        draft: u64,
        keys: Vec<u64>,
        affine: [f64; 12],
    },
    /// `draft_copy_edges(draft, keys, affine)`.
    DraftCopyEdges {
        draft: u64,
        keys: Vec<u64>,
        affine: [f64; 12],
    },
    /// `draft_move_vertex(draft, at, to)`.
    DraftMoveVertex {
        draft: u64,
        at: [f64; 3],
        to: [f64; 3],
    },
    /// `draft_offset_region(draft, region, distance)` — `region` is a key.
    DraftOffsetRegion {
        draft: u64,
        region: u64,
        distance: f64,
    },
    /// `draft_refacet_curve(draft, curve, segments)` — `curve` is a key.
    DraftRefacetCurve {
        draft: u64,
        curve: u64,
        segments: u32,
    },
    /// One forwarded plugin command's compound (docs/agents/PLUGINS.md
    /// "Compounds"): the calls its handler made while the compound was
    /// open, which landed as one undo entry. The forwarded frame itself is
    /// never recorded (no plugin runs at replay time); its children are,
    /// here and nowhere else, so an aborted compound records nothing.
    ///
    /// Replay opens a connection holding `grant` (identity `plugin:<id>`),
    /// opens a kernel compound labeled `label` with that origin, replays
    /// `calls` in order (a nested plugin command's compound is one of
    /// them), and ends the compound: one entry, as live. Additive variant:
    /// a recording without one replays on older builds unchanged.
    PluginCompound {
        /// The provider plugin's id.
        plugin: String,
        /// The compound's label: the command's declared label, or its name.
        label: String,
        /// The provider's grant, as JSON (`api::Grant`).
        grant: serde_json::Value,
        /// The calls recorded while the compound was open, in order.
        calls: Vec<RecordedCall>,
    },
    /// `make_solid_from_sketches(sketches)`: Make Solid on drawn faces
    /// (wireframe faces §6). Additive (the posture above): a recording that
    /// never makes a solid from faces replays on older builds unchanged; one
    /// that does fails to parse there, loudly.
    MakeSolidFromSketches { sketches: Vec<u64> },
}

/// A [`kernel::UvFrame`]'s components, flattened for [`RecordedCall::SetFaceUvFrame`]
/// (the kernel type has no `Serialize`/`Deserialize` — kernel crates stay free
/// of that dependency, DEVELOPMENT.md rule 1).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct UvFrameRecorded {
    pub sx: f64,
    pub sy: f64,
    pub sz: f64,
    pub tx: f64,
    pub ty: f64,
    pub tz: f64,
    pub u0: f64,
    pub v0: f64,
}

/// One image of an [`RecordedCall::ImportDae`] call's image map.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordedImage {
    /// The URI key the COLLADA file references the image by.
    pub uri: String,
    /// The encoded image bytes.
    pub bytes: Vec<u8>,
    /// `0` = PNG, `1` = JPEG (the `import_dae` images convention).
    pub format: u8,
}

/// A complete recorded session: the committed call stream plus the canonical
/// `state_hash` it produced. Replaying `calls` verbatim into a fresh
/// `Scene` must reproduce `golden_hash`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    /// Format version (`RECORDING_FORMAT_VERSION`).
    pub version: u32,
    /// The committed mutations, in application order.
    pub calls: Vec<RecordedCall>,
    /// The document `state_hash` after the last recorded call — the replay oracle.
    pub golden_hash: u64,
}

thread_local! {
    /// Whether capture is active (toggled by [`start`]/[`stop`]). Held off during
    /// replay so re-issued calls are not re-recorded.
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    /// Calls captured so far.
    static CALLS: RefCell<Vec<RecordedCall>> = const { RefCell::new(Vec::new()) };
    /// One buffer per open compound, innermost last ([`push_buffer`]):
    /// while one exists, [`record`] appends to it instead of to `CALLS`.
    static BUFFERS: RefCell<Vec<Vec<RecordedCall>>> = const { RefCell::new(Vec::new()) };
}

/// Begins capture, discarding any prior in-progress recording. The caller should
/// be on a fresh/empty document for the golden to be replayable from `Scene::new`.
pub fn start() {
    CALLS.with(|c| c.borrow_mut().clear());
    ENABLED.with(|e| e.set(true));
}

/// Stops capture; the accumulated calls remain available to [`take_calls`].
pub fn stop() {
    ENABLED.with(|e| e.set(false));
}

/// Whether capture is active.
pub fn is_active() -> bool {
    ENABLED.with(|e| e.get())
}

/// Largest source file an import call embeds in the recording; larger
/// imports record as [`RecordedCall::ImportOmitted`].
pub const EMBED_CAP: usize = 64 << 20;

/// Records an import of `bytes` under `method`: embedded through `embed`
/// when the file fits [`EMBED_CAP`], as [`RecordedCall::ImportOmitted`]
/// otherwise. The copy is only made when capture is active.
pub fn record_import(method: &str, bytes: &[u8], embed: impl FnOnce(Vec<u8>) -> RecordedCall) {
    record_import_sized(method, bytes.len(), || embed(bytes.to_vec()));
}

/// [`record_import`] for an import whose embedded payload is more than one
/// byte buffer (a COLLADA file plus its images): `total_len` is the sum the
/// cap applies to, and `embed` builds the call — and makes its copies —
/// only when capture is active and the total fits.
pub fn record_import_sized(method: &str, total_len: usize, embed: impl FnOnce() -> RecordedCall) {
    if !is_active() {
        return;
    }
    if total_len > EMBED_CAP {
        record(RecordedCall::ImportOmitted {
            of: method.to_string(),
            len: total_len as u64,
        });
    } else {
        record(embed());
    }
}

/// Appends one committed call — a no-op unless capture is active. Call this
/// **after** the mutation succeeds, so failed/refused ops are never recorded.
pub fn record(call: RecordedCall) {
    if !is_active() {
        return;
    }
    let call = BUFFERS.with(|b| match b.borrow_mut().last_mut() {
        Some(buffer) => {
            buffer.push(call);
            None
        }
        None => Some(call),
    });
    if let Some(call) = call {
        CALLS.with(|c| c.borrow_mut().push(call));
    }
}

/// Routes [`record`] into a fresh buffer until the matching [`pop_buffer`]:
/// a compound's children are held until it ends (recorded as one
/// [`RecordedCall::PluginCompound`]) or aborts (dropped). Nests; pushes
/// whether or not capture is active, so push and pop always pair.
pub fn push_buffer() {
    BUFFERS.with(|b| b.borrow_mut().push(Vec::new()));
}

/// Takes the innermost buffer [`push_buffer`] opened; empty when none is
/// open or capture was off.
pub fn pop_buffer() -> Vec<RecordedCall> {
    BUFFERS.with(|b| b.borrow_mut().pop().unwrap_or_default())
}

/// Takes the captured calls, clearing the buffer.
pub fn take_calls() -> Vec<RecordedCall> {
    CALLS.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

/// Copies the captured calls without clearing the buffer, for a bug report
/// that must not take the calls a later crash reproducer needs.
pub fn peek_calls() -> Vec<RecordedCall> {
    CALLS.with(|c| c.borrow().clone())
}

/// The captured calls as [`Recording`] JSON for the panic hook, or `None`
/// when there is nothing to capture. Never panics and never clears: the
/// thread-local may be mid-borrow (a panic raised inside [`record`]) or
/// already torn down, and either yields `None` rather than a second panic
/// inside the hook. A panicking frame cannot hash the document, so
/// `golden_hash` is `0`: the artifact reproduces the panic on replay but is
/// not a regression oracle.
pub fn panic_snapshot_json() -> Option<String> {
    #[derive(Serialize)]
    struct RecordingView<'a> {
        version: u32,
        calls: &'a [RecordedCall],
        golden_hash: u64,
    }
    // The calls a compound still holds follow the recorded ones (outermost
    // first), so a panic mid-compound still reproduces.
    let buffered: Vec<RecordedCall> = BUFFERS
        .try_with(|b| {
            b.try_borrow()
                .map(|buffers| buffers.iter().flatten().cloned().collect())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    CALLS
        .try_with(|c| {
            let calls = c.try_borrow().ok()?;
            if calls.is_empty() && buffered.is_empty() {
                return None;
            }
            let all: Vec<RecordedCall> = calls.iter().cloned().chain(buffered).collect();
            serde_json::to_string(&RecordingView {
                version: RECORDING_FORMAT_VERSION,
                calls: &all,
                golden_hash: 0,
            })
            .ok()
        })
        .ok()
        .flatten()
}

/// Runs `body` with capture suppressed (used during replay so re-issued calls
/// don't re-record), restoring the prior state after.
pub fn without_capture<R>(body: impl FnOnce() -> R) -> R {
    let prev = ENABLED.with(|e| e.replace(false));
    let out = body();
    ENABLED.with(|e| e.set(prev));
    out
}

#[cfg(test)]
pub fn reset() {
    ENABLED.with(|e| e.set(false));
    CALLS.with(|c| c.borrow_mut().clear());
    BUFFERS.with(|b| b.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_only_while_active() {
        reset();
        record(RecordedCall::BeginGroundSketch); // ignored — not started
        start();
        record(RecordedCall::BeginGroundSketch);
        record(RecordedCall::ExtrudeRegion {
            sketch: 1,
            region: 2,
            distance: 3.0,
        });
        stop();
        record(RecordedCall::BeginGroundSketch); // ignored — stopped
        let calls = take_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], RecordedCall::BeginGroundSketch);
    }

    #[test]
    fn peek_calls_copies_without_clearing() {
        reset();
        start();
        record(RecordedCall::BeginGroundSketch);
        assert_eq!(peek_calls(), vec![RecordedCall::BeginGroundSketch]);
        assert_eq!(take_calls().len(), 1, "peeking left the buffer for take");
    }

    #[test]
    fn panic_snapshot_parses_as_a_recording_and_leaves_the_buffer() {
        reset();
        assert_eq!(
            panic_snapshot_json(),
            None,
            "nothing captured, nothing handed over"
        );
        start();
        record(RecordedCall::BeginGroundSketch);
        let json = panic_snapshot_json().expect("a captured call is snapshotted");
        let rec: Recording = serde_json::from_str(&json).expect("parses as a Recording");
        assert_eq!(rec.version, RECORDING_FORMAT_VERSION);
        assert_eq!(rec.calls, vec![RecordedCall::BeginGroundSketch]);
        assert_eq!(rec.golden_hash, 0, "a panicking frame has no golden");
        assert_eq!(take_calls().len(), 1, "the snapshot did not clear");
    }

    #[test]
    fn panic_snapshot_yields_none_while_the_buffer_is_borrowed() {
        reset();
        start();
        record(RecordedCall::BeginGroundSketch);
        CALLS.with(|c| {
            let _held = c.borrow_mut();
            assert_eq!(panic_snapshot_json(), None);
        });
    }

    #[test]
    fn buffers_hold_calls_until_popped_and_nest() {
        reset();
        start();
        record(RecordedCall::BeginGroundSketch);
        push_buffer();
        record(RecordedCall::HistoryBracketOpen);
        push_buffer();
        record(RecordedCall::HistoryBracketAbort);
        let rec: Recording =
            serde_json::from_str(&panic_snapshot_json().expect("snapshot")).expect("parses");
        assert_eq!(
            rec.calls,
            vec![
                RecordedCall::BeginGroundSketch,
                RecordedCall::HistoryBracketOpen,
                RecordedCall::HistoryBracketAbort
            ],
            "a panic mid-compound still carries the buffered calls"
        );
        assert_eq!(pop_buffer(), vec![RecordedCall::HistoryBracketAbort]);
        assert_eq!(pop_buffer(), vec![RecordedCall::HistoryBracketOpen]);
        assert_eq!(pop_buffer(), Vec::new(), "nothing open pops empty");
        assert_eq!(take_calls(), vec![RecordedCall::BeginGroundSketch]);
    }

    #[test]
    fn plugin_compound_round_trips_through_json() {
        let call = RecordedCall::PluginCompound {
            plugin: "com.example.parts".into(),
            label: "Cut list".into(),
            grant: serde_json::json!({"plugin": "com.example.parts"}),
            calls: vec![RecordedCall::ApiDispatch { frame: "{}".into() }],
        };
        let json = serde_json::to_value(&call).unwrap();
        assert_eq!(json["method"], "plugin_compound");
        assert_eq!(json["calls"][0]["method"], "api_dispatch");
        let back: RecordedCall = serde_json::from_value(json).unwrap();
        assert_eq!(back, call);
    }

    #[test]
    fn without_capture_suppresses_recording() {
        reset();
        start();
        without_capture(|| record(RecordedCall::BeginGroundSketch));
        assert!(is_active(), "capture state restored after the closure");
        assert!(
            take_calls().is_empty(),
            "the suppressed call was not recorded"
        );
    }

    #[test]
    fn calls_round_trip_through_json() {
        let rec = Recording {
            version: RECORDING_FORMAT_VERSION,
            calls: vec![
                RecordedCall::BeginGroundSketch,
                RecordedCall::SketchAddSegment {
                    sketch: 5,
                    a: [0.0, 0.0, 0.0],
                    b: [1.0, 0.0, 0.0],
                },
                RecordedCall::ExtrudeRegion {
                    sketch: 5,
                    region: 9,
                    distance: 2.0,
                },
            ],
            golden_hash: 0xABCD,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"method\":\"extrude_region\""));
        let back: Recording = serde_json::from_str(&json).unwrap();
        assert_eq!(back.version, rec.version);
        assert_eq!(back.golden_hash, rec.golden_hash);
        assert_eq!(back.calls, rec.calls);
    }
}
