//! Replayable command recording (docs/DEVELOPMENT.md,  +).
//!
//! A recording is the ordered stream of committed `Scene` mutations as **typed,
//! replayable calls**. Replaying them verbatim into a fresh [`Scene`] reproduces
//! the session, and the final [`state_hash`](crate::Scene::state_hash) is
//! asserted against the recorded golden — the regression guarantee (docs/DEVELOPMENT.md).
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
//! shape is frozen in `docs/DIAGNOSTICS.md` — the handshake for the
//! Node runner and the M17 bug-report bundle.

use std::cell::{Cell, RefCell};

use serde::{Deserialize, Serialize};

/// Bump on any breaking change to the [`Recording`] JSON shape. v2 = typed
/// replayable calls (v1 was the log-tap hash-chain). See
/// `docs/DIAGNOSTICS.md`.
pub const RECORDING_FORMAT_VERSION: u32 = 2;

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
    },
    /// `split_face_inner(object, face, loop_pts)` — imprint a closed loop on a
    /// solid face (draw-on-face). `curve`, when present, is the drawn circle's
    /// analytic identity `[center.x, center.y, center.z, radius]` and routes to
    /// `split_face_inner_with_curve` so a later push-through stamps the tunnel
    /// walls (the true-curves design, playtest fix C3). Additive variant:
    /// recordings that never imprint on a face replay on older builds
    /// unchanged; one that does fails to parse there (loudly, never silently
    /// divergent), the same posture as `SketchBeginCurveWith`.
    SplitFaceInner {
        object: u64,
        face: u64,
        loop_pts: Vec<f64>,
        curve: Option<[f64; 4]>,
    },
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
    /// `open_explode_session(instance)` (docs/design/explode-session-
    /// prototype.md, a `proto/explode-session` prototype).
    OpenExplodeSession { instance: u64 },
    /// `close_explode_session()`.
    CloseExplodeSession,
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
    /// `split_face(object, face, path)` — `path` is xyz triples.
    SplitFace {
        object: u64,
        face: u64,
        path: Vec<f64>,
    },
    /// `merge_faces(object, edge)`.
    MergeFaces { object: u64, edge: u64 },
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
    /// `set_tag_hidden(path, hidden)` — not undoable, but persisted with
    /// the document (manifest v5), so it must replay for the saved bytes
    /// and state hash to match.
    SetTagHidden { path: String, hidden: bool },
    /// `delete_tag(path)`.
    DeleteTag { path: String },
    /// `set_node_user_hidden(kind, id, hidden)` — persisted view state
    /// (manifest v6), same rationale as [`RecordedCall::SetTagHidden`].
    SetNodeUserHidden { kind: u8, id: u64, hidden: bool },
    /// `set_camera_state(projection, fov_deg, eye, target, up)` — the
    /// camera's working view (docs/design/camera.md §5): not undoable, but
    /// persisted with the document (manifest v13), same rationale as
    /// [`RecordedCall::SetTagHidden`]. Additive variant: a recording that
    /// never sets a camera state (every recording captured before this
    /// shipped) replays on older builds unchanged; one that does fails to
    /// parse there — loudly, never silently divergent.
    SetCameraState {
        projection: String,
        fov_deg: f64,
        eye: [f64; 3],
        target: [f64; 3],
        up: [f64; 3],
    },
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
    /// `paint_face(object, face, material)` — `u64::MAX` = unpaint.
    PaintFace {
        object: u64,
        face: u64,
        material: u64,
    },
    /// `set_object_material(object, material)` — `u64::MAX` = clear.
    SetObjectMaterial { object: u64, material: u64 },
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
    /// `split_face_in_instance(instance, object, face, path)` — `path` is
    /// xyz triples, in WORLD space (mapped through the instance's pose⁻¹ at
    /// call time, so replay reproduces the exact same mapping).
    SplitFaceInInstance {
        instance: u64,
        object: u64,
        face: u64,
        path: Vec<f64>,
    },
    /// `split_face_inner_in_instance(instance, object, face, loop_pts)` /
    /// `split_face_inner_with_curve_in_instance(…, center, radius)` — the
    /// definition-member analog of [`RecordedCall::SplitFaceInner`]
    /// (Rectangle/Circle/Polygon's face-mode draw inside a component;
    /// component-edit-parity.md phase A2). `loop_pts` is xyz triples in
    /// WORLD space (mapped through the instance's pose⁻¹ at call time, same
    /// as `SplitFaceInInstance`'s `path`).
    SplitFaceInnerInInstance {
        instance: u64,
        object: u64,
        face: u64,
        loop_pts: Vec<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        curve: Option<[f64; 4]>,
    },
    /// `delete_def_member(component, object)`.
    DeleteDefMember { component: u64, object: u64 },

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

/// Appends one committed call — a no-op unless capture is active. Call this
/// **after** the mutation succeeds, so failed/refused ops are never recorded.
pub fn record(call: RecordedCall) {
    if is_active() {
        CALLS.with(|c| c.borrow_mut().push(call));
    }
}

/// Takes the captured calls, clearing the buffer.
pub fn take_calls() -> Vec<RecordedCall> {
    CALLS.with(|c| std::mem::take(&mut *c.borrow_mut()))
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
