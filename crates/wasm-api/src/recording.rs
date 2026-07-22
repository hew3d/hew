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
    /// `transform_instance(instance, affine)`.
    TransformInstance { instance: u64, affine: [f64; 12] },
    /// `explode_instance(instance)`.
    ExplodeInstance { instance: u64 },
    /// `make_unique(instance)`.
    MakeUnique { instance: u64 },
    /// `push_pull_in_component(component, object, face, distance)`.
    PushPullInComponent {
        component: u64,
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
    /// `add_guide_line(origin, dir)`.
    AddGuideLine { origin: [f64; 3], dir: [f64; 3] },
    /// `add_guide_point(p)`.
    AddGuidePoint { p: [f64; 3] },
    /// `delete_guide(guide)`.
    DeleteGuide { guide: u64 },
    /// `delete_all_guides()`.
    DeleteAllGuides,
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
    /// `load(bytes)` — a mid-session File ▸ Open/New replaces the whole
    /// document; embedding the `.hew` bytes keeps everything after it
    /// replayable from a fresh `Scene`.
    Load { bytes: Vec<u8> },
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
