//! Document-level fuzz harness (DEVELOPMENT.md rule 3): random document ops —
//! object edits, booleans, transforms, duplicate/delete, group/ungroup,
//! components, instances, definition-member edits (`apply_def_op`), 3D
//! Text placement (`place_text`), whole-document rescales
//! (`rescale_document`), and movable-axes moves (`set_axes`) — interleaved
//! undo/redo — over a document seeded with sketch-extruded boxes, with
//! torture mode on (the always-on validator panics at the offending op
//! instead of committing a violation).
//!
//! Invariants:
//! - after every op (applied or refused typed) every visible object validates
//!   and stays watertight;
//! - every undo/redo dispatch succeeds (DEVELOPMENT.md rule 9 — history
//!   replay is guard-exempt with proof; no failure signature is tolerated),
//!   with the single tolerated exception of a typed `RestoreConflicts`
//!   refusal against a plain (non-`PlaceTextCompound`) `CreatedObject` (see
//!   `is_known_inverse_guard_gap`) — a `PlaceTextCompound` 3D-Text placement
//!   is NOT exempted: `place_text` always retires its scratch sketch before
//!   returning, so that refusal path is expected to be unreachable from
//!   here, and its firing is treated as a genuine defect rather than
//!   silently swallowed;
//! - `save()` is deterministic, `load(save())` reproduces the same
//!   `state_hash`, after every op and at the post-sequence and maximal
//!   states;
//! - every detached sketch (`Sketch::detached`) an op does not itself edit
//!   is `==` before and after it (detached shapes §3.8): no plane draft,
//!   imprint, move, or extrusion ever arranges with one (see
//!   `check_detached_untouched`);
//! - fully unwinding the document log and replaying it twice reproduces the
//!   same states up to tolerance-aware equivalence (canonical fingerprints).
//!   The canonical geometry writer makes save bytes independent of slot
//!   allocation (see `doc_replay_diverge_repro.rs`), but cycles are compared
//!   by fingerprint, not bytes, because baked translations round-trip with
//!   ulp noise (`fl(fl(x + d) - d) != x` — the DEVELOPMENT.md fp trap), and
//!   that noise is exactly what tolerance-aware equivalence exists for.

use kernel::{
    Anchor, Annotation, BooleanOp, CapturedCurve, Document, DocumentError, FaceId, KernelOp,
    KernelOpError, Material, MaterialId, MaterialScope, NodeId, ObjectId, PendingActionKind, Plane,
    Point3, PushPullError, RadialKind, Rgba8, SketchError, SketchRegionId, Transform, UvFrame,
    Vec3,
};
use proptest::prelude::*;

mod face_arrangement_support;
use face_arrangement_support::{draw_loop, face_plane, group_faces, no_pinch};

/// Abstract document op; selectors resolve against live ids at apply time.
#[derive(Debug, Clone)]
enum DocOp {
    /// Push/pull the `face_sel`-th face of the `obj_sel`-th visible object.
    PushPull {
        obj_sel: usize,
        face_sel: usize,
        distance: f64,
    },
    /// Straight cut across a face of a visible object.
    SplitFace {
        obj_sel: usize,
        face_sel: usize,
        edge_a: usize,
        edge_b: usize,
        ta: f64,
        tb: f64,
    },
    /// Imprint a loop into a face of a visible object: a shrunk boundary
    /// copy, or (`staple: true`, quad faces) a concave U-shaped loop whose
    /// vertex average lies outside it — the hole-reassignment trap shape.
    SplitFaceInner {
        obj_sel: usize,
        face_sel: usize,
        shrink: f64,
        staple: bool,
    },
    /// Draw a triangle on a face of a visible object through the document's
    /// face-draw entry point, its corners snapped onto three distinct edges
    /// of the face's outer loop (or, `pull_in`, the third corner pulled
    /// toward the face's center so only two touch): the face-arrangement
    /// planner's touching-loop route.
    ImprintTouchingLoop {
        obj_sel: usize,
        face_sel: usize,
        edges: [usize; 3],
        ts: [f64; 3],
        pull_in: bool,
    },
    /// Draw a 24-gon on a face of a visible object with one vertex exactly
    /// on a side of the face's outer loop (at parameter `t`) and the rest
    /// inside, as the Circle tool does with its rim snapped onto an edge:
    /// a one-point touch, which lands as a disc face in a hole sharing that
    /// vertex with the outer loop (or a second circle `twin` touching the
    /// first at its far point: two holes sharing a vertex). The radius is
    /// `frac` of a size that fits a convex hole-free face; anywhere else a
    /// typed refusal or kept drawing is a correct answer.
    TangentCircle {
        obj_sel: usize,
        face_sel: usize,
        side: usize,
        t: f64,
        frac: f64,
        twin: bool,
    },
    /// A cut around the outside of an imprint, from one of its hole edges
    /// back to the same edge (a hole-anchored `split_face`), on a face of a
    /// visible object; a hole-free rectangular face first gets a centered
    /// half-size imprint, which makes the cut's acceptance deterministic.
    /// `merge_back` then deletes one of the cut's edges again — a merge
    /// across the hole boundary, which must restore the face (rule 3).
    HoleChord {
        obj_sel: usize,
        face_sel: usize,
        side: usize,
        ts: (f64, f64),
        frac: f64,
        merge_back: bool,
    },
    /// Draw a loop ACROSS one side of an imprint through the document's
    /// face-draw entry point — a quad, or (`circle`) a 24-gon turned so no
    /// vertex sits on the side, straddling it — so one piece cuts the
    /// imprint and the other cuts the face around it from the imprint's
    /// hole back to it (the planner's crossing route). A hole-free
    /// rectangular face first gets a centered half-size imprint, which
    /// makes acceptance deterministic; `from_inside` draws from the
    /// imprint rather than the face holding it.
    CrossingLoop {
        obj_sel: usize,
        face_sel: usize,
        side: usize,
        ts: (f64, f64),
        frac: f64,
        circle: bool,
        from_inside: bool,
    },
    /// On a hole-free rectangular face, two side-by-side imprints (which
    /// makes acceptance deterministic), then a loop drawn across both — a
    /// rectangle from inside one to inside the other, or (`circle`) a
    /// 24-gon centered in the gap reaching into both: its pieces between
    /// the imprints join their two holes (bridges), so the face is
    /// re-traced (stage 4). Must be accepted as four pieces.
    BridgeLoop {
        obj_sel: usize,
        face_sel: usize,
        ts: (f64, f64),
        frac: f64,
        circle: bool,
    },
    /// On a hole-free rectangular face, two side-by-side imprints and a
    /// rectangle flush between them (its pieces bridge the two holes, which
    /// merge into one), then Delete on the strip's top or bottom line: a
    /// merge across two separate runs, which splits the hole in two again.
    /// Both steps must be accepted.
    MultiRunMerge {
        obj_sel: usize,
        face_sel: usize,
        ts: (f64, f64),
        top: bool,
    },
    /// Delete (merge) an edge between two coplanar faces of a visible
    /// object — two outer loops, or an outer loop and a hole loop (a
    /// leftover's run along an imprint, or an imprint's own edge, refused as
    /// a whole-imprint dissolve). `from_features` picks a run the object's
    /// topology drawing unit of one of the object's drawing planes instead
    /// (`plane_units`), which a draft must erase whole. A merge or erase
    /// that commits must never leave a loop visiting a vertex twice.
    MergeCoplanar {
        obj_sel: usize,
        edge_sel: usize,
        from_features: bool,
    },
    /// Boolean between two distinct visible objects. A typed refusal is an
    /// accepted no-op: `DegenerateContact` for a measure-zero contact,
    /// `EmptyResult` for an empty intersect or a total subtract, and
    /// `NothingRemoved` for a subtract whose operands do not overlap.
    Boolean {
        kind: u8,
        a_sel: usize,
        b_sel: usize,
    },
    /// Translate a visible object.
    Translate {
        obj_sel: usize,
        offset: (f64, f64, f64),
    },
    /// Duplicate a visible object with a translation placement.
    Duplicate {
        obj_sel: usize,
        offset: (f64, f64, f64),
    },
    /// A linked copy of a visible object: the object is wrapped as a linked
    /// set and the copy is a second placement of it.
    DuplicateLinked {
        obj_sel: usize,
        offset: (f64, f64, f64),
    },
    /// A NON-SIMILARITY transform (non-uniform scale) baked into a visible
    /// object — the class of transform `Annotation::reanchor` cannot carry
    /// a `RadialDimension`'s captured circle through (it would turn the
    /// circle into an ellipse) and, for a `LinearDimension` with only one
    /// touched anchor, cannot always re-derive a sane placement either.
    /// Every other transform DocOp here is a similarity (pure translation),
    /// so without this variant that whole detach class was unreachable.
    NonUniformScale {
        obj_sel: usize,
        factors: (f64, f64, f64),
    },
    /// Draw a small triangle on the plane of the `face_sel`-th face of the
    /// `obj_sel`-th visible object — a loose drawing strictly inside the
    /// face, around its interior point — and own it by that object
    /// (`set_sketch_object_owner`), so every later op (transforms,
    /// booleans, slices, push/pull, duplicates, deletes, sessions,
    /// undo/redo) runs against documents holding object-owned sketches.
    /// A face too small or too awkward for a clear interior point is an
    /// accepted no-op.
    OwnSketch {
        obj_sel: usize,
        face_sel: usize,
        turn: f64,
    },
    /// Delete the `node_sel`-th top-level node.
    Delete {
        node_sel: usize,
    },
    /// Group the first `count` top-level nodes.
    Group {
        count: usize,
    },
    /// Ungroup the `group_sel`-th group.
    Ungroup {
        group_sel: usize,
    },
    /// Make a component from the `node_sel`-th top-level node.
    MakeComponent {
        node_sel: usize,
    },
    /// Place another instance of the `comp_sel`-th component, at a pose
    /// composed as rotate-then-scale-then-translate. `scale` is sometimes
    /// uniform and sometimes not (`arb_scale`), so both a similarity pose
    /// (the def-owned-sketch bake in `ExplodeInstance`/`MakeUnique` succeeds)
    /// and a non-uniform one (the sketch bake refuses typed with
    /// `CannotExplodeNonUniformScale`) are fuzz-reachable — previously this
    /// op only ever produced an identity-linear (translation-only) pose.
    PlaceInstance {
        comp_sel: usize,
        axis: (f64, f64, f64),
        angle: f64,
        scale: Vec3,
        offset: (f64, f64, f64),
    },
    /// Explode the `inst_sel`-th instance back into loose objects.
    ExplodeInstance {
        inst_sel: usize,
    },
    /// Push/pull a face of a definition MEMBER via `apply_def_op` — the
    /// shared-geometry edit path, recorded as `DocAction::DefObjectOp`.
    /// Covers happy-path def-op do/undo/redo interleavings (previously
    /// never fuzzed). The REFUSED-replay push-back is NOT reachable from
    /// here: recorded-only sequences unwind in LIFO order, so a recorded
    /// wall is always pristine again by the time its unbuild replays — the
    /// refusal needs an unrecorded (bypass) edit and is pinned by the
    /// `refused_def_object_op_*_pushes_the_action_back` unit specs in
    /// `document.rs` instead.
    DefOp {
        comp_sel: usize,
        member_sel: usize,
        face_sel: usize,
        distance: f64,
    },
    /// Draw a fixed 1×1 rectangle through the `inst_sel`-th instance
    /// (component-edit-parity.md phase K1: `begin_sketch_on_plane_in_instance`
    /// maps the ground plane through the pose⁻¹) and extrude it by `distance`
    /// (`extrude_region_in_instance`, mapped through the pose's uniform
    /// scale) — a shared-geometry BIRTH, unlike `DefOp`'s edit of an
    /// existing member. A degenerate rectangle is filtered by the strategy
    /// below; a non-uniformly-scaled instance's typed `AmbiguousInstanceScale`
    /// refusal is an accepted no-op, not a fuzz failure (same treatment as
    /// every other typed refusal in this harness).
    DrawAndExtrudeInInstance {
        inst_sel: usize,
        distance: f64,
    },
    /// [`DocOp::DrawAndExtrudeInInstance`]'s draw-only half (component-edit-
    /// parity.md phase K1 follow-up): draws the same fixed 1×1 rectangle
    /// through the `inst_sel`-th instance but never extrudes it, so the
    /// def-owned sketch survives LIVE (un-extruded) across later steps —
    /// the only way `MakeUnique`/`ExplodeInstance` ever encounter one to
    /// clone/bake. Wrapped in its own sketch gesture (unlike
    /// `DrawAndExtrudeInInstance`, which folds the whole draw into the
    /// extrude's `CreatedObject` undo step instead), so the draw is its own
    /// undoable action too.
    DrawInInstance {
        inst_sel: usize,
    },
    /// Delete the `member_sel`-th LIVE member of the `comp_sel`-th
    /// component (`delete_def_member`) — refuses typed on a definition's
    /// last live member, an accepted no-op here exactly like every other
    /// typed refusal.
    DeleteDefMember {
        comp_sel: usize,
        member_sel: usize,
    },
    /// Detach the `inst_sel`-th instance onto its own private definition
    /// copy (`make_unique`) — covers the definition-member AND def-owned-
    /// sketch clone paths (component-edit-parity.md phase K1 follow-up) and
    /// their undo/redo.
    MakeUnique {
        inst_sel: usize,
    },
    /// Creates a construction guide line (`Document::add_guide_line`) through
    /// an arbitrary point with an arbitrary non-degenerate direction. Added
    /// so the `decode_guide` verbatim-direction fix (the sibling of the
    /// sketch-plane decode drift bug: a guide's stored direction is already
    /// unit at creation, so renormalizing it again on load flips low
    /// mantissa bits) stays fuzz-reachable through `check_persistence`'s
    /// state_hash save/load round trip.
    AddGuideLine {
        origin: (f64, f64, f64),
        direction: (f64, f64, f64),
    },
    /// Moves the `guide_sel`-th live guide through a rigid pose
    /// (`Document::transform_guide`): the undo/redo of its exact geometry
    /// and the save/load round trip of a rotated direction.
    MoveGuide {
        guide_sel: usize,
        axis: (f64, f64, f64),
        angle: f64,
        offset: (f64, f64, f64),
    },
    /// Copies the `guide_sel`-th live guide `count` times along a
    /// translation (`Document::duplicate_guide_array`, Move's copy and
    /// array on a guide).
    CopyGuide {
        guide_sel: usize,
        offset: (f64, f64, f64),
        count: u32,
    },
    /// Boolean between the `a_sel`-th and `b_sel`-th LIVE members of the
    /// `comp_sel`-th component (`boolean_in_component`, component-edit-
    /// parity.md phase K2) — the shared-geometry combine, replacing both
    /// operands with a new member. `a_sel == b_sel` (same live member) is an
    /// accepted typed no-op (`DegenerateContact`), like the world `Boolean`
    /// op's own `a_sel == b_sel` case; so is a subtract between members
    /// that do not overlap (`NothingRemoved`).
    BooleanInComponent {
        comp_sel: usize,
        kind: u8,
        a_sel: usize,
        b_sel: usize,
    },
    /// Slices the `member_sel`-th LIVE member of the `inst_sel`-th
    /// instance's definition by an arbitrary WORLD-space plane
    /// (`slice_def_member`, phase K2), mapped through the instance's pose⁻¹.
    /// A degenerate direction or a plane that misses the solid is an
    /// accepted typed no-op.
    SliceDefMember {
        inst_sel: usize,
        member_sel: usize,
        origin: (f64, f64, f64),
        direction: (f64, f64, f64),
    },
    /// Bakes a WORLD-space rotate-then-scale-then-translate gesture into the
    /// `member_sel`-th LIVE member of the `inst_sel`-th instance's
    /// definition (`transform_def_member`, phase K2), conjugated through the
    /// instance's pose. `scale` is sometimes uniform and sometimes not
    /// (`arb_scale`) — unlike `extrude_region_in_instance`'s scalar
    /// distance, a full affine conjugation is never ambiguous, so a
    /// non-uniformly-scaled instance is exercised here as an ordinary
    /// success path, not a refusal.
    TransformDefMember {
        inst_sel: usize,
        member_sel: usize,
        axis: (f64, f64, f64),
        angle: f64,
        scale: Vec3,
        offset: (f64, f64, f64),
    },
    /// Sweeps a fixed small profile, drawn fresh through the `inst_sel`-th
    /// instance, around the `face_sel`-th face of the `member_sel`-th LIVE
    /// member of that SAME instance's definition (`follow_me_in_instance`
    /// with a `FaceLoop` path, phase K2) — the shared-geometry Follow Me
    /// birth. `stop_len`, when present, is a WORLD-space partial-sweep
    /// length; under a non-uniformly-scaled instance it is an accepted
    /// typed no-op (`AmbiguousInstanceScale`), exactly like
    /// `DrawAndExtrudeInInstance`'s `distance`.
    FollowMeAroundMemberFaceInInstance {
        inst_sel: usize,
        member_sel: usize,
        face_sel: usize,
        stop_len: Option<f64>,
    },
    /// Places 3D Text: a glyph-like ring+counter (a rect with a smaller
    /// rect hole, mirroring `document.rs`'s `glyph_o_sketch` unit-test
    /// fixture) at a fuzzed ground position, selecting only the ring as
    /// fill. `place_text` always retires (hides) its scratch sketch before
    /// returning — the unselected counter/hole region included — so there
    /// is no live residue of this op for any later op to reach or disturb.
    PlaceText {
        x: f64,
        y: f64,
        depth: f64,
    },
    /// Uniformly rescale the whole document about the world origin
    /// (tool-parity §3 — the Tape Measure "resize the model" flow).
    Rescale {
        factor: f64,
    },
    /// Move the document's drawing axes (tool-parity §4) to an arbitrary
    /// orthonormal right-handed frame: `axis`/`angle` describe a rotation
    /// applied to the identity basis, which is always orthonormal and
    /// right-handed by construction (a pure rotation preserves both), so
    /// this can never refuse.
    SetAxes {
        origin: (f64, f64, f64),
        axis: (f64, f64, f64),
        angle: f64,
    },
    /// Re-express the `comp_sel`-th component in a rotated, moved frame
    /// (`rebase_definition`): world geometry stays put, every placement is
    /// compensated. A rotated identity basis is always valid, so only an
    /// open session or a stale definition refuses.
    RebaseDefinition {
        comp_sel: usize,
        origin: (f64, f64, f64),
        axis: (f64, f64, f64),
        angle: f64,
    },
    /// Mirror the `inst_sel`-th instance across its local axis `axis % 3`
    /// (`flip_instance`).
    FlipInstance {
        inst_sel: usize,
        axis: u8,
    },
    /// Add a linear dimension anchored on two of the `obj_sel`-th visible
    /// object's own vertices (exercises re-anchoring: later transforms/
    /// deletes on `obj_sel` touch this annotation).
    AddLinearDimension {
        obj_sel: usize,
        v_a: usize,
        v_b: usize,
        offset: (f64, f64, f64),
    },
    /// Add a radial dimension anchored on one of the `obj_sel`-th visible
    /// object's own vertices, with a synthetic captured circle (the kernel
    /// never cross-validates the curve against the object's actual
    /// geometry — see `Document::add_radial_dimension`) — exercises the
    /// non-similarity-transform detach path this DocOp family previously
    /// left unreachable.
    AddRadialDimension {
        obj_sel: usize,
        v_sel: usize,
        radius: f64,
        leader: (f64, f64, f64),
    },
    /// Add a free-floating leader text (no anchored node — never
    /// re-anchored, never detaches; the control case beside the anchored
    /// variants above).
    AddLeaderText {
        p: (f64, f64, f64),
    },
    /// Delete the `ann_sel`-th live annotation.
    DeleteAnnotation {
        ann_sel: usize,
    },
    /// Replace the `ann_sel`-th live annotation's text/override in place,
    /// keeping its kind and anchors — `update_annotation`'s "same variant"
    /// contract. A text-only edit like this one leaves `detached` exactly as
    /// it was (only a change to an anchor/geometry field clears it — see
    /// `Document::update_annotation`'s doc comment).
    UpdateAnnotationText {
        ann_sel: usize,
        text: String,
    },
    /// Paint the `face_sel`-th face of the `obj_sel`-th visible object.
    /// `mat_sel` indexes the fixed 3-material palette PLUS the unpainted
    /// sentinel (`mat_sel % 4`, last slot = `None`).
    PaintFace {
        obj_sel: usize,
        face_sel: usize,
        mat_sel: usize,
    },
    /// Set the `obj_sel`-th visible object's base material (same `mat_sel`
    /// convention as `PaintFace`).
    SetObjectMaterial {
        obj_sel: usize,
        mat_sel: usize,
    },
    /// `replace_material` — Shift-click "replace everywhere" (`object_scope:
    /// false` = whole document) or Ctrl/Cmd+Shift (`true` = confined to the
    /// `obj_sel`-th visible object). `from_sel`/`to_sel` use the same
    /// `mat_sel` convention (3 materials + the unpainted sentinel), so `from`
    /// is exercised as both a real material and the sentinel.
    ReplaceMaterial {
        object_scope: bool,
        obj_sel: usize,
        from_sel: usize,
        to_sel: usize,
    },
    /// `set_face_uv_frame` — Position Texture's kernel commit (paint-tool
    /// design §3). `frame_sel` indexes a small fixed set of sample frames
    /// PLUS `None` (see `pick_uv_frame`), same modulo convention as
    /// `pick_material`.
    SetFaceUvFrame {
        obj_sel: usize,
        face_sel: usize,
        frame_sel: usize,
    },
    /// A face or edge attribute dictionary write (docs/agents/HEW_API.md
    /// §8): on the `elem_sel`-th face (`edge: false`) or edge of the
    /// `obj_sel`-th visible object, `op_sel` picks a key write (small int,
    /// or a nested value), a key delete, or a whole-namespace delete
    /// (the deletes refuse typed when there is nothing to delete). Later
    /// structural ops carry the dictionaries by the per-site rules, and
    /// every committed state must reopen with them.
    ElementAttr {
        obj_sel: usize,
        elem_sel: usize,
        edge: bool,
        op_sel: usize,
    },
    /// `set_material_name` on the `mat_sel`-th seeded material (same
    /// `mat_sel` convention; the sentinel slot is a no-op). A deleted
    /// material refuses typed — exercised because `DeleteMaterial` below
    /// can precede this.
    RenameMaterial {
        mat_sel: usize,
        name: String,
    },
    /// `delete_material` on the `mat_sel`-th seeded material: clears every
    /// reference (tombstoned rows included) and tombstones the handle. The
    /// global `Undo` brings it back, so later material ops see both live
    /// and deleted handles.
    DeleteMaterial {
        mat_sel: usize,
    },
    /// `delete_definition` on the `comp_sel`-th live definition — every
    /// world instance plus the definition as one entry, or a typed refusal
    /// when a live definition nests it.
    DeleteDefinition {
        comp_sel: usize,
    },
    /// `purge_unused` — one labeled entry (or nothing when idle).
    PurgeUnused,
    /// `reparent_nodes`: move the `node_sel`-th top-level node into the
    /// `group_sel`-th live group (or out to the top level when `to_top`),
    /// exercising the tree bookkeeping and its verbatim member-order
    /// restoration against every other tree op here.
    Reparent {
        node_sel: usize,
        group_sel: usize,
        to_top: bool,
    },
    /// Draw an axis-aligned lattice rectangle (0.5 m grid) on the ground
    /// sketch (the first world ground sketch, or a fresh one) inside one
    /// sketch gesture, so rectangles overlap and abut freely and the sticky
    /// rules arrange them.
    DrawRectOnGround {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    },
    /// Draw a faceted circle (a `begin_curve_with` chain carrying its exact
    /// circle) on the ground sketch inside one sketch gesture.
    DrawCircleOnGround {
        cx: i32,
        cy: i32,
        r: i32,
        segments: usize,
    },
    /// Translate one island of a world sketch by a lattice offset
    /// (`Document::transform_sketch_island`); small offsets on a coarse
    /// lattice, so it often lands on other geometry.
    MoveIsland {
        sk_sel: usize,
        isl_sel: usize,
        dx: i32,
        dy: i32,
    },
    /// Translate two islands of one world sketch together.
    MoveIslands {
        sk_sel: usize,
        a_sel: usize,
        b_sel: usize,
        dx: i32,
        dy: i32,
    },
    /// Erase one edge of a world sketch inside a sketch gesture.
    RemoveSketchEdge {
        sk_sel: usize,
        edge_sel: usize,
    },
    /// Drag a world-sketch vertex through `Document::move_sketch_vertex`;
    /// `mode` biases the target: 0 = onto another vertex, 1 = onto an
    /// edge's midpoint, 2 = a lattice offset from where it is.
    MoveSketchVertex {
        sk_sel: usize,
        v_sel: usize,
        mode: u8,
        target_sel: usize,
        dx: i32,
        dy: i32,
    },
    /// Offset one region of a world sketch inside a sketch gesture.
    OffsetRegion {
        sk_sel: usize,
        reg_sel: usize,
        dist_sel: usize,
    },
    /// Re-facet one analytic circle chain of a world sketch inside a sketch
    /// gesture.
    RefacetCurve {
        sk_sel: usize,
        curve_sel: usize,
        segments: usize,
    },
    /// Draft on the plane of the `face_sel`-th face of the `obj_sel`-th
    /// visible object (or the ground, when `ground` or nothing is visible)
    /// through the drafting engine (`Document::draft`, unified drafting §4),
    /// over every visible object with a face on that plane. The lattice
    /// (0.5 m) is anchored at the face's first outer vertex along its first
    /// edge, so shapes land on, across, and past the face's boundary,
    /// across imprints, and onto earlier drawing. `shape`: 0 rectangle,
    /// 1 faceted circle, 2 loose segment, 3 erase the `sel`-th non-boundary
    /// edge of the working sketch, 4 move the `sel`-th drawing unit of the
    /// plane (`plane_units`) by `a` lattice steps, 5 erase that unit whole.
    /// Only `DraftBoundaryEdgeTouched` is an
    /// accepted refusal (and then the document must be byte-identical);
    /// every other error fails the case.
    Draft {
        obj_sel: usize,
        face_sel: usize,
        ground: bool,
        shape: u8,
        a: (i32, i32),
        b: (i32, i32),
        sel: usize,
        segments: usize,
    },
    /// Draw a NEW detached shape (detached shapes §3.8) through
    /// `Document::draft_detached_new` on the plane [`DocOp::Draft`] would
    /// pick — the `face_sel`-th face of the `obj_sel`-th visible object, or
    /// the ground — on the same lattice: a rectangle `a` + `b`, or a circle
    /// about `a` of radius `b.0` steps. A refusal fails the case.
    DrawDetached {
        obj_sel: usize,
        face_sel: usize,
        ground: bool,
        circle: bool,
        a: (i32, i32),
        b: (i32, i32),
        segments: usize,
    },
    /// Edit the `sk_sel`-th detached sketch in its own context
    /// (`Document::draft_detached`): `mode` 0 moves its `sel`-th unit by
    /// `d` lattice steps in its plane, 1 erases its `sel`-th edge, 2
    /// continues drawing — a segment from its `sel`-th vertex by `d`.
    /// A refusal fails the case.
    EditDetached {
        sk_sel: usize,
        mode: u8,
        sel: usize,
        d: (i32, i32),
    },
    /// Extrude the `reg_sel`-th region of the `sk_sel`-th detached sketch
    /// (as listed by `Document::detached_sketch_units`) by `distance`.
    ExtrudeDetached {
        sk_sel: usize,
        reg_sel: usize,
        distance: f64,
    },
    Undo,
    Redo,
    /// Open a compound (a held bracket around a forwarded plugin command).
    BeginCompound,
    /// End the innermost open compound, if any.
    EndCompound,
    /// Abort the innermost open compound, if any.
    AbortCompound,
    /// A one-attribute transaction committed with coalesce key `key % 3`
    /// (2 is no key), under origin `origin % 2`.
    CoalescedAttr {
        key: u8,
        origin: u8,
        value: i64,
    },
}

fn arb_offset() -> impl Strategy<Value = (f64, f64, f64)> {
    (-8.0..8.0f64, -8.0..8.0f64, -4.0..4.0f64)
}

/// A non-degenerate rotation axis for `Transform::rotation` — it normalizes
/// internally, so only near-zero magnitude (the `DegenerateAxis` case) needs
/// filtering out.
fn arb_axis() -> impl Strategy<Value = (f64, f64, f64)> {
    (-1.0..1.0f64, -1.0..1.0f64, -1.0..1.0f64).prop_filter("degenerate axis", |&(x, y, z)| {
        x * x + y * y + z * z >= 0.01
    })
}

/// An instance placement scale: sometimes uniform (a similarity — the
/// def-owned-sketch bake in `ExplodeInstance`/`MakeUnique` succeeds),
/// sometimes per-axis (not a similarity — the bake refuses typed with
/// `CannotExplodeNonUniformScale`), so both paths are fuzz-reachable.
fn arb_scale() -> impl Strategy<Value = Vec3> {
    let comp = || 0.3..3.0f64;
    // A signed component: a negative one mirrors the pose (poses may
    // reflect; sessions and explodes bake the mirror).
    let signed = || prop_oneof![4 => comp(), 1 => comp().prop_map(|s| -s)];
    prop_oneof![
        2 => comp().prop_map(|s| Vec3::new(s, s, s)),
        3 => (comp(), comp(), comp()).prop_map(|(x, y, z)| Vec3::new(x, y, z)),
        1 => signed().prop_map(|s| Vec3::new(s, s.abs(), s.abs())),
    ]
}

/// Whether free drawing (a live sketch no object owns) lies on `plane`,
/// facing either way: a draft on the plane arranges with it, so ops that pin
/// exact counts or edges hold them only without it.
fn free_drawing_on(doc: &Document, plane: Plane) -> bool {
    let anchor = Point3::new(0.0, 0.0, 0.0)
        + plane.normal() * (-plane.signed_distance(Point3::new(0.0, 0.0, 0.0)));
    doc.sketch_ids().into_iter().any(|s| {
        doc.sketch_owner_object(s).is_none()
            && doc.sketch(s).is_some_and(|sk| {
                let q = sk.plane();
                !sk.edges().is_empty()
                    && q.normal().dot(plane.normal()).abs() > 1.0 - 1e-9
                    && q.signed_distance(anchor).abs() <= 1e-6
            })
    })
}

/// A hole-free rectangular face of the `obj_sel`-th visible object (the
/// `face_sel`-th face), given two side-by-side imprints at x ∈ [0.15, 0.4]
/// and [0.6, 0.85], y ∈ [0.25, 0.75] of its frame — returned as
/// `(origin corner, full u extent, full v extent)`, counter-clockwise seen
/// from the face. `None` (nothing changed) when the face is not a hole-free
/// rectangle.
fn two_imprints(
    doc: &mut Document,
    obj_sel: usize,
    face_sel: usize,
    step: usize,
) -> Option<(ObjectId, (Point3, Vec3, Vec3))> {
    let oid = nth(&doc.visible_object_ids(), obj_sel)?;
    let obj = doc.object(oid).expect("visible id resolves");
    let face = obj.faces().keys().nth(face_sel % obj.faces().len())?;
    if !obj.faces()[face].inner_loops.is_empty() {
        return None;
    }
    let ring: Vec<Point3> = obj.loop_positions(obj.faces()[face].outer_loop).collect();
    let rectangle = ring.len() == 4
        && (0..4).all(|i| {
            let e0 = ring[(i + 1) % 4] - ring[i];
            let e1 = ring[(i + 2) % 4] - ring[(i + 1) % 4];
            e0.length() >= 1e-3 && e0.dot(e1).abs() <= 1e-9 * e0.length() * e1.length()
        });
    if !rectangle {
        return None;
    }
    let frame = (ring[0], ring[1] - ring[0], ring[3] - ring[0]);
    let at = |x: f64, y: f64| frame.0 + frame.1 * x + frame.2 * y;
    for (x0, x1) in [(0.15, 0.4), (0.6, 0.85)] {
        let obj = doc.object(oid).expect("visible id resolves");
        let holder = obj
            .faces()
            .keys()
            .find(|&f| obj.face_contains_point(f, at(0.05, 0.05)))
            .expect("the rectangle again");
        let imprinted = doc.apply_object_op(
            oid,
            KernelOp::SplitFaceInner {
                face: holder,
                loop_path: vec![at(x0, 0.25), at(x1, 0.25), at(x1, 0.75), at(x0, 0.75)],
                restore: None,
                curves: Vec::new(),
                edge_attrs: Vec::new(),
            },
        );
        assert!(
            imprinted.is_ok(),
            "step {step}: an imprint on a fresh rectangle was refused: {:?}",
            imprinted.err()
        );
    }
    Some((oid, frame))
}

/// The world-sketch edit ops (ground drawing, island/vertex moves, erase,
/// offset, re-facet), weighted among themselves.
fn arb_sketch_edit() -> impl Strategy<Value = DocOp> {
    prop_oneof![
        3 => (0..16i32, 0..16i32, 1..8i32, 1..8i32)
            .prop_map(|(x, y, w, h)| DocOp::DrawRectOnGround { x, y, w, h }),
        1 => (0..16i32, 0..16i32, 1..8i32, 24..40usize)
            .prop_map(|(cx, cy, r, segments)| DocOp::DrawCircleOnGround { cx, cy, r, segments }),
        3 => (any::<usize>(), any::<usize>(), -6..=6i32, -6..=6i32)
            .prop_map(|(sk_sel, isl_sel, dx, dy)| DocOp::MoveIsland { sk_sel, isl_sel, dx, dy }),
        2 => (any::<usize>(), any::<usize>(), any::<usize>(), -6..=6i32, -6..=6i32)
            .prop_map(|(sk_sel, a_sel, b_sel, dx, dy)| DocOp::MoveIslands {
                sk_sel, a_sel, b_sel, dx, dy,
            }),
        1 => (any::<usize>(), any::<usize>())
            .prop_map(|(sk_sel, edge_sel)| DocOp::RemoveSketchEdge { sk_sel, edge_sel }),
        3 => (any::<usize>(), any::<usize>(), 0..3u8, any::<usize>(), -6..=6i32, -6..=6i32)
            .prop_map(|(sk_sel, v_sel, mode, target_sel, dx, dy)| DocOp::MoveSketchVertex {
                sk_sel, v_sel, mode, target_sel, dx, dy,
            }),
        1 => (any::<usize>(), any::<usize>(), any::<usize>())
            .prop_map(|(sk_sel, reg_sel, dist_sel)| DocOp::OffsetRegion { sk_sel, reg_sel, dist_sel }),
        1 => (any::<usize>(), any::<usize>(), 24..64usize)
            .prop_map(|(sk_sel, curve_sel, segments)| DocOp::RefacetCurve {
                sk_sel, curve_sel, segments,
            }),
        6 => (
            any::<usize>(),
            any::<usize>(),
            proptest::bool::weighted(0.15),
            0..6u8,
            (-2..14i32, -2..14i32),
            (1..9i32, 1..9i32),
            any::<usize>(),
            24..40usize,
        )
            .prop_map(|(obj_sel, face_sel, ground, shape, a, b, sel, segments)| {
                DocOp::Draft { obj_sel, face_sel, ground, shape, a, b, sel, segments }
            }),
        2 => (
            any::<usize>(),
            any::<usize>(),
            proptest::bool::weighted(0.25),
            any::<bool>(),
            (-2..14i32, -2..14i32),
            (1..9i32, 1..9i32),
            24..40usize,
        )
            .prop_map(|(obj_sel, face_sel, ground, circle, a, b, segments)| {
                DocOp::DrawDetached { obj_sel, face_sel, ground, circle, a, b, segments }
            }),
        2 => (any::<usize>(), 0..3u8, any::<usize>(), (-6..=6i32, -6..=6i32))
            .prop_map(|(sk_sel, mode, sel, d)| DocOp::EditDetached { sk_sel, mode, sel, d }),
        1 => (
            any::<usize>(),
            any::<usize>(),
            (-3.0..3.0f64).prop_filter("degenerate distance", |d| d.abs() >= 0.05),
        )
            .prop_map(|(sk_sel, reg_sel, distance)| DocOp::ExtrudeDetached {
                sk_sel, reg_sel, distance,
            }),
    ]
}

/// World-sketch edits with undo/redo interleaved, for the sketch-focused
/// property.
fn arb_world_sketch_op() -> impl Strategy<Value = DocOp> {
    prop_oneof![
        12 => arb_sketch_edit(),
        3 => Just(DocOp::Undo),
        2 => Just(DocOp::Redo),
    ]
}

fn arb_doc_op() -> impl Strategy<Value = DocOp> {
    let distance = || (-4.0..4.0f64).prop_filter("degenerate distance", |d| d.abs() >= 0.05);
    prop_oneof![
        3 => (any::<usize>(), any::<usize>(), distance()).prop_map(|(obj_sel, face_sel, distance)| {
            DocOp::PushPull { obj_sel, face_sel, distance }
        }),
        2 => (any::<usize>(), any::<usize>(), any::<usize>(), any::<usize>(), 0.25..0.75f64, 0.25..0.75f64)
            .prop_map(|(obj_sel, face_sel, edge_a, edge_b, ta, tb)| DocOp::SplitFace {
                obj_sel, face_sel, edge_a, edge_b, ta, tb,
            }),
        2 => (any::<usize>(), any::<usize>(), 0.3..0.7f64, proptest::bool::ANY).prop_map(
            |(obj_sel, face_sel, shrink, staple)| {
                DocOp::SplitFaceInner { obj_sel, face_sel, shrink, staple }
            }
        ),
        2 => (
            any::<usize>(),
            any::<usize>(),
            [any::<usize>(), any::<usize>(), any::<usize>()],
            [0.2..0.8f64, 0.2..0.8f64, 0.2..0.8f64],
            proptest::bool::ANY,
        )
            .prop_map(|(obj_sel, face_sel, edges, ts, pull_in)| DocOp::ImprintTouchingLoop {
                obj_sel, face_sel, edges, ts, pull_in,
            }),
        2 => (
            any::<usize>(),
            any::<usize>(),
            any::<usize>(),
            0.2..0.8f64,
            0.2..1.0f64,
            proptest::bool::ANY,
        )
            .prop_map(|(obj_sel, face_sel, side, t, frac, twin)| DocOp::TangentCircle {
                obj_sel, face_sel, side, t, frac, twin,
            }),
        2 => (
            any::<usize>(),
            any::<usize>(),
            any::<usize>(),
            (0.0..1.0f64, 0.0..1.0f64),
            0.1..1.0f64,
            proptest::bool::ANY,
        )
            .prop_map(|(obj_sel, face_sel, side, ts, frac, merge_back)| DocOp::HoleChord {
                obj_sel, face_sel, side, ts, frac, merge_back,
            }),
        2 => (
            any::<usize>(),
            any::<usize>(),
            any::<usize>(),
            (0.0..1.0f64, 0.0..1.0f64),
            0.1..1.0f64,
            proptest::bool::ANY,
            proptest::bool::ANY,
        )
            .prop_map(|(obj_sel, face_sel, side, ts, frac, circle, from_inside)| {
                DocOp::CrossingLoop { obj_sel, face_sel, side, ts, frac, circle, from_inside }
            }),
        2 => (
            any::<usize>(),
            any::<usize>(),
            (0.0..1.0f64, 0.0..1.0f64),
            0.0..1.0f64,
            proptest::bool::ANY,
        )
            .prop_map(|(obj_sel, face_sel, ts, frac, circle)| DocOp::BridgeLoop {
                obj_sel, face_sel, ts, frac, circle,
            }),
        2 => (any::<usize>(), any::<usize>(), (0.0..1.0f64, 0.0..1.0f64), proptest::bool::ANY)
            .prop_map(|(obj_sel, face_sel, ts, top)| DocOp::MultiRunMerge {
                obj_sel, face_sel, ts, top,
            }),
        2 => (any::<usize>(), any::<usize>(), proptest::bool::ANY).prop_map(
            |(obj_sel, edge_sel, from_features)| DocOp::MergeCoplanar {
                obj_sel, edge_sel, from_features,
            }
        ),
        3 => (0u8..3, any::<usize>(), any::<usize>()).prop_map(|(kind, a_sel, b_sel)| {
            DocOp::Boolean { kind, a_sel, b_sel }
        }),
        2 => (any::<usize>(), arb_offset()).prop_map(|(obj_sel, offset)| {
            DocOp::Translate { obj_sel, offset }
        }),
        2 => (any::<usize>(), arb_offset()).prop_map(|(obj_sel, offset)| {
            DocOp::Duplicate { obj_sel, offset }
        }),
        1 => (any::<usize>(), arb_offset()).prop_map(|(obj_sel, offset)| {
            DocOp::DuplicateLinked { obj_sel, offset }
        }),
        2 => (any::<usize>(), (0.2..3.0f64, 0.2..3.0f64, 0.2..3.0f64)).prop_map(
            |(obj_sel, factors)| DocOp::NonUniformScale { obj_sel, factors }
        ),
        // A reflection baked into a visible object (negative x factor).
        1 => (any::<usize>(), (0.2..3.0f64, 0.2..3.0f64, 0.2..3.0f64)).prop_map(
            |(obj_sel, f)| DocOp::NonUniformScale { obj_sel, factors: (-f.0, f.1, f.2) }
        ),
        4 => (any::<usize>(), any::<usize>(), 0.0..std::f64::consts::TAU).prop_map(
            |(obj_sel, face_sel, turn)| DocOp::OwnSketch { obj_sel, face_sel, turn }
        ),
        1 => any::<usize>().prop_map(|node_sel| DocOp::Delete { node_sel }),
        1 => (2usize..4).prop_map(|count| DocOp::Group { count }),
        1 => any::<usize>().prop_map(|group_sel| DocOp::Ungroup { group_sel }),
        1 => any::<usize>().prop_map(|node_sel| DocOp::MakeComponent { node_sel }),
        1 => (any::<usize>(), arb_axis(), -3.0..3.0f64, arb_scale(), arb_offset()).prop_map(
            |(comp_sel, axis, angle, scale, offset)| DocOp::PlaceInstance {
                comp_sel, axis, angle, scale, offset,
            }
        ),
        1 => any::<usize>().prop_map(|inst_sel| DocOp::ExplodeInstance { inst_sel }),
        1 => (any::<usize>(), any::<usize>(), any::<usize>(), distance()).prop_map(
            |(comp_sel, member_sel, face_sel, distance)| DocOp::DefOp {
                comp_sel, member_sel, face_sel, distance,
            }
        ),
        1 => (any::<usize>(), distance()).prop_map(|(inst_sel, distance)| {
            DocOp::DrawAndExtrudeInInstance { inst_sel, distance }
        }),
        1 => any::<usize>().prop_map(|inst_sel| DocOp::DrawInInstance { inst_sel }),
        1 => (any::<usize>(), any::<usize>()).prop_map(|(comp_sel, member_sel)| {
            DocOp::DeleteDefMember { comp_sel, member_sel }
        }),
        1 => any::<usize>().prop_map(|inst_sel| DocOp::MakeUnique { inst_sel }),
        1 => (arb_offset(), arb_axis()).prop_map(|(origin, direction)| {
            DocOp::AddGuideLine { origin, direction }
        }),
        1 => (any::<usize>(), arb_axis(), -3.0..3.0f64, arb_offset()).prop_map(
            |(guide_sel, axis, angle, offset)| DocOp::MoveGuide {
                guide_sel, axis, angle, offset,
            }
        ),
        1 => (any::<usize>(), arb_offset(), 1u32..4).prop_map(|(guide_sel, offset, count)| {
            DocOp::CopyGuide { guide_sel, offset, count }
        }),
        1 => (0u8..3, any::<usize>(), any::<usize>(), any::<usize>()).prop_map(
            |(kind, comp_sel, a_sel, b_sel)| DocOp::BooleanInComponent {
                comp_sel, kind, a_sel, b_sel,
            }
        ),
        1 => (any::<usize>(), any::<usize>(), arb_offset(), arb_axis()).prop_map(
            |(inst_sel, member_sel, origin, direction)| DocOp::SliceDefMember {
                inst_sel, member_sel, origin, direction,
            }
        ),
        1 => (any::<usize>(), any::<usize>(), arb_axis(), -3.0..3.0f64, arb_scale(), arb_offset())
            .prop_map(|(inst_sel, member_sel, axis, angle, scale, offset)| {
                DocOp::TransformDefMember { inst_sel, member_sel, axis, angle, scale, offset }
            }),
        1 => (any::<usize>(), any::<usize>(), any::<usize>(), proptest::option::of(distance()))
            .prop_map(|(inst_sel, member_sel, face_sel, stop_len)| {
                DocOp::FollowMeAroundMemberFaceInInstance { inst_sel, member_sel, face_sel, stop_len }
            }),
        2 => (-20.0..20.0f64, -20.0..20.0f64, distance()).prop_map(|(x, y, depth)| {
            DocOp::PlaceText { x, y, depth: depth.abs() }
        }),
        1 => (0.2..5.0f64).prop_map(|factor| DocOp::Rescale { factor }),
        1 => (any::<usize>(), arb_offset(), arb_axis(), 0.0..std::f64::consts::TAU).prop_map(
            |(comp_sel, origin, axis, angle)| DocOp::RebaseDefinition { comp_sel, origin, axis, angle }
        ),
        1 => (any::<usize>(), 0u8..3).prop_map(|(inst_sel, axis)| DocOp::FlipInstance { inst_sel, axis }),
        1 => (arb_offset(), arb_offset(), 0.0..std::f64::consts::TAU).prop_map(
            |(origin, axis, angle)| DocOp::SetAxes { origin, axis, angle }
        ),
        2 => (any::<usize>(), any::<usize>(), any::<usize>(), arb_offset()).prop_map(
            |(obj_sel, v_a, v_b, offset)| DocOp::AddLinearDimension { obj_sel, v_a, v_b, offset }
        ),
        2 => (any::<usize>(), any::<usize>(), 0.1..5.0f64, arb_offset()).prop_map(
            |(obj_sel, v_sel, radius, leader)| DocOp::AddRadialDimension {
                obj_sel, v_sel, radius, leader,
            }
        ),
        1 => arb_offset().prop_map(|p| DocOp::AddLeaderText { p }),
        1 => any::<usize>().prop_map(|ann_sel| DocOp::DeleteAnnotation { ann_sel }),
        1 => (any::<usize>(), "[a-z]{0,6}").prop_map(|(ann_sel, text)| {
            DocOp::UpdateAnnotationText { ann_sel, text }
        }),
        2 => (any::<usize>(), any::<usize>(), any::<usize>()).prop_map(
            |(obj_sel, face_sel, mat_sel)| DocOp::PaintFace { obj_sel, face_sel, mat_sel }
        ),
        1 => (any::<usize>(), any::<usize>()).prop_map(|(obj_sel, mat_sel)| {
            DocOp::SetObjectMaterial { obj_sel, mat_sel }
        }),
        2 => (proptest::bool::ANY, any::<usize>(), any::<usize>(), any::<usize>()).prop_map(
            |(object_scope, obj_sel, from_sel, to_sel)| DocOp::ReplaceMaterial {
                object_scope, obj_sel, from_sel, to_sel,
            }
        ),
        1 => (any::<usize>(), any::<usize>(), any::<usize>()).prop_map(
            |(obj_sel, face_sel, frame_sel)| DocOp::SetFaceUvFrame { obj_sel, face_sel, frame_sel }
        ),
        2 => (any::<usize>(), any::<usize>(), proptest::bool::ANY, any::<usize>()).prop_map(
            |(obj_sel, elem_sel, edge, op_sel)| DocOp::ElementAttr { obj_sel, elem_sel, edge, op_sel }
        ),
        1 => (any::<usize>(), "[A-Za-z]{1,8}").prop_map(|(mat_sel, name)| {
            DocOp::RenameMaterial { mat_sel, name }
        }),
        1 => any::<usize>().prop_map(|mat_sel| DocOp::DeleteMaterial { mat_sel }),
        1 => any::<usize>().prop_map(|comp_sel| DocOp::DeleteDefinition { comp_sel }),
        1 => Just(DocOp::PurgeUnused),
        1 => (any::<usize>(), any::<usize>(), proptest::bool::ANY).prop_map(
            |(node_sel, group_sel, to_top)| DocOp::Reparent { node_sel, group_sel, to_top }
        ),
        14 => arb_sketch_edit(),
        2 => Just(DocOp::Undo),
        1 => Just(DocOp::Redo),
        1 => Just(DocOp::BeginCompound),
        1 => Just(DocOp::EndCompound),
        1 => Just(DocOp::AbortCompound),
        2 => (0..3u8, 0..2u8, -50..50i64)
            .prop_map(|(key, origin, value)| DocOp::CoalescedAttr { key, origin, value }),
    ]
}

/// The compound and coalescing ops, applied in `run_doc_case` (they need
/// the stack of open compounds). `Ok(true)` when `op` was one of them.
fn apply_compound_op(
    doc: &mut Document,
    step: usize,
    op: &DocOp,
    open: &mut Vec<(kernel::CompoundToken, usize, bool)>,
) -> Result<bool, TestCaseError> {
    let meta = |label: &str, origin: u8, coalesce: Option<String>| kernel::CompoundMeta {
        label: label.to_string(),
        origin: kernel::HistoryOrigin::Connection(format!("plugin:fuzz.p{origin}")),
        coalesce,
    };
    match op {
        DocOp::BeginCompound => {
            let depth = doc.undo_depth();
            open.push((doc.begin_compound(meta("compound", 0, None)), depth, false));
        }
        DocOp::EndCompound => {
            if let Some((token, depth, wrote)) = open.pop() {
                match doc.end_compound(token) {
                    // Everything the compound held is one entry on top of
                    // the history it began on: nothing leaked below it
                    // (a coalesced commit, say).
                    Ok(outcome) => {
                        prop_assert_eq!(
                            doc.undo_depth(),
                            depth + usize::from(outcome.committed),
                            "step {}: a compound's work is one entry above its start",
                            step
                        );
                        prop_assert!(
                            !wrote || outcome.committed,
                            "step {}: a coalesced commit inside escaped the compound",
                            step
                        );
                        if outcome.committed
                            && let Some(outer) = open.last_mut()
                        {
                            outer.2 = true;
                        }
                    }
                    // An undo inside disturbs the compound: refused and
                    // restored, which is the contract.
                    Err(kernel::DocumentError::TransactionHistoryDisturbed) => {}
                    Err(e) => {
                        return Err(TestCaseError::fail(format!(
                            "step {step}: end_compound failed: {e}"
                        )));
                    }
                }
            }
        }
        DocOp::AbortCompound => {
            if let Some((token, _, _)) = open.pop() {
                let before = doc.document_epoch();
                doc.abort_compound(token).map_err(|e| {
                    TestCaseError::fail(format!("step {step}: abort_compound failed: {e}"))
                })?;
                prop_assert!(doc.document_epoch() > before, "abort moves the epoch");
            }
        }
        DocOp::CoalescedAttr { key, origin, value } => {
            let txn = doc.begin_transaction();
            doc.attr_set(
                kernel::AttrTarget::Document,
                "org.fuzz",
                "k",
                kernel::AttrValue::Int(*value),
            )
            .map_err(|e| TestCaseError::fail(format!("step {step}: attr_set failed: {e}")))?;
            let coalesce = (*key < 2).then(|| format!("k{key}"));
            doc.commit_transaction(txn, meta("edit", *origin, coalesce))
                .map_err(|e| TestCaseError::fail(format!("step {step}: commit failed: {e}")))?;
            if let Some(top) = open.last_mut() {
                top.2 = true;
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Fixed 3-material palette, seeded once. `add_material` is deliberately
/// not on the undo stack, and `delete_material` tombstones rather than
/// removes, so these handles stay VALID for the whole test — a deleted one
/// simply refuses typed (`UnknownMaterial`) until an undo revives it,
/// which is exactly the stale-handle path worth fuzzing. `mat_sel % 4`
/// resolves into it: `3` (the last slot) is the unpainted sentinel `None`.
fn seed_materials(doc: &mut Document) -> Vec<MaterialId> {
    vec![
        doc.add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30))),
        doc.add_material(Material::solid("Green", Rgba8::rgb(30, 220, 30))),
        doc.add_material(Material::solid("Blue", Rgba8::rgb(30, 30, 220))),
    ]
}

/// Resolves a fuzz `mat_sel` against the fixed palette: `sel % (len + 1)`,
/// the last index being the unpainted sentinel `None`.
fn pick_material(materials: &[MaterialId], sel: usize) -> Option<MaterialId> {
    let idx = sel % (materials.len() + 1);
    materials.get(idx).copied()
}

/// Resolves a fuzz `frame_sel` against a small fixed set of sample
/// [`UvFrame`]s, `sel % 6` — three well-formed frames, two deliberately
/// DEGENERATE ones (a zero-length gradient, and parallel `s`/`t`) that
/// `set_face_uv_frame` must refuse ([`DocumentError::DegenerateUvFrame`])
/// rather than silently store — `apply_doc_op` below discards the
/// `Result` via `let _ =`, same posture as `PaintFace`, so the refusal
/// itself isn't asserted here; `check_doc`'s invariant walk after every
/// step is what proves the refusal left the document uncorrupted — and
/// the last index `None` (the planar-default sentinel), same modulo
/// convention as `pick_material`.
fn pick_uv_frame(sel: usize) -> Option<UvFrame> {
    const FRAMES: [UvFrame; 5] = [
        UvFrame {
            s: Vec3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            t: Vec3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            u0: 0.0,
            v0: 0.0,
        },
        UvFrame {
            s: Vec3 {
                x: 0.0,
                y: 2.0,
                z: 0.0,
            },
            t: Vec3 {
                x: -2.0,
                y: 0.0,
                z: 0.0,
            },
            u0: 0.25,
            v0: -0.5,
        },
        UvFrame {
            s: Vec3 {
                x: 0.5,
                y: 0.5,
                z: 0.0,
            },
            t: Vec3 {
                x: -0.5,
                y: 0.5,
                z: 0.7,
            },
            u0: 1.0,
            v0: 1.0,
        },
        // Degenerate: zero-length s gradient.
        UvFrame {
            s: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            t: Vec3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            u0: 0.0,
            v0: 0.0,
        },
        // Degenerate: s and t parallel (no genuine 2D gradient).
        UvFrame {
            s: Vec3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            t: Vec3 {
                x: 2.0,
                y: 0.0,
                z: 0.0,
            },
            u0: 0.0,
            v0: 0.0,
        },
    ];
    let idx = sel % (FRAMES.len() + 1);
    FRAMES.get(idx).copied()
}

/// One object's material state in a [`MaterialsSnapshot`]: its base, plus
/// every face's own (sorted by handle).
type ObjectMaterialsSnapshot = (
    ObjectId,
    Option<MaterialId>,
    Vec<(FaceId, Option<MaterialId>)>,
);

/// Every visible object's material state: base material, plus every face's
/// own — for the LOCAL round-trip check bracketing a single
/// `replace_material` call (see `DocOp::ReplaceMaterial` in `apply_doc_op`).
/// Raw handles are fine here (unlike `doc_fingerprint`, which must survive a
/// full document reload/replay): this snapshot only ever compares against
/// another snapshot of the SAME live `Document`, taken immediately before
/// and after one `replace_material` + its own immediate undo/redo — it never
/// crosses an earlier structural op's own undo/redo, so it can't land on the
/// face-handle-drift gap `doc_fingerprint`'s doc comment describes.
type MaterialsSnapshot = Vec<ObjectMaterialsSnapshot>;

fn snapshot_materials(doc: &Document) -> MaterialsSnapshot {
    let mut objects: Vec<_> = doc
        .visible_object_ids()
        .into_iter()
        .map(|oid| {
            let obj = doc.object(oid).expect("visible id resolves");
            let mut faces: Vec<(FaceId, Option<MaterialId>)> = obj
                .faces()
                .iter()
                .map(|(fid, f)| (fid, f.material))
                .collect();
            faces.sort_unstable_by_key(|&(fid, _)| fid);
            (oid, obj.default_material(), faces)
        })
        .collect();
    objects.sort_unstable_by_key(|&(oid, _, _)| oid);
    objects
}

/// Seeds one box by sketching a rectangle on the ground plane and extruding.
/// Overlapping seed rectangles extrude directly — the standing-solid gate was
/// dropped, so interpenetration is allowed everywhere (the sketch-solid-model design).
fn add_box(doc: &mut Document, x: f64, y: f64, dx: f64, dy: f64, h: f64) -> ObjectId {
    let plane = Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
        .expect("unit normal");
    let s = doc.add_sketch(plane);
    doc.begin_sketch_gesture(s).expect("gesture opens");
    {
        let sk = doc.sketch_mut(s).expect("sketch exists");
        let p = [
            Point3::new(x, y, 0.0),
            Point3::new(x + dx, y, 0.0),
            Point3::new(x + dx, y + dy, 0.0),
            Point3::new(x, y + dy, 0.0),
        ];
        for k in 0..4 {
            sk.add_segment(p[k], p[(k + 1) % 4]).expect("segment adds");
        }
    }
    doc.end_sketch_gesture(s).expect("gesture closes");
    let region = doc
        .sketch(s)
        .expect("sketch exists")
        .regions()
        .keys()
        .next()
        .expect("rectangle closes one region");
    let (oid, _) = doc.extrude_region(s, region, h).expect("box extrudes");
    oid
}

/// The tolerated undo/redo gaps surfacing through the document layer:
/// - the `UnbuildPushPull` inverse refusing typed when its recorded walls are
///   no longer pristine quads (see `op_fuzz.rs::is_known_inverse_guard_gap`
///   for the full rationale). The two op-level gaps this once also
///   tolerated were retired by proof-carrying replay and are deliberately
///   not accepted here.
/// - a typed `SketchError::RestoreConflicts` refusal against a plain
///   (non-`PlaceTextCompound`) `CreatedObject`'s removed scaffolding —
///   proven safe, atomic, document-untouched by
///   `Document::undo_created_object`'s own push-back; torture mode still
///   panics immediately on any actual corruption regardless of this
///   tolerance, so abandoning the round-trip here never masks a real
///   defect. Narrowed by
///   `Document::peek_undo_action_kind`/`peek_redo_action_kind` to EXACTLY
///   that one documented-intended case — mirroring the `UnbuildPushPull`
///   peek-narrowing right below: a `RestoreConflicts` surfacing against
///   `DetachedSketchIsland` (or anything else) is a DIFFERENT, unproven
///   refusal path and must not be silently swallowed here.
///
///   A `PlaceTextCompound` 3D-Text placement is deliberately NOT in this
///   list. `DocOp::TouchGlyphSketch` used to leave untracked geometry
///   sitting on a bundled `CreatedObject`'s removed scaffolding position
///   while `place_text`'s scratch sketch was still live, making
///   `Document::undo_place_text_compound`'s feasibility pre-check refuse
///   typed (pinned, before its deletion, by document.rs's
///   `undo_compound_refuses_atomically_when_a_counter_glyph_kept_the_sketch_live`
///   unit spec). `place_text` now always retires (hides) that sketch
///   before returning (see its cleanup step and
///   `Document::compound_reversal_feasible`'s doc comment), closing the
///   only known window: nothing can touch the sketch while its
///   `PlaceTextCompound` is still pending, so this refusal is expected to
///   never fire again. If it ever does, that is a regression, not a
///   tolerated gap — letting it fail loudly here is the correct trap
///   posture, so `PlaceTextCompound` is not matched below.
///
/// Material replays are deliberately not tolerated: a face-attribute action
/// finds its face again by geometry when a structural undo or redo in
/// between re-mints the handle, so `ReplaceMaterialReplayStale` (and
/// `UnknownFace` from a paint or Position Texture replay) means a kernel bug.
fn is_known_inverse_guard_gap(doc: &Document, e: &DocumentError, redo: bool) -> bool {
    if matches!(e, DocumentError::Sketch(SketchError::RestoreConflicts)) {
        let pending = if redo {
            doc.peek_redo_action_kind()
        } else {
            doc.peek_undo_action_kind()
        };
        return matches!(pending, Some(PendingActionKind::CreatedObject));
    }
    let signature = matches!(
        e,
        DocumentError::InverseFailed(KernelOpError::PushPull(PushPullError::NonManifoldResult))
    );
    let pending = if redo {
        doc.peek_redo_object_op()
    } else {
        doc.peek_undo_object_op()
    };
    signature && matches!(pending, Some(KernelOp::UnbuildPushPull { .. }))
}

/// Every visible object validates and is watertight — WORLD objects via
/// `visible_object_ids`, and every live DEFINITION MEMBER too
/// (component-edit-parity.md phase K2): `debug_validate`'s always-on torture
/// check already runs `validate()` over every live object regardless of
/// ownership, but it does not assert full watertightness, so this test-level
/// check widens to definition members explicitly — the new K2 def-op family
/// (`follow_me_in_instance*`, `boolean_in_component`, `slice_def_member`,
/// `transform_def_member`) all birth or edit members, and a member that
/// merely validates but has opened up (non-manifold, leaky) would otherwise
/// go completely undetected by this harness.
fn check_doc(doc: &Document, step: usize, what: &str) -> Result<(), TestCaseError> {
    for oid in doc.visible_object_ids() {
        let obj = doc.object(oid).expect("visible id resolves");
        if let Err(e) = obj.validate() {
            return Err(TestCaseError::fail(format!(
                "step {step} ({what}): object {oid:?} invalid: {e}"
            )));
        }
        prop_assert_eq!(
            obj.watertight(),
            kernel::WatertightState::Watertight,
            "step {} ({}): object {:?} opened up",
            step,
            what,
            oid
        );
        // The validator ignores shell orientation, so a baked reflection
        // that failed to turn its faces over would pass every check above;
        // a right-side-out closed shell has positive signed volume.
        prop_assert!(
            obj.signed_volume() > 0.0,
            "step {} ({}): object {:?} is inside out",
            step,
            what,
            oid
        );
    }
    for cid in doc.component_ids() {
        let live: Vec<ObjectId> = doc
            .def_members(cid)
            .expect("live component resolves")
            .into_iter()
            .filter(|&o| doc.object(o).is_some())
            .collect();
        for oid in live {
            let obj = doc.object(oid).expect("live member resolves");
            if let Err(e) = obj.validate() {
                return Err(TestCaseError::fail(format!(
                    "step {step} ({what}): definition member {oid:?} invalid: {e}"
                )));
            }
            prop_assert_eq!(
                obj.watertight(),
                kernel::WatertightState::Watertight,
                "step {} ({}): definition member {:?} opened up",
                step,
                what,
                oid
            );
        }
    }
    check_sketch_regions(doc, step, what)?;
    Ok(())
}

/// `save` determinism and `load(save())` state-hash fidelity.
fn check_persistence(doc: &Document, what: &str) -> Result<(), TestCaseError> {
    let bytes = doc.save();
    prop_assert_eq!(&bytes, &doc.save(), "{}: save is not deterministic", what);
    let reloaded = match Document::load(&bytes) {
        Ok(d) => d,
        Err(e) => {
            return Err(TestCaseError::fail(format!(
                "{what}: load(save(doc)) failed: {e}"
            )));
        }
    };
    prop_assert_eq!(
        reloaded.state_hash(),
        doc.state_hash(),
        "{}: state hash changed across save/load",
        what
    );
    prop_assert_eq!(
        &reloaded.save(),
        &bytes,
        "{}: save -> load -> save is not byte-identical",
        what
    );
    Ok(())
}

/// KB-22: `save()` returns and its bytes reopen, and every sketch the
/// document enumerates (world sketches plus each live definition's own)
/// names an owning definition that is itself enumerable. Run after every
/// op, undo, and redo — lighter than [`check_persistence`] (no state-hash or
/// byte comparison), so the full unwind/replay can afford it at each step.
fn check_save_and_sketch_owners(doc: &Document, what: &str) -> Result<(), TestCaseError> {
    let comps = doc.component_ids();
    let mut sketches = doc.sketch_ids();
    for &cid in &comps {
        sketches.extend(
            doc.def_member_sketches(cid)
                .expect("live component resolves")
                .into_iter()
                .filter(|&s| doc.sketch(s).is_some()),
        );
    }
    for sid in sketches {
        if let Some(owner) = doc.sketch_owner_component(sid) {
            prop_assert!(
                comps.contains(&owner),
                "{}: sketch {:?} is owned by {:?}, which is not enumerable",
                what,
                sid,
                owner
            );
        }
    }
    let bytes = doc.save();
    if let Err(e) = Document::load(&bytes) {
        return Err(TestCaseError::fail(format!(
            "{what}: load(save(doc)) failed: {e}"
        )));
    }
    Ok(())
}

/// Canonical, slot-order-independent document fingerprint: per visible
/// object, its sorted vertex positions (rounded well below any op tolerance)
/// and sorted face sizes; plus node/instance/component counts. Two documents
/// with equal fingerprints are the same model up to object and face handle
/// renaming (material handles print as they are; they never change within a
/// document).
///
/// Each face's front and back material and texture frame are part of it,
/// keyed by the face's outer-loop positions, so a paint, Replace Material,
/// Delete Material or Position Texture that a full unwind and replay fails to
/// restore shows up as a mismatch. Those actions find their face again by
/// geometry when a structural undo or redo in between re-mints its handle.
fn doc_fingerprint(doc: &Document) -> String {
    let q = |c: f64| (c * 1e7).round() as i64;
    let object_print = |oid: ObjectId| {
        let obj = doc.object(oid).expect("live id resolves");
        let (pts, faces) = obj.to_polygons();
        let mut vs: Vec<(i64, i64, i64)> = pts.iter().map(|p| (q(p.x), q(p.y), q(p.z))).collect();
        vs.sort_unstable();
        let mut sizes: Vec<usize> = faces.iter().map(Vec::len).collect();
        sizes.sort_unstable();
        let mut paint: Vec<String> = obj
            .faces()
            .values()
            .map(|f| {
                let mut ring: Vec<(i64, i64, i64)> = obj
                    .loop_positions(f.outer_loop)
                    .map(|p| (q(p.x), q(p.y), q(p.z)))
                    .collect();
                ring.sort_unstable();
                // A Move maps the frame, so its undo carries float noise.
                let frame = f
                    .uv_frame
                    .map(|u| [u.s.x, u.s.y, u.s.z, u.t.x, u.t.y, u.t.z, u.u0, u.v0].map(q));
                format!("{ring:?}:{:?}/{:?}/{frame:?}", f.material, f.back_material)
            })
            .collect();
        paint.sort_unstable();
        format!("{vs:?}|{sizes:?}|{paint:?}")
    };
    let mut objects: Vec<String> = doc
        .visible_object_ids()
        .into_iter()
        .map(object_print)
        .collect();
    objects.sort_unstable();
    // Definition members are not world objects; fingerprint them per
    // component, and instances by their (quantized) poses, or a replay that
    // corrupts a definition or restores a wrong pose would compare equal.
    // A definition's `members` list stays populated (hidden, not removed)
    // across an undone birth or a `delete_def_member` — component-edit-
    // parity.md phase K1, mirroring how a hidden world object stays listed
    // in its parent group. Only LIVE members are part of the fingerprint;
    // `object_print` would panic on a hidden id.
    let mut defs: Vec<String> = doc
        .component_ids()
        .into_iter()
        .map(|cid| {
            let mut members: Vec<String> = doc
                .def_members(cid)
                .expect("live component resolves")
                .into_iter()
                .filter(|&oid| doc.object(oid).is_some())
                .map(object_print)
                .collect();
            members.sort_unstable();
            format!("{members:?}")
        })
        .collect();
    defs.sort_unstable();
    let mut poses: Vec<Vec<i64>> = doc
        .instance_ids()
        .into_iter()
        .map(|iid| {
            doc.instance_pose(iid)
                .expect("live instance resolves")
                .to_affine()
                .iter()
                .map(|&c| q(c))
                .collect()
        })
        .collect();
    poses.sort_unstable();
    // World sketches, and each definition's OWN (live) def-owned sketches
    // (component-edit-parity.md phase K1 follow-up) — see `sketch_print`.
    // A hidden/consumed def-owned sketch is skipped by `sketch()` returning
    // `None`, exactly like a hidden member is skipped above.
    let mut world_sketches: Vec<String> = doc
        .sketch_ids()
        .into_iter()
        .map(|sid| owned_sketch_print(doc, sid, &object_print))
        .collect();
    world_sketches.sort_unstable();
    let mut def_sketches: Vec<String> = doc
        .component_ids()
        .into_iter()
        .map(|cid| {
            let mut owned: Vec<String> = doc
                .def_member_sketches(cid)
                .expect("live component resolves")
                .into_iter()
                .filter(|&sid| doc.sketch(sid).is_some())
                .map(|sid| owned_sketch_print(doc, sid, &object_print))
                .collect();
            owned.sort_unstable();
            format!("{owned:?}")
        })
        .collect();
    def_sketches.sort_unstable();
    // The movable drawing axes frame (tool-parity design §4): both `Rescale`
    // and `SetAxes` mutate it (directly or, for `Rescale`, only its origin —
    // see `Document::rescale_document`'s doc comment), so it must be in the
    // fingerprint or a replay that drops/corrupts it compares equal to one
    // that doesn't.
    let axes = doc.axes();
    let axes_print = (
        q(axes.origin.x),
        q(axes.origin.y),
        q(axes.origin.z),
        q(axes.x.x),
        q(axes.x.y),
        q(axes.x.z),
        q(axes.y.x),
        q(axes.y.y),
        q(axes.y.z),
    );
    // Annotations: kind + quantized geometry + whether each anchor still
    // names a node + `detached`. Quantized for the same reason object
    // vertices are: re-anchoring composes transforms exactly like a baked
    // object move does, so it inherits the identical ulp-noise trap.
    // Node identity is deliberately NOT embedded (ids can differ across a
    // hypothetical fresh replay, matching the id-independent philosophy
    // `object_print` above already follows); "has a node" is enough to catch
    // a reanchor/detach regression.
    let mut annotations: Vec<String> = doc
        .annotations()
        .into_iter()
        .map(|(_, a, detached)| annotation_print(&a, detached, q))
        .collect();
    annotations.sort_unstable();
    format!(
        "objs={objects:?} defs={defs:?} poses={poses:?} groups={} \
         world_sketches={world_sketches:?} def_sketches={def_sketches:?} axes={axes_print:?} \
         annotations={annotations:?}",
        doc.group_ids().len()
    )
}

/// Canonical, id-independent fingerprint of one sketch's geometry: sorted
/// (quantized-position) edge endpoint pairs plus region count. The
/// def-owned-sketch analog of `object_print` in `doc_fingerprint` — added
/// alongside `DocOp::DrawInInstance` (component-edit-parity.md phase K1
/// follow-up), which, unlike `DrawAndExtrudeInInstance`, can leave a LIVE
/// def-owned sketch standing at a fingerprinted state; without this, a
/// `make_unique`/`explode_instance` bug that corrupts a cloned/baked sketch
/// (but leaves every Object alone) would go completely undetected by the
/// round-trip check below.
type QuantizedPoint = (i64, i64, i64);

fn sketch_print(s: &kernel::Sketch) -> String {
    let q = |c: f64| (c * 1e7).round() as i64;
    let qp = |p: Point3| (q(p.x), q(p.y), q(p.z));
    let mut edges: Vec<(QuantizedPoint, QuantizedPoint)> = s
        .edges()
        .values()
        .map(|e| {
            let a = qp(s.vertices()[e.from].position);
            let b = qp(s.vertices()[e.to].position);
            if a <= b { (a, b) } else { (b, a) }
        })
        .collect();
    edges.sort_unstable();
    format!("{edges:?}|regions={}", s.regions().len())
}

/// [`sketch_print`] plus, for an object-owned sketch, the owner's geometry
/// fingerprint — so a replay that drops, re-points, or mis-moves an owner
/// link (the `object_sketches` table) compares unequal.
fn owned_sketch_print(
    doc: &Document,
    sid: kernel::SketchId,
    object_print: &dyn Fn(ObjectId) -> String,
) -> String {
    let base = sketch_print(doc.sketch(sid).expect("live id resolves"));
    match doc.sketch_owner_object(sid) {
        Some(owner) => format!("{base}@{}", object_print(owner)),
        None => base,
    }
}

/// Quantized, id-independent fingerprint of one annotation — see
/// `doc_fingerprint`'s doc comment for why quantization is needed here too.
fn annotation_print(a: &Annotation, detached: bool, q: impl Fn(f64) -> i64) -> String {
    let qp = |p: Point3| (q(p.x), q(p.y), q(p.z));
    let qv = |v: Vec3| (q(v.x), q(v.y), q(v.z));
    // Quantized plane identity: normal direction plus the representative
    // point `Plane::point()` returns — covers a plane re-derivation
    // regression (findings around `LinearDimension`'s one-anchor-touched
    // re-derivation and `RadialDimension`'s frozen curve) that a print
    // omitting the plane entirely would silently miss.
    let qplane = |p: &Plane| (qv(p.normal()), qp(p.point()));
    let has_node = |anchor: &Anchor| anchor.node.is_some();
    match a {
        Annotation::LinearDimension {
            a,
            b,
            offset,
            plane,
            text_override,
        } => format!(
            "linear|{}|{:?}|{}|{:?}|{:?}|{:?}|{text_override:?}|{detached}",
            has_node(a),
            qp(a.point),
            has_node(b),
            qp(b.point),
            qv(*offset),
            qplane(plane),
        ),
        Annotation::RadialDimension {
            anchor,
            kind,
            curve,
            leader_dir,
            text_override,
        } => {
            let radius = q(curve.radius);
            format!(
                "radial|{}|{:?}|{kind:?}|{radius}|{:?}|{:?}|{:?}|{text_override:?}|{detached}",
                has_node(anchor),
                qp(anchor.point),
                qp(curve.center),
                qplane(&curve.plane),
                qv(*leader_dir),
            )
        }
        Annotation::LeaderText {
            anchor,
            offset,
            text,
        } => format!(
            "leader|{}|{:?}|{:?}|{text:?}|{detached}",
            has_node(anchor),
            qp(anchor.point),
            qv(*offset),
        ),
    }
}

/// The live world sketches the free-sketch ops (draw, move, drag, remove)
/// pick from: not object-owned, and on a horizontal plane. Those ops assume
/// a ground-style sketch (their lattice moves are XY translations, which
/// keep a horizontal sketch in plane), which an object-owned sketch on an
/// arbitrary face plane is not — nor is a sketch that was owned and then
/// freed again by an undo. An owned sketch is exercised through the ops
/// that carry it (transforms, push/pull, booleans, slices, duplicates,
/// sessions, undo/redo) and [`DocOp::OwnSketch`] itself.
fn free_sketch_ids(doc: &Document) -> Vec<kernel::SketchId> {
    doc.sketch_ids()
        .into_iter()
        .filter(|&s| doc.sketch_owner_object(s).is_none())
        .filter(|&s| {
            let n = doc.sketch(s).expect("live id").plane().normal();
            n.x == 0.0 && n.y == 0.0
        })
        .collect()
}

/// The ground sketch the world-sketch ops draw on: the first live world
/// sketch lying on the z = 0 plane facing +z, or a fresh one.
fn ground_sketch(doc: &mut Document) -> kernel::SketchId {
    let found = free_sketch_ids(doc).into_iter().find(|&sid| {
        let sk = doc.sketch(sid).expect("live id");
        let plane = sk.plane();
        let n = plane.normal();
        // A detached shape is no ground drawing: it joins nothing.
        !sk.detached() && n.x == 0.0 && n.y == 0.0 && n.z == 1.0 && plane.point().z == 0.0
    });
    found.unwrap_or_else(|| {
        let plane = Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
            .expect("unit normal");
        doc.add_sketch(plane)
    })
}

/// The live world detached sketches ([`DocOp::EditDetached`] and
/// [`DocOp::ExtrudeDetached`] pick from these), in id order.
fn detached_sketch_ids(doc: &Document) -> Vec<kernel::SketchId> {
    let mut ids: Vec<kernel::SketchId> = doc
        .sketch_ids()
        .into_iter()
        .filter(|&s| doc.sketch(s).expect("live id").detached())
        .collect();
    ids.sort_unstable();
    ids
}

/// Every live world detached sketch as it stands.
fn detached_snapshot(doc: &Document) -> Vec<(kernel::SketchId, kernel::Sketch)> {
    detached_sketch_ids(doc)
        .into_iter()
        .map(|s| (s, doc.sketch(s).expect("live id").clone()))
        .collect()
}

/// The sketch `op` itself edits, resolved against `doc` BEFORE it applies:
/// the free-sketch ops' and the detached ops' picks. Any other detached
/// sketch must come through the op unchanged.
fn op_target_sketch(doc: &Document, op: &DocOp) -> Option<kernel::SketchId> {
    match *op {
        DocOp::MoveIsland { sk_sel, .. }
        | DocOp::MoveIslands { sk_sel, .. }
        | DocOp::RemoveSketchEdge { sk_sel, .. }
        | DocOp::MoveSketchVertex { sk_sel, .. }
        | DocOp::OffsetRegion { sk_sel, .. }
        | DocOp::RefacetCurve { sk_sel, .. } => nth(&free_sketch_ids(doc), sk_sel),
        DocOp::EditDetached { sk_sel, .. } | DocOp::ExtrudeDetached { sk_sel, .. } => {
            nth(&detached_sketch_ids(doc), sk_sel)
        }
        _ => None,
    }
}

/// Detached shapes §3.8: every detached sketch of `before` that `op` does
/// not itself edit (`target`) and that is still live is `==` (plane, flag,
/// drawing, handles) — no plane arrangement ever reached it — and no live
/// detached sketch is object-owned (I9). Undo, redo, and a whole-document
/// rescale legitimately change every sketch and are not checked.
fn check_detached_untouched(
    doc: &Document,
    step: usize,
    op: &DocOp,
    before: &[(kernel::SketchId, kernel::Sketch)],
    target: Option<kernel::SketchId>,
) -> Result<(), TestCaseError> {
    for s in detached_sketch_ids(doc) {
        prop_assert!(
            doc.sketch_owner_object(s).is_none(),
            "step {}: detached sketch {:?} is object-owned (I9)",
            step,
            s
        );
    }
    if matches!(op, DocOp::Undo | DocOp::Redo | DocOp::Rescale { .. }) {
        return Ok(());
    }
    for (s, sk) in before {
        if Some(*s) == target {
            continue;
        }
        if let Some(now) = doc.sketch(*s) {
            prop_assert!(
                now == sk,
                "step {} ({:?}): detached sketch {:?} changed",
                step,
                op,
                s
            );
        }
    }
    Ok(())
}

/// An in-plane basis of `n`.
fn plane_basis_of(n: Vec3) -> (Vec3, Vec3) {
    let helper = if n.x.abs() < 0.9 {
        Vec3::new(1.0, 0.0, 0.0)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };
    let u = n.cross(helper).normalized().expect("non-parallel helper");
    (u, n.cross(u))
}

fn lattice_translation(dx: i32, dy: i32) -> Transform {
    Transform::translation(Vec3::new(f64::from(dx) * 0.5, f64::from(dy) * 0.5, 0.0))
}

/// Every world and definition-owned sketch handle.
fn all_sketch_ids(doc: &Document) -> Vec<kernel::SketchId> {
    let mut ids = doc.sketch_ids();
    for cid in doc.component_ids() {
        if let Some(owned) = doc.def_member_sketches(cid) {
            ids.extend(owned);
        }
    }
    ids
}

/// Every region of every live sketch has simple rings (no ring visits a
/// vertex twice: a walk through a touch point is split there), and yields a
/// valid `Profile` unless two of its rings share a vertex (a hole touching
/// the outer ring or another hole at one point — a region the arrangement
/// reports but no single manifold solid can be extruded from, refused
/// typed as `RegionTouchesHole`).
fn check_sketch_regions(doc: &Document, step: usize, what: &str) -> Result<(), TestCaseError> {
    for sid in all_sketch_ids(doc) {
        let Some(sk) = doc.sketch(sid) else { continue };
        for (rid, r) in sk.regions() {
            let rings: Vec<&Vec<kernel::SketchVertexId>> =
                std::iter::once(&r.outer).chain(r.holes.iter()).collect();
            for ring in &rings {
                let mut seen = std::collections::BTreeSet::new();
                if !ring.iter().all(|v| seen.insert(*v)) {
                    return Err(TestCaseError::fail(format!(
                        "step {step} ({what}): sketch {sid:?} region {rid:?} has a ring \
                         visiting a vertex twice"
                    )));
                }
            }
            let touching = (0..rings.len()).any(|i| {
                (i + 1..rings.len()).any(|j| rings[i].iter().any(|v| rings[j].contains(v)))
            });
            match sk.profile(rid) {
                Err(e) if !touching => {
                    return Err(TestCaseError::fail(format!(
                        "step {step} ({what}): sketch {sid:?} region {rid:?} has no profile: {e:?}"
                    )));
                }
                Ok(_) if touching => {
                    return Err(TestCaseError::fail(format!(
                        "step {step} ({what}): sketch {sid:?} region {rid:?} with touching \
                         rings gave a profile"
                    )));
                }
                Err(e) if touching && e != SketchError::RegionTouchesHole => {
                    return Err(TestCaseError::fail(format!(
                        "step {step} ({what}): sketch {sid:?} region {rid:?} with touching \
                         rings refused as {e:?}, not RegionTouchesHole"
                    )));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// A small triangle strictly inside the `face_sel`-th face of `obj`, on the
/// face's own plane: the three corners, the face's interior point, and the
/// face normal. `None` when the face is too small or has no clear interior
/// point (the loop centroid must lie inside the outer loop and outside
/// every hole, with clearance from every boundary edge).
fn interior_triangle(
    obj: &kernel::Object,
    face_sel: usize,
    turn: f64,
) -> Option<([Point3; 3], Point3, Vec3)> {
    let faces = obj.faces();
    let fid = faces.keys().nth(face_sel % faces.len())?;
    let face = &faces[fid];
    let n = face.plane.normal();
    let outer: Vec<Point3> = obj.loop_positions(face.outer_loop).collect();
    let holes: Vec<Vec<Point3>> = face
        .inner_loops
        .iter()
        .map(|&l| obj.loop_positions(l).collect())
        .collect();
    // In-plane basis.
    let helper = if n.x.abs() < 0.9 {
        Vec3::new(1.0, 0.0, 0.0)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };
    let u = n.cross(helper).normalized().ok()?;
    let v = n.cross(u);
    let k = 1.0 / outer.len() as f64;
    let c = outer
        .iter()
        .fold(Point3::ORIGIN, |acc, &q| acc + q.to_vec() * k);
    let to2 = |q: Point3| ((q - c).dot(u), (q - c).dot(v));
    let inside = |poly: &[Point3]| {
        let pts: Vec<(f64, f64)> = poly.iter().map(|&q| to2(q)).collect();
        let mut hit = false;
        let mut j = pts.len() - 1;
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[j]);
            if (a.1 > 0.0) != (b.1 > 0.0) && 0.0 < (b.0 - a.0) * (0.0 - a.1) / (b.1 - a.1) + a.0 {
                hit = !hit;
            }
            j = i;
        }
        hit
    };
    if !inside(&outer) || holes.iter().any(|h| inside(h)) {
        return None;
    }
    // Clearance: distance from the centroid to every boundary edge.
    let mut dmin = f64::MAX;
    for poly in std::iter::once(&outer).chain(holes.iter()) {
        for i in 0..poly.len() {
            let (a, b) = (to2(poly[i]), to2(poly[(i + 1) % poly.len()]));
            let ab = (b.0 - a.0, b.1 - a.1);
            let len2 = ab.0 * ab.0 + ab.1 * ab.1;
            if len2 < 1e-18 {
                continue;
            }
            let t = (-(a.0 * ab.0 + a.1 * ab.1) / len2).clamp(0.0, 1.0);
            let d = ((a.0 + t * ab.0).powi(2) + (a.1 + t * ab.1).powi(2)).sqrt();
            dmin = dmin.min(d);
        }
    }
    let r = 0.4 * dmin;
    if r.is_nan() || r <= 0.02 {
        return None;
    }
    let corner = |ang: f64| c + u * (r * ang.cos()) + v * (r * ang.sin());
    let tri = [
        corner(turn),
        corner(turn + 2.094_395_1),
        corner(turn + 4.188_790_2),
    ];
    Some((tri, c, n))
}

fn nth<T: Copy>(items: &[T], sel: usize) -> Option<T> {
    if items.is_empty() {
        None
    } else {
        Some(items[sel % items.len()])
    }
}

/// Builds a glyph-like ring+counter sketch (an outer square with a smaller
/// concentric square hole — `document.rs`'s `glyph_o_sketch` fixture,
/// re-expressed against the public API since this is an external test) at
/// `(x, y)` and extrudes ONLY the ring via `place_text`. `None` on any
/// refusal (typically a degenerate/overlapping position) or if the drawn
/// geometry didn't resolve the expected two regions — always harmless to
/// the caller.
fn place_glyph(doc: &mut Document, x: f64, y: f64, depth: f64) -> Option<()> {
    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)).ok()?;
    let sketch = doc.add_sketch(plane);
    doc.begin_sketch_gesture(sketch).ok()?;
    let outer = [
        Point3::new(x - 1.0, y - 1.0, 0.0),
        Point3::new(x + 1.0, y - 1.0, 0.0),
        Point3::new(x + 1.0, y + 1.0, 0.0),
        Point3::new(x - 1.0, y + 1.0, 0.0),
    ];
    let inner = [
        Point3::new(x - 0.4, y - 0.4, 0.0),
        Point3::new(x + 0.4, y - 0.4, 0.0),
        Point3::new(x + 0.4, y + 0.4, 0.0),
        Point3::new(x - 0.4, y + 0.4, 0.0),
    ];
    {
        let sk = doc.sketch_mut(sketch)?;
        for k in 0..4 {
            sk.add_segment(outer[k], outer[(k + 1) % 4]).ok()?;
        }
        for k in 0..4 {
            sk.add_segment(inner[k], inner[(k + 1) % 4]).ok()?;
        }
    }
    doc.end_sketch_gesture(sketch).ok()?;

    let regions: Vec<SketchRegionId> = doc.sketch(sketch)?.regions().keys().collect();
    if regions.len() != 2 {
        // A fuzzed position overlapping earlier geometry can resolve into
        // more/fewer regions than the clean two-loop case — abandon
        // harmlessly rather than guess which one is the "ring".
        return None;
    }
    let ring = regions
        .iter()
        .copied()
        .find(|&r| !doc.sketch(sketch).unwrap().regions()[r].holes.is_empty())?;

    let (_component, _instance, _change) = doc
        .place_text(sketch, &[ring], depth, "Fuzz Glyph".to_string(), None)
        .ok()?;
    Some(())
}

/// Applies one abstract op. `materials` is the fixed palette `seed_materials`
/// built up front (paint ops resolve their `mat_sel`/`from_sel`/`to_sel`
/// against it).
/// Whether face `f` of `obj` lies on `plane` (same facing, every outer
/// vertex within a nanometre) — the drafting engine's own criterion.
fn fuzz_face_on_plane(obj: &kernel::Object, f: FaceId, plane: &Plane) -> bool {
    let face = &obj.faces()[f];
    (face.plane.normal() - plane.normal()).length() < 1e-9
        && obj
            .loop_positions(face.outer_loop)
            .all(|p| plane.signed_distance(p).abs() < 1e-9)
}

/// The solids' own edges on `plane`: edges of a face on the plane whose
/// other side is not (the drafting engine never moves or removes these).
fn fixed_segments(doc: &Document, objs: &[ObjectId], plane: &Plane) -> Vec<(Point3, Point3)> {
    let mut out = Vec::new();
    for &o in objs {
        let obj = doc.object(o).expect("visible");
        let face_of = |h| obj.loops()[obj.half_edges()[h].loop_id].face;
        for (e, ed) in obj.edges().iter() {
            let fa = face_of(ed.half_edge);
            let on_a = fuzz_face_on_plane(obj, fa, plane);
            let on_b = ed
                .twin_half_edge
                .is_some_and(|t| fuzz_face_on_plane(obj, face_of(t), plane));
            if on_a != on_b || (on_a && ed.twin_half_edge.is_none()) {
                out.push(obj.edge_endpoints(e).expect("live edge"));
            }
        }
    }
    out
}

/// The plane a [`DocOp::Draft`] (or [`DocOp::DrawDetached`]) draws on and
/// its lattice frame (origin, first axis): the `face_sel`-th face of the
/// `obj_sel`-th visible object, anchored at its first outer vertex along
/// its first edge; or the ground, when `ground` or nothing is visible.
/// `None` when the face's first edge has no direction.
fn draft_lattice(
    doc: &Document,
    obj_sel: usize,
    face_sel: usize,
    ground: bool,
) -> Option<(Plane, Point3, Vec3)> {
    let visible = doc.visible_object_ids();
    match nth(&visible, obj_sel).filter(|_| !ground) {
        None => Some((
            Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
                .expect("unit normal"),
            Point3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        )),
        Some(oid) => {
            let obj = doc.object(oid).expect("visible");
            let f = obj.faces().keys().nth(face_sel % obj.faces().len())?;
            let ring: Vec<Point3> = obj.loop_positions(obj.faces()[f].outer_loop).collect();
            let plane = obj.faces()[f].plane;
            let u = (ring[1] - ring[0]).normalized().ok()?;
            Some((plane, ring[0], u))
        }
    }
}

/// One [`DocOp::DrawDetached`]: a new detached shape, one undo step.
#[allow(clippy::too_many_arguments)]
fn apply_draw_detached(
    doc: &mut Document,
    step: usize,
    obj_sel: usize,
    face_sel: usize,
    ground: bool,
    circle: bool,
    a: (i32, i32),
    b: (i32, i32),
    segments: usize,
) -> Result<(), TestCaseError> {
    let Some((plane, origin, u)) = draft_lattice(doc, obj_sel, face_sel, ground) else {
        return Ok(());
    };
    let v = plane.normal().cross(u);
    let at = |x: i32, y: i32| origin + u * (f64::from(x) * 0.5) + v * (f64::from(y) * 0.5);
    let depth0 = doc.undo_depth();
    let result = doc.draft_detached_new(plane, None, "Fuzz detached", |sk| {
        if circle {
            let center = at(a.0, a.1);
            let radius = f64::from(b.0) * 0.5;
            let ring: Vec<Point3> = (0..segments)
                .map(|i| {
                    let t = std::f64::consts::TAU * i as f64 / segments as f64;
                    center + u * (radius * t.cos()) + v * (radius * t.sin())
                })
                .collect();
            sk.begin_curve_with(kernel::CurveGeom { center, radius })?;
            for i in 0..segments {
                sk.add_segment(ring[i], ring[(i + 1) % segments])?;
            }
            sk.end_curve();
        } else {
            let (x1, y1) = (a.0 + b.0, a.1 + b.1);
            let c = [at(a.0, a.1), at(x1, a.1), at(x1, y1), at(a.0, y1)];
            for k in 0..4 {
                sk.add_segment(c[k], c[(k + 1) % 4])?;
            }
        }
        Ok(())
    });
    match result {
        Ok((report, _)) => {
            prop_assert_eq!(doc.undo_depth(), depth0 + 1, "step {}: one undo step", step);
            prop_assert!(
                report.objects.is_empty(),
                "step {}: an object changed",
                step
            );
            prop_assert_eq!(report.sketches.len(), 1, "step {}: one new sketch", step);
            let s = report.sketches[0];
            prop_assert!(
                doc.sketch(s).is_some_and(|sk| sk.detached()),
                "step {}: the new sketch is not detached",
                step
            );
            Ok(())
        }
        Err(e) => Err(TestCaseError::fail(format!(
            "step {step}: a detached shape was refused: {e:?} (plane {plane:?})"
        ))),
    }
}

/// One [`DocOp::EditDetached`]: an edit in the sketch's own context.
fn apply_edit_detached(
    doc: &mut Document,
    step: usize,
    sk_sel: usize,
    mode: u8,
    sel: usize,
    d: (i32, i32),
) -> Result<(), TestCaseError> {
    let Some(s) = nth(&detached_sketch_ids(doc), sk_sel) else {
        return Ok(());
    };
    let sk = doc.sketch(s).expect("live").clone();
    let (u, v) = plane_basis_of(sk.plane().normal());
    let by = u * (f64::from(d.0) * 0.5) + v * (f64::from(d.1) * 0.5);
    let units = match doc.detached_sketch_units(s) {
        Ok(units) => units,
        Err(e) => {
            return Err(TestCaseError::fail(format!(
                "step {step}: a detached sketch has no listing: {e:?}"
            )));
        }
    };
    let depth0 = doc.undo_depth();
    let result = doc.draft_detached(s, "Fuzz detached edit", |w| {
        match mode {
            0 => {
                if units.units.is_empty() || by.length() == 0.0 {
                    return Ok(());
                }
                let unit = &units.units[sel % units.units.len()];
                let es: Vec<_> = unit
                    .edges
                    .iter()
                    .map(|e| {
                        w.edge_at_positions(e.a, e.b)
                            .expect("a listed edge is in the lift")
                    })
                    .collect();
                w.apply_transform_edges(&es, &Transform::translation(by))?;
            }
            1 => {
                let es: Vec<_> = w.edges().keys().collect();
                if let Some(e) = nth(&es, sel) {
                    w.remove_edge(e)?;
                }
            }
            _ => {
                let vs: Vec<Point3> = w.vertices().values().map(|x| x.position).collect();
                if let Some(p) = nth(&vs, sel)
                    && by.length() > 0.0
                {
                    w.add_segment(p, p + by)?;
                }
            }
        }
        Ok(())
    });
    match result {
        Ok((report, _)) => {
            let committed = doc.undo_depth();
            prop_assert!(
                committed == depth0 || committed == depth0 + 1,
                "step {}: a detached edit recorded {} steps",
                step,
                committed.saturating_sub(depth0)
            );
            prop_assert!(
                report.objects.is_empty(),
                "step {}: an object changed",
                step
            );
            prop_assert!(
                report.sketches.iter().all(|&x| x == s),
                "step {}: a detached edit wrote another sketch: {:?}",
                step,
                report.sketches
            );
            if let Some(now) = doc.sketch(s) {
                prop_assert!(now.detached(), "step {}: the sketch lost its flag", step);
            }
            Ok(())
        }
        Err(e) => Err(TestCaseError::fail(format!(
            "step {step}: a detached edit (mode {mode}) was refused: {e:?}"
        ))),
    }
}

/// One [`DocOp::Draft`].
#[allow(clippy::too_many_arguments)]
fn apply_draft(
    doc: &mut Document,
    step: usize,
    obj_sel: usize,
    face_sel: usize,
    ground: bool,
    shape: u8,
    a: (i32, i32),
    b: (i32, i32),
    sel: usize,
    segments: usize,
) -> Result<(), TestCaseError> {
    let visible = doc.visible_object_ids();
    let Some((plane, origin, u)) = draft_lattice(doc, obj_sel, face_sel, ground) else {
        return Ok(());
    };
    let n = plane.normal();
    let v = n.cross(u);
    let at = |x: i32, y: i32| origin + u * (f64::from(x) * 0.5) + v * (f64::from(y) * 0.5);
    let objs: Vec<ObjectId> = visible
        .iter()
        .copied()
        .filter(|&o| {
            let obj = doc.object(o).expect("visible");
            obj.faces()
                .keys()
                .any(|f| fuzz_face_on_plane(obj, f, &plane))
        })
        .collect();
    let fixed = fixed_segments(doc, &objs, &plane);
    let before = doc.save();
    let depth0 = doc.undo_depth();
    let erase = shape >= 3;
    // Shapes 4 and 5 act on a drawing unit of the plane, picked from the
    // lift before the draft opens.
    let unit = if shape >= 4 {
        let Ok(pu) = doc.plane_units(plane, None, &objs) else {
            return Ok(());
        };
        if pu.units.is_empty() {
            return Ok(());
        }
        Some(pu.units[sel % pu.units.len()].clone())
    } else {
        None
    };
    let result = doc.draft(plane, None, &objs, "Fuzz draft", |sk| {
        match shape {
            0 => {
                let (x0, y0) = a;
                let (x1, y1) = (a.0 + b.0, a.1 + b.1);
                let c = [at(x0, y0), at(x1, y0), at(x1, y1), at(x0, y1)];
                for k in 0..4 {
                    sk.add_segment(c[k], c[(k + 1) % 4])?;
                }
            }
            1 => {
                let center = at(a.0, a.1);
                let radius = f64::from(b.0) * 0.5;
                let ring: Vec<Point3> = (0..segments)
                    .map(|i| {
                        let t = std::f64::consts::TAU * i as f64 / segments as f64;
                        center + u * (radius * t.cos()) + v * (radius * t.sin())
                    })
                    .collect();
                sk.begin_curve_with(kernel::CurveGeom { center, radius })?;
                for i in 0..segments {
                    sk.add_segment(ring[i], ring[(i + 1) % segments])?;
                }
                sk.end_curve();
            }
            2 => {
                let (p0, p1) = (at(a.0, a.1), at(a.0 + b.0, a.1 + b.1 - 1));
                if (p1 - p0).length() > 1e-6 {
                    sk.add_segment(p0, p1)?;
                }
            }
            4 | 5 => {
                let unit = unit.as_ref().expect("a unit was picked");
                let es: Vec<_> = unit
                    .edges
                    .iter()
                    .map(|e| {
                        sk.edge_at_positions(e.a, e.b)
                            .expect("a unit edge in the lift")
                    })
                    .collect();
                if shape == 4 {
                    let d = u * (f64::from(a.0 - 6) * 0.5) + v * (f64::from(a.1 - 6) * 0.5);
                    sk.apply_transform_edges(&es, &Transform::translation(d))?;
                } else {
                    for e in es {
                        sk.remove_edge(e)?;
                    }
                }
            }
            _ => {
                // Erase an edge that is not one of a solid's own.
                let safe: Vec<_> = sk
                    .edges()
                    .iter()
                    .filter(|(_, e)| {
                        let (p0, p1) =
                            (sk.vertices()[e.from].position, sk.vertices()[e.to].position);
                        let mid = Point3::new(
                            (p0.x + p1.x) / 2.0,
                            (p0.y + p1.y) / 2.0,
                            (p0.z + p1.z) / 2.0,
                        );
                        !fixed.iter().any(|&(c, d)| {
                            let dir = d - c;
                            let t = ((mid - c).dot(dir) / dir.length_squared()).clamp(0.0, 1.0);
                            (c + dir * t - mid).length() < 1e-7
                        })
                    })
                    .map(|(id, _)| id)
                    .collect();
                if let Some(e) = nth(&safe, sel) {
                    sk.remove_edge(e)?;
                }
            }
        }
        Ok(())
    });
    match result {
        Ok(_) => {
            if doc.draft_sketch().is_some() {
                return Err(TestCaseError::fail(format!(
                    "step {step}: a committed draft is still open"
                )));
            }
            let committed = doc.undo_depth();
            if committed != depth0 && committed != depth0 + 1 {
                return Err(TestCaseError::fail(format!(
                    "step {step}: a draft recorded {} undo steps",
                    committed.saturating_sub(depth0)
                )));
            }
            if committed == depth0 && doc.save() != before {
                return Err(TestCaseError::fail(format!(
                    "step {step}: an unrecorded draft changed the document"
                )));
            }
            Ok(())
        }
        Err(DocumentError::DraftBoundaryEdgeTouched) if erase => {
            if doc.save() != before || doc.draft_sketch().is_some() {
                return Err(TestCaseError::fail(format!(
                    "step {step}: a refused draft left a trace"
                )));
            }
            Ok(())
        }
        Err(e) => Err(TestCaseError::fail(format!(
            "step {step}: a draft was refused: {e:?} (shape {shape}, plane {plane:?}, {} objects)",
            objs.len()
        ))),
    }
}

fn apply_doc_op(
    doc: &mut Document,
    step: usize,
    op: &DocOp,
    materials: &[MaterialId],
) -> Result<bool, TestCaseError> {
    match op {
        DocOp::PushPull {
            obj_sel,
            face_sel,
            distance,
        } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let obj = doc.object(oid).expect("visible id resolves");
            let Some(face) = obj.faces().keys().nth(face_sel % obj.faces().len()) else {
                return Ok(true);
            };
            let _ = doc.apply_object_op(
                oid,
                KernelOp::PushPull {
                    face,
                    distance: *distance,
                },
            );
        }
        DocOp::SplitFace {
            obj_sel,
            face_sel,
            edge_a,
            edge_b,
            ta,
            tb,
        } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let obj = doc.object(oid).expect("visible id resolves");
            let Some(face) = obj.faces().keys().nth(face_sel % obj.faces().len()) else {
                return Ok(true);
            };
            let boundary: Vec<Point3> = obj.loop_positions(obj.faces()[face].outer_loop).collect();
            let sides = boundary.len();
            if sides < 3 {
                return Ok(true);
            }
            let (a, b) = (edge_a % sides, edge_b % sides);
            if a == b {
                return Ok(true);
            }
            let point_on = |i: usize, t: f64| {
                let p = boundary[i];
                let q = boundary[(i + 1) % sides];
                p + (q - p) * t
            };
            let path = vec![point_on(a, *ta), point_on(b, *tb)];
            let _ = doc.apply_object_op(
                oid,
                KernelOp::SplitFace {
                    face,
                    path,
                    restore: None,
                    curves: Vec::new(),
                },
            );
        }
        DocOp::SplitFaceInner {
            obj_sel,
            face_sel,
            shrink,
            staple,
        } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let obj = doc.object(oid).expect("visible id resolves");
            let Some(face) = obj.faces().keys().nth(face_sel % obj.faces().len()) else {
                return Ok(true);
            };
            let boundary: Vec<Point3> = obj.loop_positions(obj.faces()[face].outer_loop).collect();
            if boundary.len() < 3 {
                return Ok(true);
            }
            let loop_path: Vec<Point3> = if *staple {
                // Concave staple in the quad's bilinear frame (quad faces
                // only; skewed quads may still reject typed — fine).
                if boundary.len() != 4 {
                    return Ok(true);
                }
                let (o, ua, vb) = (boundary[0], boundary[1], boundary[3]);
                let at = |a: f64, b: f64| o + (ua - o) * a + (vb - o) * b;
                vec![
                    at(0.2, 0.6),
                    at(0.4, 0.6),
                    at(0.4, 0.8),
                    at(0.6, 0.8),
                    at(0.6, 0.6),
                    at(0.8, 0.6),
                    at(0.8, 0.9),
                    at(0.2, 0.9),
                ]
            } else {
                let inv = 1.0 / boundary.len() as f64;
                let c = boundary.iter().fold(Point3::new(0.0, 0.0, 0.0), |acc, p| {
                    Point3::new(acc.x + p.x * inv, acc.y + p.y * inv, acc.z + p.z * inv)
                });
                boundary.iter().map(|&p| c + (p - c) * *shrink).collect()
            };
            let _ = doc.apply_object_op(
                oid,
                KernelOp::SplitFaceInner {
                    face,
                    loop_path,
                    restore: None,
                    curves: Vec::new(),
                    edge_attrs: Vec::new(),
                },
            );
        }
        DocOp::ImprintTouchingLoop {
            obj_sel,
            face_sel,
            edges,
            ts,
            pull_in,
        } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let obj = doc.object(oid).expect("visible id resolves");
            let Some(face) = obj.faces().keys().nth(face_sel % obj.faces().len()) else {
                return Ok(true);
            };
            let boundary: Vec<Point3> = obj.loop_positions(obj.faces()[face].outer_loop).collect();
            let sides = boundary.len();
            if sides < 3 {
                return Ok(true);
            }
            let mut picked: Vec<usize> = edges.iter().map(|e| e % sides).collect();
            picked.sort_unstable();
            picked.dedup();
            if picked.len() != 3 {
                return Ok(true);
            }
            let point_on = |i: usize, t: f64| {
                let p = boundary[i];
                let q = boundary[(i + 1) % sides];
                p + (q - p) * t
            };
            let mut loop_path: Vec<Point3> = (0..3).map(|k| point_on(picked[k], ts[k])).collect();
            if *pull_in {
                let inv = 1.0 / sides as f64;
                let c = boundary.iter().fold(Point3::new(0.0, 0.0, 0.0), |acc, p| {
                    Point3::new(acc.x + p.x * inv, acc.y + p.y * inv, acc.z + p.z * inv)
                });
                loop_path[2] = loop_path[2] + (c - loop_path[2]) * 0.5;
            }
            // On a face that is hole-free and strictly convex with every
            // side at least 0.1 mm and every corner turning by more than
            // 0.06° (sin > 1e-3), the triangle's sides cross the face's
            // open interior, more than POINT_MERGE from every boundary
            // vertex and edge but at its own two or three corners, so the
            // planner must accept it. Elsewhere (a concave face the
            // triangle can leave, a hole it can cross, sides short or
            // nearly collinear enough to put a corner within POINT_MERGE
            // of a side) a typed refusal is a correct answer.
            let normal = obj.faces()[face].plane.normal();
            let must_accept = obj.faces()[face].inner_loops.is_empty()
                && (0..sides).all(|i| {
                    let a = boundary[i];
                    let b = boundary[(i + 1) % sides];
                    let c = boundary[(i + 2) % sides];
                    let (e0, e1) = (b - a, c - b);
                    e0.length() >= 1e-4
                        && e0.cross(e1).dot(normal) > 1e-3 * e0.length() * e1.length()
                })
                && {
                    let (a, b, c) = (loop_path[0], loop_path[1], loop_path[2]);
                    (b - a).cross(c - a).length() > 1e-3 * (b - a).length() * (c - a).length()
                };
            let plane = face_plane(doc, oid, face);
            let clean = doc.object_owned_sketches(oid).is_empty()
                && doc
                    .drawable_objects_on_plane(plane, None)
                    .iter()
                    .all(|&o| o == oid)
                && !free_drawing_on(doc, plane);
            let result = draw_loop(doc, None, oid, face, &loop_path, None);
            assert!(
                !must_accept || result.is_ok(),
                "step {step}: a touching triangle on a convex face was refused: {:?}",
                result.as_ref().err()
            );
            if must_accept
                && clean
                && let Ok((report, _)) = &result
            {
                assert!(
                    report.kept_as_sketch.is_empty(),
                    "step {step}: a touching triangle on a convex face must land as face \
                     topology, not sketch drawing: {report:?}"
                );
            }
        }
        DocOp::TangentCircle {
            obj_sel,
            face_sel,
            side,
            t,
            frac,
            twin,
        } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let obj = doc.object(oid).expect("visible id resolves");
            let Some(face) = obj.faces().keys().nth(face_sel % obj.faces().len()) else {
                return Ok(true);
            };
            let boundary: Vec<Point3> = obj.loop_positions(obj.faces()[face].outer_loop).collect();
            let sides = boundary.len();
            if sides < 3 {
                return Ok(true);
            }
            let normal = obj.faces()[face].plane.normal();
            let i = side % sides;
            let (a, b) = (boundary[i], boundary[(i + 1) % sides]);
            let Ok(along) = (b - a).normalized() else {
                return Ok(true);
            };
            let Ok(inward) = normal.cross(along).normalized() else {
                return Ok(true);
            };
            let touch = a + (b - a) * *t;
            // A radius that keeps the circle off every other side: a
            // fraction of the touch point's clearance to the rest of the
            // boundary along the inward direction, and of the side itself.
            let clearance = (0..sides)
                .filter(|&k| k != i)
                .map(|k| {
                    let (p, q) = (boundary[k], boundary[(k + 1) % sides]);
                    let d = q - p;
                    let len2 = d.length_squared();
                    let u = ((touch - p).dot(d) / len2).clamp(0.0, 1.0);
                    (touch - (p + d * u)).length()
                })
                .fold(f64::INFINITY, f64::min)
                .min((b - a).length() * (*t).min(1.0 - *t));
            let r = 0.4 * frac * clearance;
            if r.is_nan() || r <= 1e-4 {
                return Ok(true);
            }
            let center = touch + inward * r;
            let circle = |c: Point3, rad: f64, start: Vec3| -> Vec<Point3> {
                let side_dir = normal.cross(start);
                (0..24)
                    .map(|k| {
                        let ang = std::f64::consts::TAU * k as f64 / 24.0;
                        c + start * (rad * ang.cos()) + side_dir * (rad * ang.sin())
                    })
                    .collect()
            };
            // Vertex 0 at the touch point, exactly.
            let mut ring = circle(center, r, inward * -1.0);
            ring[0] = touch;
            let g = kernel::CurveGeom { center, radius: r };
            let must_accept = obj.faces()[face].inner_loops.is_empty()
                && (0..sides).all(|k| {
                    let (p, q, c) = (
                        boundary[k],
                        boundary[(k + 1) % sides],
                        boundary[(k + 2) % sides],
                    );
                    let (e0, e1) = (q - p, c - q);
                    e0.length() >= 1e-3
                        && e0.cross(e1).dot(normal) > 1e-3 * e0.length() * e1.length()
                });
            let plane = face_plane(doc, oid, face);
            let clean = doc.object_owned_sketches(oid).is_empty()
                && doc
                    .drawable_objects_on_plane(plane, None)
                    .iter()
                    .all(|&o| o == oid)
                && !free_drawing_on(doc, plane);
            let result = draw_loop(doc, None, oid, face, &ring, Some(g));
            assert!(
                !must_accept || result.is_ok(),
                "step {step}: a tangent circle on a convex face was refused: {:?}",
                result.as_ref().err()
            );
            if must_accept && clean {
                let (report, _) = result.as_ref().expect("accepted");
                assert!(
                    report.kept_as_sketch.is_empty(),
                    "step {step}: a tangent circle on a convex face must land as face \
                     topology: {report:?}"
                );
                let obj = doc.object(oid).expect("live");
                let fplane = obj.faces().values().find(|f| {
                    (f.plane.normal() - normal).length() < 1e-9
                        && f.plane.signed_distance(touch).abs() < 1e-9
                });
                assert!(fplane.is_some(), "step {step}: the face's plane survives");
                if *twin {
                    // A second circle touching the first at its far point,
                    // outside it, when it fits.
                    let far = ring[12];
                    let r2 = 0.25 * r;
                    let c2 = far + inward * r2;
                    let inside =
                        |p: Point3| obj.faces().keys().any(|f| obj.face_contains_point(f, p));
                    let mut ring2 = circle(c2, r2, inward * -1.0);
                    ring2[0] = far;
                    if ring2.iter().skip(1).all(|&p| inside(p)) {
                        let seed = obj
                            .faces()
                            .keys()
                            .find(|&f| obj.face_contains_point(f, c2 + inward * (0.5 * r2)))
                            .unwrap_or(face);
                        let g2 = kernel::CurveGeom {
                            center: c2,
                            radius: r2,
                        };
                        let second = draw_loop(doc, None, oid, seed, &ring2, Some(g2));
                        assert!(
                            second.is_ok(),
                            "step {step}: a circle touching a disc at one point was refused: {:?}",
                            second.err()
                        );
                    }
                }
            }
        }
        DocOp::HoleChord {
            obj_sel,
            face_sel,
            side,
            ts,
            frac,
            merge_back,
        } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let obj = doc.object(oid).expect("visible id resolves");
            let Some(face) = obj.faces().keys().nth(face_sel % obj.faces().len()) else {
                return Ok(true);
            };
            let normal = obj.faces()[face].plane.normal();
            let mut must_accept = false;
            if obj.faces()[face].inner_loops.is_empty() {
                // Only a hole-free rectangle gets the imprint: its centered
                // half-size copy leaves a gap of a quarter of the face's
                // extent on every side, which the cut below stays within.
                let ring: Vec<Point3> = obj.loop_positions(obj.faces()[face].outer_loop).collect();
                let rectangle = ring.len() == 4
                    && (0..4).all(|i| {
                        let e0 = ring[(i + 1) % 4] - ring[i];
                        let e1 = ring[(i + 2) % 4] - ring[(i + 1) % 4];
                        e0.length() >= 1e-3 && e0.dot(e1).abs() <= 1e-9 * e0.length() * e1.length()
                    });
                if !rectangle {
                    return Ok(true);
                }
                let c = ring.iter().fold(Point3::new(0.0, 0.0, 0.0), |acc, p| {
                    Point3::new(acc.x + p.x / 4.0, acc.y + p.y / 4.0, acc.z + p.z / 4.0)
                });
                let loop_path: Vec<Point3> = ring.iter().map(|&p| c + (p - c) * 0.5).collect();
                let imprinted = doc.apply_object_op(
                    oid,
                    KernelOp::SplitFaceInner {
                        face,
                        loop_path,
                        restore: None,
                        curves: Vec::new(),
                        edge_attrs: Vec::new(),
                    },
                );
                assert!(
                    imprinted.is_ok(),
                    "step {step}: a centered half-size imprint on a rectangle was refused: {:?}",
                    imprinted.err()
                );
                must_accept = true;
            }
            let obj = doc.object(oid).expect("visible id resolves");
            let hole = obj.faces()[face].inner_loops[0];
            let ring: Vec<Point3> = obj.loop_positions(hole).collect();
            let n = ring.len();
            let k = side % n;
            let (a, b) = (ring[k], ring[(k + 1) % n]);
            let shortest = (0..n)
                .map(|i| (ring[(i + 1) % n] - ring[i]).length())
                .fold(f64::INFINITY, f64::min);
            // Hole half-edges carry the face on their left.
            let Ok(out) = normal.cross(b - a).normalized() else {
                return Ok(true);
            };
            let d = frac * 0.2 * shortest;
            let (t0, t1) = (ts.0.min(ts.1), ts.0.max(ts.1));
            if t1 - t0 < 0.1 {
                return Ok(true);
            }
            let (pa, pb) = (a + (b - a) * t0, a + (b - a) * t1);
            let path = vec![pa, pa + out * d, pb + out * d, pb];
            let split = doc.apply_object_op(
                oid,
                KernelOp::SplitFace {
                    face,
                    path,
                    restore: None,
                    curves: Vec::new(),
                },
            );
            assert!(
                !must_accept || split.is_ok(),
                "step {step}: a cut around a fresh imprint's edge was refused: {:?}",
                split.as_ref().err()
            );
            if let (Ok((kernel::KernelOpReport::FaceSplit(r), _)), true) = (&split, *merge_back) {
                let faces = doc.object(oid).expect("object").faces().len();
                let merged = doc.apply_object_op(
                    oid,
                    KernelOp::MergeFaces {
                        edge: r.new_edges[0],
                    },
                );
                assert!(
                    merged.is_ok(),
                    "step {step}: merging a hole-anchored cut back was refused: {:?}",
                    merged.err()
                );
                assert_eq!(
                    doc.object(oid).expect("object").faces().len(),
                    faces - 1,
                    "step {step}: the merge restores the face count"
                );
            }
        }
        DocOp::BridgeLoop {
            obj_sel,
            face_sel,
            ts,
            frac,
            circle,
        } => {
            let Some((oid, frame)) = two_imprints(doc, *obj_sel, *face_sel, step) else {
                return Ok(true);
            };
            let at = |x: f64, y: f64| frame.0 + frame.1 * x + frame.2 * y;
            let (lu, lv) = (frame.1.length(), frame.2.length());
            let (loop_path, curve) = if *circle {
                // Reaching 0.1–0.2 of the width past the gap's edges, no
                // farther than a quarter of the height up or down.
                let r = ((0.1 + 0.1 * (0.2 + 0.8 * frac)) * lu).min(0.24 * lv);
                if r <= 0.11 * lu {
                    return Ok(true);
                }
                let m = at(0.5, 0.5);
                let (eu, ev) = (frame.1 / lu, frame.2 / lv);
                let ring: Vec<Point3> = (0..24)
                    .map(|i| {
                        let t = std::f64::consts::PI / 24.0
                            + std::f64::consts::TAU * f64::from(i) / 24.0;
                        m + eu * (r * t.cos()) + ev * (r * t.sin())
                    })
                    .collect();
                (
                    ring,
                    Some(kernel::CurveGeom {
                        center: m,
                        radius: r,
                    }),
                )
            } else {
                let (y0, y1) = (0.3 + 0.15 * ts.0, 0.55 + 0.15 * ts.1);
                (
                    vec![at(0.3, y0), at(0.7, y0), at(0.7, y1), at(0.3, y1)],
                    None,
                )
            };
            let obj = doc.object(oid).expect("visible id resolves");
            let seed = obj
                .faces()
                .keys()
                .find(|&f| obj.face_contains_point(f, at(0.05, 0.05)))
                .expect("the face around the imprints");
            let plane = face_plane(doc, oid, seed);
            let faces_before = group_faces(doc.object(oid).expect("object"), &plane).len();
            // Earlier owned drawing the loop crosses joins the arrangement
            // (more faces), and a coincident duplicate of the object holding
            // the same imprints may take the new lines instead (either holds
            // them; `coincident_objects_keep_their_own_imprints` pins the
            // single-owner case), so the exact count is pinned only without
            // them.
            let clean = doc.object_owned_sketches(oid).is_empty()
                && doc
                    .drawable_objects_on_plane(plane, None)
                    .iter()
                    .all(|&o| o == oid)
                && !free_drawing_on(doc, plane);
            let result = draw_loop(doc, None, oid, seed, &loop_path, curve);
            match &result {
                Ok((report, _)) => {
                    assert!(
                        !clean || report.kept_as_sketch.is_empty(),
                        "step {step}: the bridging loop must land as topology: {report:?}"
                    );
                    // A cut in each imprint and two bridges: the face around
                    // the imprints and the two imprints become six faces.
                    let faces = group_faces(doc.object(oid).expect("object"), &plane).len();
                    if clean {
                        assert_eq!(
                            faces,
                            faces_before + 3,
                            "step {step}: a cut in each imprint and two bridges"
                        );
                    }
                }
                Err(e) => {
                    panic!("step {step}: a loop across two fresh imprints was refused: {e:?}")
                }
            }
        }
        DocOp::MultiRunMerge {
            obj_sel,
            face_sel,
            ts,
            top,
        } => {
            let Some((oid, frame)) = two_imprints(doc, *obj_sel, *face_sel, step) else {
                return Ok(true);
            };
            let at = |x: f64, y: f64| frame.0 + frame.1 * x + frame.2 * y;
            let (y0, y1) = (0.3 + 0.15 * ts.0, 0.55 + 0.15 * ts.1);
            let strip = vec![at(0.4, y0), at(0.6, y0), at(0.6, y1), at(0.4, y1)];
            let obj = doc.object(oid).expect("visible id resolves");
            let seed = obj
                .faces()
                .keys()
                .find(|&f| obj.face_contains_point(f, at(0.05, 0.05)))
                .expect("the face around the imprints");
            // Owned drawing the strip crosses, or a coincident duplicate of
            // the object holding the same imprints, changes where the strip's
            // lines land; the merge below is pinned only for the clean case.
            let plane = face_plane(doc, oid, seed);
            let clean = doc.object_owned_sketches(oid).is_empty()
                && doc
                    .drawable_objects_on_plane(plane, None)
                    .iter()
                    .all(|&o| o == oid)
                && !free_drawing_on(doc, plane);
            let drawn = draw_loop(doc, None, oid, seed, &strip, None);
            assert!(
                drawn.is_ok(),
                "step {step}: a strip flush between two fresh imprints was refused: {:?}",
                drawn.err()
            );
            if !clean {
                return Ok(true);
            }
            let y = if *top { y1 } else { y0 };
            let (p, q) = (at(0.4, y), at(0.6, y));
            let obj = doc.object(oid).expect("visible id resolves");
            let edge = obj
                .edges()
                .keys()
                .find(|&e| {
                    obj.edge_endpoints(e).is_some_and(|(a, b)| {
                        (a.approx_eq(p, 1e-9) && b.approx_eq(q, 1e-9))
                            || (a.approx_eq(q, 1e-9) && b.approx_eq(p, 1e-9))
                    })
                })
                .expect("the strip's line");
            let merged = doc.apply_object_op(oid, KernelOp::MergeFaces { edge });
            assert!(
                merged.is_ok(),
                "step {step}: deleting the strip's line (two runs) was refused: {:?}",
                merged.err()
            );
            let obj = doc.object(oid).expect("visible id resolves");
            let around = obj
                .faces()
                .keys()
                .find(|&f| obj.face_contains_point(f, at(0.05, 0.05)))
                .expect("the face around the imprints");
            assert_eq!(
                obj.faces()[around].inner_loops.len(),
                2,
                "step {step}: the hole split back in two"
            );
        }
        DocOp::CrossingLoop {
            obj_sel,
            face_sel,
            side,
            ts,
            frac,
            circle,
            from_inside,
        } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let obj = doc.object(oid).expect("visible id resolves");
            let Some(face) = obj.faces().keys().nth(face_sel % obj.faces().len()) else {
                return Ok(true);
            };
            let normal = obj.faces()[face].plane.normal();
            let mut must_accept = false;
            if obj.faces()[face].inner_loops.is_empty() {
                // Only a hole-free rectangle gets the imprint (see
                // `HoleChord`): the loop below stays within a fifth of the
                // imprint's shortest side of the side it crosses, inside
                // both the imprint and the quarter-extent gap around it.
                let ring: Vec<Point3> = obj.loop_positions(obj.faces()[face].outer_loop).collect();
                let rectangle = ring.len() == 4
                    && (0..4).all(|i| {
                        let e0 = ring[(i + 1) % 4] - ring[i];
                        let e1 = ring[(i + 2) % 4] - ring[(i + 1) % 4];
                        e0.length() >= 1e-3 && e0.dot(e1).abs() <= 1e-9 * e0.length() * e1.length()
                    });
                if !rectangle {
                    return Ok(true);
                }
                let c = ring.iter().fold(Point3::new(0.0, 0.0, 0.0), |acc, p| {
                    Point3::new(acc.x + p.x / 4.0, acc.y + p.y / 4.0, acc.z + p.z / 4.0)
                });
                let loop_path: Vec<Point3> = ring.iter().map(|&p| c + (p - c) * 0.5).collect();
                let imprinted = doc.apply_object_op(
                    oid,
                    KernelOp::SplitFaceInner {
                        face,
                        loop_path,
                        restore: None,
                        curves: Vec::new(),
                        edge_attrs: Vec::new(),
                    },
                );
                assert!(
                    imprinted.is_ok(),
                    "step {step}: a centered half-size imprint on a rectangle was refused: {:?}",
                    imprinted.err()
                );
                must_accept = true;
            }
            let obj = doc.object(oid).expect("visible id resolves");
            let hole = obj.faces()[face].inner_loops[0];
            let hole_hes: Vec<_> = obj.loop_half_edges(hole).collect();
            let ring: Vec<Point3> = obj.loop_positions(hole).collect();
            let n = ring.len();
            let k = side % n;
            let (a, b) = (ring[k], ring[(k + 1) % n]);
            let shortest = (0..n)
                .map(|i| (ring[(i + 1) % n] - ring[i]).length())
                .fold(f64::INFINITY, f64::min);
            // Hole half-edges carry the face on their left.
            let (Ok(out), Ok(along)) = (normal.cross(b - a).normalized(), (b - a).normalized())
            else {
                return Ok(true);
            };
            let (t0, t1) = (0.1 + 0.8 * ts.0.min(ts.1), 0.1 + 0.8 * ts.0.max(ts.1));
            if t1 - t0 < 0.1 {
                return Ok(true);
            }
            let d = frac * 0.2 * shortest;
            let (loop_path, curve) = if *circle {
                let tm = (t0 + t1) / 2.0;
                let r = d.min(0.9 * tm.min(1.0 - tm) * (b - a).length());
                let m = a + (b - a) * tm;
                let ring: Vec<Point3> = (0..24)
                    .map(|i| {
                        let t = std::f64::consts::PI / 24.0
                            + std::f64::consts::TAU * f64::from(i) / 24.0;
                        m + along * (r * t.cos()) + out * (r * t.sin())
                    })
                    .collect();
                (
                    ring,
                    Some(kernel::CurveGeom {
                        center: m,
                        radius: r,
                    }),
                )
            } else {
                let (pa, pb) = (a + (b - a) * t0, a + (b - a) * t1);
                (
                    vec![pa - out * d, pb - out * d, pb + out * d, pa + out * d],
                    None,
                )
            };
            let seed = if *from_inside {
                // The imprint: the face across the hole's first edge.
                let Some(t) = obj.half_edges()[hole_hes[0]].twin else {
                    return Ok(true);
                };
                obj.loops()[obj.half_edges()[t].loop_id].face
            } else {
                face
            };
            let plane = face_plane(doc, oid, seed);
            let faces_before = group_faces(doc.object(oid).expect("object"), &plane).len();
            let clean = doc.object_owned_sketches(oid).is_empty()
                && doc
                    .drawable_objects_on_plane(plane, None)
                    .iter()
                    .all(|&o| o == oid)
                && !free_drawing_on(doc, plane);
            let result = draw_loop(doc, None, oid, seed, &loop_path, curve);
            assert!(
                !must_accept || result.is_ok(),
                "step {step}: a loop across a fresh imprint's side was refused: {:?}",
                result.as_ref().err()
            );
            if must_accept && let Ok((report, _)) = &result {
                assert!(
                    !clean || report.kept_as_sketch.is_empty(),
                    "step {step}: the crossing loop must land as topology: {report:?}"
                );
                // One cut on each side of the imprint's side: two new faces.
                let faces = group_faces(doc.object(oid).expect("object"), &plane).len();
                if clean {
                    assert_eq!(
                        faces,
                        faces_before + 2,
                        "step {step}: one cut on each side of the imprint's side"
                    );
                }
            }
        }
        DocOp::MergeCoplanar {
            obj_sel,
            edge_sel,
            from_features,
        } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            if *from_features {
                // Every drawing unit made only of this object's face
                // topology, over every plane it carries drawing on.
                let mut candidates = Vec::new();
                for plane in doc.object_drawing_planes(oid) {
                    let objs = {
                        let mut v = doc.drawable_objects_on_plane(plane, None);
                        if !v.contains(&oid) {
                            v.push(oid);
                        }
                        v
                    };
                    let Ok(pu) = doc.plane_units(plane, None, &objs) else {
                        continue;
                    };
                    for u in pu.units {
                        if u.edges
                            .iter()
                            .all(|e| e.store == kernel::DraftStore::Topology(oid))
                        {
                            candidates.push((plane, objs.clone(), u));
                        }
                    }
                }
                if candidates.is_empty() {
                    return Ok(true);
                }
                let (plane, objs, unit) = candidates[*edge_sel % candidates.len()].clone();
                let result = doc.draft(plane, None, &objs, "Fuzz erase unit", |sk| {
                    for e in &unit.edges {
                        let id = sk
                            .edge_at_positions(e.a, e.b)
                            .expect("a unit edge in the lift");
                        sk.remove_edge(id)?;
                    }
                    Ok(())
                });
                assert!(
                    result.is_ok(),
                    "step {step}: a topology drawing unit did not erase: {:?}",
                    result.as_ref().err()
                );
                let obj = doc.object(oid).expect("object");
                assert!(
                    no_pinch(obj, &plane),
                    "step {step}: an erase committed a loop visiting a vertex twice"
                );
                return Ok(true);
            }
            let obj = doc.object(oid).expect("visible id resolves");
            let candidates: Vec<kernel::EdgeId> = {
                obj.edges()
                    .iter()
                    .filter_map(|(eid, e)| {
                        let t = e.twin_half_edge?;
                        let la = obj.half_edges()[e.half_edge].loop_id;
                        let lb = obj.half_edges()[t].loop_id;
                        let (fa, fb) = (obj.loops()[la].face, obj.loops()[lb].face);
                        let coplanar = obj.faces()[fa]
                            .plane
                            .normal()
                            .dot(obj.faces()[fb].plane.normal())
                            > 0.999;
                        (fa != fb && coplanar).then_some(eid)
                    })
                    .collect()
            };
            let Some(edge) = nth(&candidates, *edge_sel) else {
                return Ok(true);
            };
            let result = doc.apply_object_op(oid, KernelOp::MergeFaces { edge });
            // Refusals (an imprint's whole outline, faces that would pinch)
            // are typed answers. A committed merge never pinches its face.
            if let Ok((kernel::KernelOpReport::FaceMerge(r), _)) = &result {
                let obj = doc.object(oid).expect("object");
                let face = &obj.faces()[r.merged_face];
                for l in std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied()) {
                    let vs: Vec<_> = obj
                        .loop_half_edges(l)
                        .map(|h| obj.half_edges()[h].origin)
                        .collect();
                    assert!(
                        (0..vs.len()).all(|i| !vs[i + 1..].contains(&vs[i])),
                        "step {step}: a merge committed a loop visiting a vertex twice"
                    );
                }
            }
        }
        DocOp::Boolean { kind, a_sel, b_sel } => {
            let ids = doc.visible_object_ids();
            if ids.len() < 2 {
                return Ok(true);
            }
            // Two DISTINCT islands: b is a nonzero cyclic step from a.
            let ia = a_sel % ids.len();
            let ib = (ia + 1 + b_sel % (ids.len() - 1)) % ids.len();
            let (a, b) = (ids[ia], ids[ib]);
            let op = match kind {
                0 => BooleanOp::Union,
                1 => BooleanOp::Subtract,
                _ => BooleanOp::Intersect,
            };
            let _ = doc.boolean(op, a, b);
        }
        DocOp::Translate { obj_sel, offset } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let t = Transform::translation(Vec3::new(offset.0, offset.1, offset.2));
            let _ = doc.transform_object(oid, &t);
        }
        DocOp::Duplicate { obj_sel, offset } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let t = Transform::translation(Vec3::new(offset.0, offset.1, offset.2));
            let _ = doc.duplicate_node(NodeId::Object(oid), &t, kernel::LinkPolicy::Independent);
        }
        DocOp::DuplicateLinked { obj_sel, offset } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let t = Transform::translation(Vec3::new(offset.0, offset.1, offset.2));
            let _ = doc.duplicate_node(NodeId::Object(oid), &t, kernel::LinkPolicy::Linked);
        }
        DocOp::OwnSketch {
            obj_sel,
            face_sel,
            turn,
        } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let Some(tri) = interior_triangle(doc.object(oid).expect("visible"), *face_sel, *turn)
            else {
                return Ok(true);
            };
            let Ok(plane) = Plane::from_point_normal(tri.1, tri.2) else {
                return Ok(true);
            };
            let sid = doc.add_sketch(plane);
            if doc.begin_sketch_gesture(sid).is_err() {
                return Ok(true);
            }
            let mut ok = true;
            {
                let sk = doc.sketch_mut(sid).expect("just created");
                for k in 0..3 {
                    if sk.add_segment(tri.0[k], tri.0[(k + 1) % 3]).is_err() {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                let _ = doc.end_sketch_gesture(sid);
                let _ = doc.set_sketch_object_owner(sid, Some(oid));
            } else {
                doc.cancel_sketch_gesture();
            }
        }
        DocOp::NonUniformScale { obj_sel, factors } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let t = Transform::scale(Vec3::new(factors.0, factors.1, factors.2));
            let _ = doc.transform_object(oid, &t);
        }
        DocOp::Delete { node_sel } => {
            let Some(node) = nth(&doc.top_level_nodes(), *node_sel) else {
                return Ok(true);
            };
            let _ = doc.delete_node(node);
        }
        DocOp::Group { count } => {
            let nodes = doc.top_level_nodes();
            if nodes.len() < 2 {
                return Ok(true);
            }
            let members: Vec<NodeId> = nodes.into_iter().take(*count).collect();
            let _ = doc.group_nodes(&members);
        }
        DocOp::Ungroup { group_sel } => {
            let Some(gid) = nth(&doc.group_ids(), *group_sel) else {
                return Ok(true);
            };
            let _ = doc.ungroup(gid);
        }
        DocOp::MakeComponent { node_sel } => {
            let Some(node) = nth(&doc.top_level_nodes(), *node_sel) else {
                return Ok(true);
            };
            let _ = doc.make_component(&[node]);
        }
        DocOp::PlaceInstance {
            comp_sel,
            axis,
            angle,
            scale,
            offset,
        } => {
            let Some(cid) = nth(&doc.component_ids(), *comp_sel) else {
                return Ok(true);
            };
            let Ok(rotate) = Transform::rotation(Vec3::new(axis.0, axis.1, axis.2), *angle) else {
                return Ok(true);
            };
            let t = rotate
                .then(&Transform::scale(*scale))
                .then(&Transform::translation(Vec3::new(
                    offset.0, offset.1, offset.2,
                )));
            let _ = doc.place_instance(cid, t);
        }
        DocOp::ExplodeInstance { inst_sel } => {
            let Some(iid) = nth(&doc.instance_ids(), *inst_sel) else {
                return Ok(true);
            };
            let _ = doc.explode_instance(iid);
        }
        DocOp::DefOp {
            comp_sel,
            member_sel,
            face_sel,
            distance,
        } => {
            let Some(cid) = nth(&doc.component_ids(), *comp_sel) else {
                return Ok(true);
            };
            // A member id can be hidden — an undone `DrawAndExtrudeInInstance`
            // birth, or a `DeleteDefMember` — while staying listed (phase K1);
            // select only among the LIVE ones.
            let live: Vec<ObjectId> = doc
                .def_members(cid)
                .expect("live component resolves")
                .into_iter()
                .filter(|&o| doc.object(o).is_some())
                .collect();
            let Some(oid) = nth(&live, *member_sel) else {
                return Ok(true);
            };
            let obj = doc.object(oid).expect("member id resolves");
            let Some(face) = obj.faces().keys().nth(face_sel % obj.faces().len()) else {
                return Ok(true);
            };
            let _ = doc.apply_def_op(
                cid,
                oid,
                KernelOp::PushPull {
                    face,
                    distance: *distance,
                },
            );
        }
        DocOp::DrawAndExtrudeInInstance { inst_sel, distance } => {
            let Some(iid) = nth(&doc.instance_ids(), *inst_sel) else {
                return Ok(true);
            };
            let Some(pose) = doc.instance_pose(iid) else {
                return Ok(true);
            };
            let ground =
                Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
                    .expect("unit normal");
            let Ok((sid, _)) = doc.begin_sketch_on_plane_in_instance(iid, ground) else {
                return Ok(true);
            };
            let Ok(pose_inv) = pose.inverse() else {
                return Ok(true);
            };
            let corners = [
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ]
            .map(|p| pose_inv.apply_point(p));
            {
                let sk = doc.sketch_mut(sid).expect("just created");
                for k in 0..4 {
                    if sk.add_segment(corners[k], corners[(k + 1) % 4]).is_err() {
                        return Ok(true);
                    }
                }
            }
            let Some(region) = doc.sketch(sid).and_then(|s| s.regions().keys().next()) else {
                return Ok(true);
            };
            let _ = doc.extrude_region_in_instance(iid, sid, region, *distance);
        }
        DocOp::DrawInInstance { inst_sel } => {
            let Some(iid) = nth(&doc.instance_ids(), *inst_sel) else {
                return Ok(true);
            };
            let Some(pose) = doc.instance_pose(iid) else {
                return Ok(true);
            };
            let ground =
                Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
                    .expect("unit normal");
            let Ok((sid, _)) = doc.begin_sketch_on_plane_in_instance(iid, ground) else {
                return Ok(true);
            };
            let Ok(pose_inv) = pose.inverse() else {
                return Ok(true);
            };
            let corners = [
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ]
            .map(|p| pose_inv.apply_point(p));
            if doc.begin_sketch_gesture(sid).is_err() {
                return Ok(true);
            }
            let mut ok = true;
            {
                let sk = doc.sketch_mut(sid).expect("just created");
                for k in 0..4 {
                    if sk.add_segment(corners[k], corners[(k + 1) % 4]).is_err() {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                let _ = doc.end_sketch_gesture(sid);
            } else {
                // Leave no half-drawn, ungestured debris behind.
                doc.cancel_sketch_gesture();
            }
        }
        DocOp::DeleteDefMember {
            comp_sel,
            member_sel,
        } => {
            let Some(cid) = nth(&doc.component_ids(), *comp_sel) else {
                return Ok(true);
            };
            let live: Vec<ObjectId> = doc
                .def_members(cid)
                .expect("live component resolves")
                .into_iter()
                .filter(|&o| doc.object(o).is_some())
                .collect();
            let Some(oid) = nth(&live, *member_sel) else {
                return Ok(true);
            };
            let _ = doc.delete_def_member(cid, oid);
        }
        DocOp::MakeUnique { inst_sel } => {
            let Some(iid) = nth(&doc.instance_ids(), *inst_sel) else {
                return Ok(true);
            };
            let _ = doc.make_unique(iid);
        }
        DocOp::AddGuideLine { origin, direction } => {
            let origin = Point3::new(origin.0, origin.1, origin.2);
            let direction = Vec3::new(direction.0, direction.1, direction.2);
            let _ = doc.add_guide_line(origin, direction);
        }
        DocOp::MoveGuide {
            guide_sel,
            axis,
            angle,
            offset,
        } => {
            let Some(gid) = nth(&doc.guide_ids(), *guide_sel) else {
                return Ok(true);
            };
            let Ok(rot) = Transform::rotation(Vec3::new(axis.0, axis.1, axis.2), *angle) else {
                return Ok(true);
            };
            let pose = rot.then(&Transform::translation(Vec3::new(
                offset.0, offset.1, offset.2,
            )));
            let _ = doc.transform_guide(gid, &pose);
        }
        DocOp::CopyGuide {
            guide_sel,
            offset,
            count,
        } => {
            let Some(gid) = nth(&doc.guide_ids(), *guide_sel) else {
                return Ok(true);
            };
            let step = Transform::translation(Vec3::new(offset.0, offset.1, offset.2));
            let count = std::num::NonZeroU32::new(*count).expect("strategy draws 1..4");
            let _ = doc.duplicate_guide_array(gid, &step, count);
        }
        DocOp::BooleanInComponent {
            comp_sel,
            kind,
            a_sel,
            b_sel,
        } => {
            let Some(cid) = nth(&doc.component_ids(), *comp_sel) else {
                return Ok(true);
            };
            let live: Vec<ObjectId> = doc
                .def_members(cid)
                .expect("live component resolves")
                .into_iter()
                .filter(|&o| doc.object(o).is_some())
                .collect();
            let (Some(a), Some(b)) = (nth(&live, *a_sel), nth(&live, *b_sel)) else {
                return Ok(true);
            };
            let op = match kind {
                0 => BooleanOp::Union,
                1 => BooleanOp::Subtract,
                _ => BooleanOp::Intersect,
            };
            let _ = doc.boolean_in_component(cid, a, b, op);
        }
        DocOp::SliceDefMember {
            inst_sel,
            member_sel,
            origin,
            direction,
        } => {
            let Some(iid) = nth(&doc.instance_ids(), *inst_sel) else {
                return Ok(true);
            };
            let Some(cid) = doc.instance_def(iid) else {
                return Ok(true);
            };
            let live: Vec<ObjectId> = doc
                .def_members(cid)
                .expect("live component resolves")
                .into_iter()
                .filter(|&o| doc.object(o).is_some())
                .collect();
            let Some(oid) = nth(&live, *member_sel) else {
                return Ok(true);
            };
            let point = Point3::new(origin.0, origin.1, origin.2);
            let normal = Vec3::new(direction.0, direction.1, direction.2);
            let Ok(plane) = Plane::from_point_normal(point, normal) else {
                return Ok(true);
            };
            let _ = doc.slice_def_member(iid, oid, &plane);
        }
        DocOp::TransformDefMember {
            inst_sel,
            member_sel,
            axis,
            angle,
            scale,
            offset,
        } => {
            let Some(iid) = nth(&doc.instance_ids(), *inst_sel) else {
                return Ok(true);
            };
            let Some(cid) = doc.instance_def(iid) else {
                return Ok(true);
            };
            let live: Vec<ObjectId> = doc
                .def_members(cid)
                .expect("live component resolves")
                .into_iter()
                .filter(|&o| doc.object(o).is_some())
                .collect();
            let Some(oid) = nth(&live, *member_sel) else {
                return Ok(true);
            };
            let Ok(rotate) = Transform::rotation(Vec3::new(axis.0, axis.1, axis.2), *angle) else {
                return Ok(true);
            };
            let t = rotate
                .then(&Transform::scale(*scale))
                .then(&Transform::translation(Vec3::new(
                    offset.0, offset.1, offset.2,
                )));
            let _ = doc.transform_def_member(iid, oid, &t);
        }
        DocOp::FollowMeAroundMemberFaceInInstance {
            inst_sel,
            member_sel,
            face_sel,
            stop_len,
        } => {
            let Some(iid) = nth(&doc.instance_ids(), *inst_sel) else {
                return Ok(true);
            };
            let Some(cid) = doc.instance_def(iid) else {
                return Ok(true);
            };
            let live: Vec<ObjectId> = doc
                .def_members(cid)
                .expect("live component resolves")
                .into_iter()
                .filter(|&o| doc.object(o).is_some())
                .collect();
            let Some(path_obj) = nth(&live, *member_sel) else {
                return Ok(true);
            };
            let obj = doc.object(path_obj).expect("member id resolves");
            let Some(face) = obj.faces().keys().nth(face_sel % obj.faces().len()) else {
                return Ok(true);
            };
            // A small fixed profile on a vertical plane through the instance
            // (the same shape `DrawAndExtrudeInInstance` draws, reused here
            // as a Follow Me profile rather than a straight extrusion).
            let plane = Plane::from_polygon(&[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
                Point3::new(0.0, 0.0, 1.0),
            ])
            .expect("unit normal");
            let Ok((sid, _)) = doc.begin_sketch_on_plane_in_instance(iid, plane) else {
                return Ok(true);
            };
            let corners = [
                Point3::new(0.0, -0.3, -0.3),
                Point3::new(0.0, 0.3, -0.3),
                Point3::new(0.0, 0.3, 0.3),
                Point3::new(0.0, -0.3, 0.3),
            ];
            {
                let sk = doc.sketch_mut(sid).expect("just created");
                for k in 0..4 {
                    if sk.add_segment(corners[k], corners[(k + 1) % 4]).is_err() {
                        return Ok(true);
                    }
                }
            }
            let Some(region) = doc.sketch(sid).and_then(|s| s.regions().keys().next()) else {
                return Ok(true);
            };
            let path = kernel::FollowMePath::FaceLoop {
                object: path_obj,
                face,
            };
            let _ = doc.follow_me_in_instance(iid, sid, region, &path, *stop_len);
        }
        DocOp::PlaceText { x, y, depth } => {
            let _ = place_glyph(doc, *x, *y, *depth);
        }
        DocOp::Rescale { factor } => {
            // Always a valid (finite, positive) factor by construction
            // (`arb_doc_op`'s `0.2..5.0` range), so this cannot refuse.
            doc.rescale_document(*factor)
                .expect("rescale never refuses a valid factor");
        }
        DocOp::RebaseDefinition {
            comp_sel,
            origin,
            axis,
            angle,
        } => {
            let Some(cid) = nth(&doc.component_ids(), *comp_sel) else {
                return Ok(true);
            };
            let unit_axis = Vec3::new(axis.0, axis.1, axis.2)
                .normalized()
                .unwrap_or(Vec3::new(0.0, 0.0, 1.0));
            let rot = Transform::rotation(unit_axis, *angle)
                .expect("a unit axis rotation is never singular");
            let x = rot.apply_vector(Vec3::new(1.0, 0.0, 0.0));
            let y = rot.apply_vector(Vec3::new(0.0, 1.0, 0.0));
            let o = Point3::new(origin.0, origin.1, origin.2);
            match doc.rebase_definition(cid, o, x, y) {
                Ok(_)
                | Err(DocumentError::RebaseDuringSession)
                | Err(DocumentError::UnknownComponent) => {}
                Err(e) => panic!("a rotated frame refused: {e:?}"),
            }
        }
        DocOp::FlipInstance { inst_sel, axis } => {
            let Some(iid) = nth(&doc.instance_ids(), *inst_sel) else {
                return Ok(true);
            };
            let axis = match axis % 3 {
                0 => kernel::Axis::X,
                1 => kernel::Axis::Y,
                _ => kernel::Axis::Z,
            };
            let _ = doc.flip_instance(iid, axis);
        }
        DocOp::SetAxes {
            origin,
            axis,
            angle,
        } => {
            // A pure rotation's image of the identity basis is always
            // orthonormal and right-handed, so this can never refuse
            // (unlike `Rescale`, no arithmetic guard is needed at all).
            let raw_axis = Vec3::new(axis.0, axis.1, axis.2);
            let unit_axis = raw_axis.normalized().unwrap_or(Vec3::new(0.0, 0.0, 1.0));
            let rot = Transform::rotation(unit_axis, *angle)
                .expect("a unit axis rotation is never singular");
            let x = rot.apply_vector(Vec3::new(1.0, 0.0, 0.0));
            let y = rot.apply_vector(Vec3::new(0.0, 1.0, 0.0));
            let o = Point3::new(origin.0, origin.1, origin.2);
            doc.set_axes(o, x, y)
                .expect("a rotated identity basis is always a valid axes frame");
        }
        DocOp::AddLinearDimension {
            obj_sel,
            v_a,
            v_b,
            offset,
        } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let obj = doc.object(oid).expect("visible id resolves");
            let (points, _) = obj.to_polygons();
            if points.len() < 2 {
                return Ok(true);
            }
            let pa = points[v_a % points.len()];
            let pb = points[v_b % points.len()];
            let plane =
                Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
                    .expect("unit normal");
            let _ = doc.add_linear_dimension(
                Anchor {
                    node: Some(NodeId::Object(oid)),
                    point: pa,
                },
                Anchor {
                    node: Some(NodeId::Object(oid)),
                    point: pb,
                },
                Vec3::new(offset.0, offset.1, offset.2),
                plane,
                None,
            );
        }
        DocOp::AddRadialDimension {
            obj_sel,
            v_sel,
            radius,
            leader,
        } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let obj = doc.object(oid).expect("visible id resolves");
            let (points, _) = obj.to_polygons();
            if points.is_empty() {
                return Ok(true);
            }
            // The kernel never cross-validates a captured curve against the
            // object's actual geometry (`Document::add_radial_dimension`'s
            // doc comment) — reuse the anchor point as the synthetic
            // circle's center so every field stays finite and the radius
            // stays positive.
            let anchor_point = points[v_sel % points.len()];
            let plane =
                Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
                    .expect("unit normal");
            let curve = CapturedCurve {
                center: anchor_point,
                radius: *radius,
                plane,
            };
            let _ = doc.add_radial_dimension(
                Anchor {
                    node: Some(NodeId::Object(oid)),
                    point: anchor_point,
                },
                RadialKind::Radius,
                curve,
                Vec3::new(leader.0, leader.1, leader.2),
                None,
            );
        }
        DocOp::AddLeaderText { p } => {
            let _ = doc.add_leader_text(
                Anchor {
                    node: None,
                    point: Point3::new(p.0, p.1, p.2),
                },
                Vec3::ZERO,
                "note".to_string(),
            );
        }
        DocOp::DeleteAnnotation { ann_sel } => {
            let Some(id) = nth(&doc.annotation_ids(), *ann_sel) else {
                return Ok(true);
            };
            let _ = doc.delete_annotation(id);
        }
        DocOp::UpdateAnnotationText { ann_sel, text } => {
            let Some(id) = nth(&doc.annotation_ids(), *ann_sel) else {
                return Ok(true);
            };
            let Some(current) = doc.annotation(id).cloned() else {
                return Ok(true);
            };
            let updated = match current {
                Annotation::LinearDimension {
                    a,
                    b,
                    offset,
                    plane,
                    ..
                } => Annotation::LinearDimension {
                    a,
                    b,
                    offset,
                    plane,
                    text_override: Some(text.clone()),
                },
                Annotation::RadialDimension {
                    anchor,
                    kind,
                    curve,
                    leader_dir,
                    ..
                } => Annotation::RadialDimension {
                    anchor,
                    kind,
                    curve,
                    leader_dir,
                    text_override: Some(text.clone()),
                },
                Annotation::LeaderText { anchor, offset, .. } => Annotation::LeaderText {
                    anchor,
                    offset,
                    text: text.clone(),
                },
            };
            let _ = doc.update_annotation(id, updated);
        }
        DocOp::PaintFace {
            obj_sel,
            face_sel,
            mat_sel,
        } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let obj = doc.object(oid).expect("visible id resolves");
            let Some(face) = obj.faces().keys().nth(face_sel % obj.faces().len()) else {
                return Ok(true);
            };
            let _ = doc.paint_face(oid, face, pick_material(materials, *mat_sel));
        }
        DocOp::SetObjectMaterial { obj_sel, mat_sel } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let _ = doc.set_object_material(oid, pick_material(materials, *mat_sel));
        }
        DocOp::ReplaceMaterial {
            object_scope,
            obj_sel,
            from_sel,
            to_sel,
        } => {
            let scope = if *object_scope {
                let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                    return Ok(true);
                };
                MaterialScope::Object(oid)
            } else {
                MaterialScope::Document
            };
            let from = pick_material(materials, *from_sel);
            let to = pick_material(materials, *to_sel);
            let before = snapshot_materials(doc);
            if doc.replace_material(scope, from, to).is_ok() {
                let after = snapshot_materials(doc);
                // A no-op (from == to, or nothing matched `from`) pushes no
                // undo entry — nothing to round-trip. Otherwise, one local
                // undo/redo bracketing THIS call (nothing else runs in
                // between) must restore/reproduce the snapshot exactly —
                // the atomic-undo contract, fuzzed against random document
                // states/scopes/materials on top of the targeted
                // `document_specs.rs` unit specs.
                if after != before {
                    doc.undo().map_err(|e| {
                        TestCaseError::fail(format!(
                            "step {step}: replace_material's own undo failed: {e}"
                        ))
                    })?;
                    if snapshot_materials(doc) != before {
                        return Err(TestCaseError::fail(format!(
                            "step {step}: replace_material undo did not restore the prior \
                             material state exactly"
                        )));
                    }
                    doc.redo().map_err(|e| {
                        TestCaseError::fail(format!(
                            "step {step}: replace_material's own redo failed: {e}"
                        ))
                    })?;
                    if snapshot_materials(doc) != after {
                        return Err(TestCaseError::fail(format!(
                            "step {step}: replace_material redo did not reproduce the \
                             post-op material state exactly"
                        )));
                    }
                }
            }
        }
        DocOp::SetFaceUvFrame {
            obj_sel,
            face_sel,
            frame_sel,
        } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let obj = doc.object(oid).expect("visible id resolves");
            let Some(face) = obj.faces().keys().nth(face_sel % obj.faces().len()) else {
                return Ok(true);
            };
            let _ = doc.set_face_uv_frame(oid, face, pick_uv_frame(*frame_sel));
        }
        DocOp::ElementAttr {
            obj_sel,
            elem_sel,
            edge,
            op_sel,
        } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(true);
            };
            let obj = doc.object(oid).expect("visible id resolves");
            const NS: &str = "com.example.fuzz";
            let key = ["a", "b"][op_sel % 2];
            let value = if op_sel % 7 == 0 {
                kernel::AttrValue::List(vec![
                    kernel::AttrValue::Float(0.5),
                    kernel::AttrValue::Text("x".into()),
                ])
            } else {
                kernel::AttrValue::Int((op_sel % 3) as i64)
            };
            let (nf, ne) = (obj.faces().len(), obj.edges().len());
            let face = obj.faces().keys().nth(elem_sel % nf.max(1));
            let e = obj.edges().keys().nth(elem_sel % ne.max(1));
            let result = match (*edge, op_sel % 4) {
                (false, 0 | 1) => face.map(|f| doc.face_attr_set(oid, f, NS, key, value)),
                (false, 2) => face.map(|f| doc.face_attr_delete(oid, f, NS, Some(key))),
                (false, _) => face.map(|f| doc.face_attr_delete(oid, f, NS, None)),
                (true, 0 | 1) => e.map(|x| doc.edge_attr_set(oid, x, NS, key, value)),
                (true, 2) => e.map(|x| doc.edge_attr_delete(oid, x, NS, Some(key))),
                (true, _) => e.map(|x| doc.edge_attr_delete(oid, x, NS, None)),
            };
            match result {
                None | Some(Ok(_)) | Some(Err(DocumentError::UnknownAttr)) => {}
                Some(Err(e)) => {
                    return Err(TestCaseError::fail(format!(
                        "step {step}: an attribute write on a live element refused: {e}"
                    )));
                }
            }
        }
        DocOp::RenameMaterial { mat_sel, name } => {
            if let Some(mid) = pick_material(materials, *mat_sel) {
                let _ = doc.set_material_name(mid, name.clone());
            }
        }
        DocOp::DeleteMaterial { mat_sel } => {
            if let Some(mid) = pick_material(materials, *mat_sel) {
                let _ = doc.delete_material(mid);
            }
        }
        DocOp::DeleteDefinition { comp_sel } => {
            let Some(cid) = nth(&doc.component_ids(), *comp_sel) else {
                return Ok(true);
            };
            let _ = doc.delete_definition(cid);
        }
        DocOp::PurgeUnused => {
            let _ = doc.purge_unused();
        }
        DocOp::Reparent {
            node_sel,
            group_sel,
            to_top,
        } => {
            // Any live world node, top-level or nested: a group's members
            // are reachable through `group_members`, so pick from the union.
            let mut nodes: Vec<NodeId> = doc.top_level_nodes();
            for g in doc.group_ids() {
                if let Some(members) = doc.group_members(g) {
                    nodes.extend(members);
                }
            }
            let Some(node) = nth(&nodes, *node_sel) else {
                return Ok(true);
            };
            let parent = if *to_top {
                None
            } else {
                nth(&doc.group_ids(), *group_sel)
            };
            let _ = doc.reparent_nodes(&[node], parent);
        }
        DocOp::DrawRectOnGround { x, y, w, h } => {
            let (x0, y0) = (f64::from(*x) * 0.5, f64::from(*y) * 0.5);
            let (x1, y1) = (x0 + f64::from(*w) * 0.5, y0 + f64::from(*h) * 0.5);
            let c = [
                Point3::new(x0, y0, 0.0),
                Point3::new(x1, y0, 0.0),
                Point3::new(x1, y1, 0.0),
                Point3::new(x0, y1, 0.0),
            ];
            let sid = ground_sketch(doc);
            doc.begin_sketch_gesture(sid).expect("gesture opens");
            for k in 0..4 {
                if let Err(e) = doc
                    .sketch_mut(sid)
                    .expect("ground sketch is live")
                    .add_segment(c[k], c[(k + 1) % 4])
                {
                    doc.cancel_sketch_gesture();
                    return Err(TestCaseError::fail(format!(
                        "step {step}: a lattice rectangle edge was refused: {e:?}"
                    )));
                }
            }
            if let Err(e) = doc.end_sketch_gesture(sid) {
                return Err(TestCaseError::fail(format!(
                    "step {step}: closing a rectangle gesture failed: {e:?}"
                )));
            }
        }
        DocOp::DrawCircleOnGround {
            cx,
            cy,
            r,
            segments,
        } => {
            let center = Point3::new(f64::from(*cx) * 0.5, f64::from(*cy) * 0.5, 0.0);
            let radius = f64::from(*r) * 0.5;
            let at = |i: usize| {
                let a = std::f64::consts::TAU * (i as f64) / (*segments as f64);
                Point3::new(
                    center.x + radius * a.cos(),
                    center.y + radius * a.sin(),
                    0.0,
                )
            };
            let sid = ground_sketch(doc);
            doc.begin_sketch_gesture(sid).expect("gesture opens");
            let mut failure = None;
            {
                let sk = doc.sketch_mut(sid).expect("ground sketch is live");
                match sk.begin_curve_with(kernel::CurveGeom { center, radius }) {
                    Err(e) => failure = Some(e),
                    Ok(_) => {
                        for i in 0..*segments {
                            if let Err(e) = sk.add_segment(at(i), at(i + 1)) {
                                failure = Some(e);
                                break;
                            }
                        }
                        sk.end_curve();
                    }
                }
            }
            if let Some(e) = failure {
                doc.cancel_sketch_gesture();
                return Err(TestCaseError::fail(format!(
                    "step {step}: a circle facet was refused: {e:?}"
                )));
            }
            if let Err(e) = doc.end_sketch_gesture(sid) {
                return Err(TestCaseError::fail(format!(
                    "step {step}: closing a circle gesture failed: {e:?}"
                )));
            }
        }
        DocOp::Draft {
            obj_sel,
            face_sel,
            ground,
            shape,
            a,
            b,
            sel,
            segments,
        } => {
            return apply_draft(
                doc, step, *obj_sel, *face_sel, *ground, *shape, *a, *b, *sel, *segments,
            )
            .map(|()| true);
        }
        DocOp::DrawDetached {
            obj_sel,
            face_sel,
            ground,
            circle,
            a,
            b,
            segments,
        } => {
            apply_draw_detached(
                doc, step, *obj_sel, *face_sel, *ground, *circle, *a, *b, *segments,
            )?;
        }
        DocOp::EditDetached {
            sk_sel,
            mode,
            sel,
            d,
        } => {
            apply_edit_detached(doc, step, *sk_sel, *mode, *sel, *d)?;
        }
        DocOp::ExtrudeDetached {
            sk_sel,
            reg_sel,
            distance,
        } => {
            let Some(s) = nth(&detached_sketch_ids(doc), *sk_sel) else {
                return Ok(true);
            };
            let Ok(units) = doc.detached_sketch_units(s) else {
                return Err(TestCaseError::fail(format!(
                    "step {step}: a detached sketch has no listing"
                )));
            };
            let regions: Vec<_> = units.regions.iter().filter_map(|r| r.extrude).collect();
            let Some((sk, rid)) = nth(&regions, *reg_sel) else {
                return Ok(true);
            };
            prop_assert_eq!(
                sk,
                s,
                "step {}: a detached region extrudes as its own",
                step
            );
            let objects0 = doc.visible_object_ids().len();
            if doc.extrude_region(sk, rid, *distance).is_ok() {
                prop_assert_eq!(
                    doc.visible_object_ids().len(),
                    objects0 + 1,
                    "step {}: extruding a detached region makes one new object",
                    step
                );
                if let Some(now) = doc.sketch(s) {
                    prop_assert!(now.detached(), "step {}: the sketch lost its flag", step);
                }
            }
        }
        DocOp::MoveIsland {
            sk_sel,
            isl_sel,
            dx,
            dy,
        } => {
            let Some(sid) = nth(&free_sketch_ids(doc), *sk_sel) else {
                return Ok(true);
            };
            let ids: Vec<_> = doc.sketch(sid).expect("live").islands().keys().collect();
            let Some(island) = nth(&ids, *isl_sel) else {
                return Ok(true);
            };
            // An in-plane move never refuses on contact: it arranges like a
            // draw (unified-drafting decision 4). Any refusal is a bug.
            if let Err(e) = doc.transform_sketch_island(sid, island, &lattice_translation(*dx, *dy))
            {
                return Err(TestCaseError::fail(format!(
                    "step {step}: an in-plane island move was refused: {e:?}"
                )));
            }
        }
        DocOp::MoveIslands {
            sk_sel,
            a_sel,
            b_sel,
            dx,
            dy,
        } => {
            let Some(sid) = nth(&free_sketch_ids(doc), *sk_sel) else {
                return Ok(true);
            };
            let ids: Vec<_> = doc.sketch(sid).expect("live").islands().keys().collect();
            if ids.len() < 2 {
                return Ok(true);
            }
            // Two DISTINCT islands: b is a nonzero cyclic step from a.
            let ia = a_sel % ids.len();
            let ib = (ia + 1 + b_sel % (ids.len() - 1)) % ids.len();
            let (a, b) = (ids[ia], ids[ib]);
            if let Err(e) =
                doc.transform_sketch_islands(sid, &[a, b], &lattice_translation(*dx, *dy))
            {
                return Err(TestCaseError::fail(format!(
                    "step {step}: an in-plane two-island move was refused: {e:?}"
                )));
            }
        }
        DocOp::RemoveSketchEdge { sk_sel, edge_sel } => {
            let Some(sid) = nth(&free_sketch_ids(doc), *sk_sel) else {
                return Ok(true);
            };
            let ids: Vec<_> = doc.sketch(sid).expect("live").edges().keys().collect();
            let Some(edge) = nth(&ids, *edge_sel) else {
                return Ok(true);
            };
            doc.begin_sketch_gesture(sid).expect("gesture opens");
            let ok = doc.sketch_mut(sid).expect("live").remove_edge(edge).is_ok();
            if ok {
                let _ = doc.end_sketch_gesture(sid);
            } else {
                doc.cancel_sketch_gesture();
            }
        }
        DocOp::MoveSketchVertex {
            sk_sel,
            v_sel,
            mode,
            target_sel,
            dx,
            dy,
        } => {
            let Some(sid) = nth(&free_sketch_ids(doc), *sk_sel) else {
                return Ok(true);
            };
            let sk = doc.sketch(sid).expect("live");
            let verts: Vec<_> = sk.vertices().keys().collect();
            let Some(v) = nth(&verts, *v_sel) else {
                return Ok(true);
            };
            let here = sk.vertices()[v].position;
            let target = match mode {
                0 => nth(&verts, *target_sel)
                    .map(|t| sk.vertices()[t].position)
                    .unwrap_or(here),
                1 => {
                    let edges: Vec<_> = sk.edges().values().copied().collect();
                    nth(&edges, *target_sel)
                        .map(|e| {
                            let (a, b) =
                                (sk.vertices()[e.from].position, sk.vertices()[e.to].position);
                            Point3::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0, (a.z + b.z) / 2.0)
                        })
                        .unwrap_or(here)
                }
                _ => Point3::new(
                    here.x + f64::from(*dx) * 0.5,
                    here.y + f64::from(*dy) * 0.5,
                    here.z,
                ),
            };
            // A drag arranges like a draw; it refuses only structurally: an
            // incident edge collapsing onto its other end, or a region the
            // in-place drag would leave unprofileable (a pinch).
            match doc.move_sketch_vertex(sid, v, target) {
                Ok(_)
                | Err(DocumentError::Sketch(
                    SketchError::DegenerateSegment | SketchError::MalformedRegion,
                )) => {}
                Err(e) => {
                    return Err(TestCaseError::fail(format!(
                        "step {step}: a vertex drag was refused: {e:?}"
                    )));
                }
            }
        }
        DocOp::OffsetRegion {
            sk_sel,
            reg_sel,
            dist_sel,
        } => {
            let Some(sid) = nth(&free_sketch_ids(doc), *sk_sel) else {
                return Ok(true);
            };
            let ids: Vec<_> = doc.sketch(sid).expect("live").regions().keys().collect();
            let Some(region) = nth(&ids, *reg_sel) else {
                return Ok(true);
            };
            const DISTANCES: [f64; 6] = [0.25, -0.25, 0.5, -0.1, 0.1, -0.5];
            let d = DISTANCES[dist_sel % DISTANCES.len()];
            doc.begin_sketch_gesture(sid).expect("gesture opens");
            let ok = doc
                .sketch_mut(sid)
                .expect("live")
                .offset_region(region, d)
                .is_ok();
            if ok {
                let _ = doc.end_sketch_gesture(sid);
            } else {
                doc.cancel_sketch_gesture();
            }
        }
        DocOp::RefacetCurve {
            sk_sel,
            curve_sel,
            segments,
        } => {
            let Some(sid) = nth(&free_sketch_ids(doc), *sk_sel) else {
                return Ok(true);
            };
            let sk = doc.sketch(sid).expect("live");
            let mut curves: Vec<kernel::SketchCurveId> =
                sk.edges().values().filter_map(|e| e.curve).collect();
            curves.sort_unstable();
            curves.dedup();
            curves.retain(|c| sk.curve_geom(*c).is_some());
            let Some(curve) = nth(&curves, *curve_sel) else {
                return Ok(true);
            };
            doc.begin_sketch_gesture(sid).expect("gesture opens");
            let ok = doc
                .sketch_mut(sid)
                .expect("live")
                .refacet_curve(curve, *segments)
                .is_ok();
            if ok {
                let _ = doc.end_sketch_gesture(sid);
            } else {
                doc.cancel_sketch_gesture();
            }
        }
        DocOp::BeginCompound
        | DocOp::EndCompound
        | DocOp::AbortCompound
        | DocOp::CoalescedAttr { .. } => {
            unreachable!("applied by apply_compound_op")
        }
        DocOp::Undo => {
            if doc.can_undo()
                && let Err(e) = doc.undo()
            {
                if !is_known_inverse_guard_gap(doc, &e, false) {
                    return Err(TestCaseError::fail(format!(
                        "step {step}: document undo failed: {e}"
                    )));
                }
                // Tolerated UnbuildPushPull gap: undo failed typed, the
                // document is untouched but its log can no longer unwind past
                // the refused inverse — abandon the round-trip for this case.
                return Ok(false);
            }
        }
        DocOp::Redo => {
            if doc.can_redo()
                && let Err(e) = doc.redo()
            {
                if !is_known_inverse_guard_gap(doc, &e, true) {
                    return Err(TestCaseError::fail(format!(
                        "step {step}: document redo failed: {e}"
                    )));
                }
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// A seed box: lower corner, footprint size, height.
type BoxSeed = ((f64, f64), (f64, f64), f64);

/// One fuzz case: seed boxes, apply `ops` checking every invariant after
/// each, then unwind/replay the whole log twice (see the module docs).
fn run_doc_case(seeds: Vec<BoxSeed>, ops: Vec<DocOp>) -> Result<(), TestCaseError> {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    for ((x, y), (dx, dy), h) in seeds {
        add_box(&mut doc, x, y, dx, dy, h);
    }
    let materials = seed_materials(&mut doc);
    check_doc(&doc, 0, "seed")?;

    if std::env::var("FUZZ_TRACE").is_ok() {
        eprintln!("DOC CASE ops={ops:?}");
    }

    let mut open = Vec::new();
    for (step, op) in ops.iter().enumerate() {
        let detached_before = detached_snapshot(&doc);
        let target = op_target_sketch(&doc, op);
        if !apply_compound_op(&mut doc, step, op, &mut open)?
            && !apply_doc_op(&mut doc, step, op, &materials)?
        {
            return Ok(());
        }
        check_doc(&doc, step, "apply")?;
        check_detached_untouched(&doc, step, op, &detached_before, target)?;
        check_save_and_sketch_owners(&doc, &format!("step {step}"))?;
        // Every committed state reopens: an op that left a face the
        // loader rejects would lose the user's file on the next save.
        check_persistence(&doc, &format!("step {step}"))?;
    }

    // Close what the ops left open, innermost first, so the round-trip
    // below unwinds a history no compound is holding.
    while let Some((token, _, _)) = open.pop() {
        match doc.end_compound(token) {
            Ok(_) | Err(kernel::DocumentError::TransactionHistoryDisturbed) => {}
            Err(e) => return Err(TestCaseError::fail(format!("closing compounds: {e}"))),
        }
    }
    check_persistence(&doc, "post-sequence")?;

    // Unwind the whole document log, replay it, and do both again; the
    // first replay must reproduce what redoing just the steps the ops left
    // undone gives, both ends of the second cycle must reproduce the
    // first's, and every
    // undo/redo must succeed (rule 9), with the single tolerated
    // exception of the `UnbuildPushPull` gap — `Ok(false)` means it fired
    // and the round-trip is abandoned for this case (the document stays
    // valid, but the log can no longer unwind past the refused inverse).
    // Fingerprints, not save bytes: baked-transform and sweep round-trips
    // carry ulp noise, which the fingerprint's quantization absorbs and
    // byte comparison would not.
    let unwind = |doc: &mut Document, label: &str| -> Result<bool, TestCaseError> {
        let mut n = 0usize;
        while doc.can_undo() {
            match doc.undo() {
                Ok(_) => {}
                Err(e) if is_known_inverse_guard_gap(doc, &e, false) => return Ok(false),
                Err(e) => {
                    return Err(TestCaseError::fail(format!("{label}, undo #{n}: {e}")));
                }
            }
            check_doc(doc, n, label)?;
            check_save_and_sketch_owners(doc, &format!("{label} #{n}"))?;
            n += 1;
        }
        Ok(true)
    };
    let replay = |doc: &mut Document, label: &str| -> Result<bool, TestCaseError> {
        let mut n = 0usize;
        while doc.can_redo() {
            match doc.redo() {
                Ok(_) => {}
                Err(e) if is_known_inverse_guard_gap(doc, &e, true) => return Ok(false),
                Err(e) => {
                    return Err(TestCaseError::fail(format!("{label}, redo #{n}: {e}")));
                }
            }
            check_doc(doc, n, label)?;
            check_save_and_sketch_owners(doc, &format!("{label} #{n}"))?;
            n += 1;
        }
        Ok(true)
    };

    // The ops interleave undo and redo, so the state they leave can have
    // steps pending on the redo stack; a full replay redoes those too.
    // Redoing them on a copy gives the state the full cycle must match,
    // without going through a full unwind.
    let mut built = doc.clone();
    if !replay(&mut built, "pending redo")? {
        return Ok(());
    }
    let built_print = doc_fingerprint(&built);
    drop(built);

    if !unwind(&mut doc, "first unwind")? {
        return Ok(());
    }
    let empty_print = doc_fingerprint(&doc);

    if !replay(&mut doc, "first replay")? {
        return Ok(());
    }
    let maximal_print = doc_fingerprint(&doc);
    prop_assert_eq!(
        &maximal_print,
        &built_print,
        "the first full redo did not reproduce the state the ops built"
    );
    check_persistence(&doc, "maximal")?;

    if !unwind(&mut doc, "second unwind")? {
        return Ok(());
    }
    prop_assert_eq!(
        doc_fingerprint(&doc),
        empty_print,
        "second full undo did not reproduce the fully-unwound state"
    );

    if !replay(&mut doc, "second replay")? {
        return Ok(());
    }
    prop_assert_eq!(
        doc_fingerprint(&doc),
        maximal_print,
        "second full redo did not reproduce the maximal state"
    );
    Ok(())
}

/// KB-22, the fuzzer's original seed: a sketch drawn into a definition
/// (by a refused in-instance Follow Me) while its `delete_definition` sat
/// undone on the redo stack made `save()` panic on the redo. The minimized
/// spec lives in `definition_delete_undo_specs.rs`; this replays the exact
/// case under every harness invariant.
#[test]
fn kb22_save_after_redoing_a_definition_delete() {
    let seeds: Vec<BoxSeed> = vec![
        (
            (-5.7950551958729335, -3.9242074461820176),
            (1.9773289823311382, 1.8691345160206931),
            1.127080679912591,
        ),
        (
            (-2.9851796139686115, 0.8923326424736255),
            (1.4053546085447017, 2.05801571414664),
            0.6467165493011736,
        ),
        (
            (-0.7605684166980339, 1.9920919085928452),
            (4.683170220651774, 1.9623960650445467),
            4.109616389142183,
        ),
    ];
    let ops = vec![
        DocOp::Undo,
        DocOp::AddLinearDimension {
            obj_sel: 7004580106481675702,
            v_a: 827558879587809830,
            v_b: 10900233935612383634,
            offset: (-3.1734708095408712, -4.261111886731595, -3.600808611614945),
        },
        DocOp::MakeComponent {
            node_sel: 10689752944661742954,
        },
        DocOp::DeleteDefinition {
            comp_sel: 14005115186812097118,
        },
        DocOp::AddRadialDimension {
            obj_sel: 14451272870395314874,
            v_sel: 4746832276610707069,
            radius: 0.3718003241348433,
            leader: (7.348305143132818, 0.26421970565703307, -1.573646005427876),
        },
        DocOp::PlaceInstance {
            comp_sel: 15399635707428390039,
            axis: (
                0.7764453856001974,
                -0.7897148451646767,
                -0.41544960718106744,
            ),
            angle: 2.4295502668537754,
            scale: Vec3::new(2.4713741962902827, 1.4709511129002013, 2.3926172376292794),
            offset: (5.677597158460211, -7.764147646974772, -3.452170459670012),
        },
        DocOp::Undo,
        DocOp::PlaceInstance {
            comp_sel: 8879482871949261076,
            axis: (
                -0.29419302784396073,
                -0.20032373423046143,
                -0.2077593077523247,
            ),
            angle: 0.5915691299998936,
            scale: Vec3::new(2.3321617982776313, 2.3321617982776313, 2.3321617982776313),
            offset: (-1.6532238151952883, 0.9951831924552207, -2.9806115608068953),
        },
        DocOp::DefOp {
            comp_sel: 11882322038670728699,
            member_sel: 13421546395919311841,
            face_sel: 9582150626024753920,
            distance: 2.4044418853681666,
        },
        DocOp::Undo,
        DocOp::SetFaceUvFrame {
            obj_sel: 5908377362997804141,
            face_sel: 7753915013767089619,
            frame_sel: 5699336088435063196,
        },
        DocOp::FollowMeAroundMemberFaceInInstance {
            inst_sel: 13900433402026004742,
            member_sel: 623422252848888087,
            face_sel: 3873899750457946790,
            stop_len: Some(2.349734431957779),
        },
    ];
    run_doc_case(seeds, ops).expect("KB-22 replay holds every invariant");
}

/// KB-30: two coincident unit boxes intersected, then a sketch owned on the
/// result, a crossing loop, Make Component and a member transform. The
/// validator reported an object-owned sketch lying on no face of its owner
/// (I8).
#[test]
fn kb30_sketch_on_face_after_coincident_intersect() {
    let seeds: Vec<BoxSeed> = vec![((0.0, 0.0), (1.0, 1.0), 0.5), ((0.0, 0.0), (1.0, 1.0), 0.5)];
    let ops = vec![
        DocOp::Boolean {
            kind: 2,
            a_sel: 0,
            b_sel: 0,
        },
        DocOp::OwnSketch {
            obj_sel: 0,
            face_sel: 7825113691772547323,
            turn: 0.0,
        },
        DocOp::CrossingLoop {
            obj_sel: 0,
            face_sel: 3933536894228226167,
            side: 0,
            ts: (0.0, 0.7176536165147582),
            frac: 0.1,
            circle: false,
            from_inside: false,
        },
        DocOp::MakeComponent { node_sel: 0 },
        DocOp::TransformDefMember {
            inst_sel: 0,
            member_sel: 0,
            axis: (0.0, 0.0, -0.7628613740878377),
            angle: 0.0,
            scale: Vec3::new(0.3, 0.3, 0.3),
            offset: (-2.3523967960678642, 0.0, 0.0),
        },
    ];
    run_doc_case(seeds, ops).expect("KB-30 replay holds every invariant");
}

/// KB-34: a circle tangent to a box's edge, then a face split. Redoing the
/// split after a full unwind refused with "cut path crosses itself or
/// grazes the boundary".
#[test]
fn kb34_redo_split_after_tangent_circle() {
    let seeds: Vec<BoxSeed> = vec![
        ((0.0, 0.0), (1.0, 1.0), 0.5),
        ((0.0, 0.0), (1.0, 1.0), 0.5),
        ((0.0, 0.0), (1.0, 1.0), 2.7131479042599187),
    ];
    let ops = vec![
        DocOp::TangentCircle {
            obj_sel: 3078489218597910833,
            face_sel: 15043238798366512228,
            side: 15568592410549646645,
            t: 0.6808869321157451,
            frac: 0.2,
            twin: false,
        },
        DocOp::SplitFace {
            obj_sel: 2386591658481364397,
            face_sel: 11787325478549280132,
            edge_a: 3815206797068229728,
            edge_b: 8241556265330311875,
            ta: 0.6801022240073851,
            tb: 0.3464279946365581,
        },
    ];
    run_doc_case(seeds, ops).expect("KB-34 replay holds every invariant");
}

/// A tangent circle drawn over two imprints and a loop bridging them,
/// clipped by them, then merged into the face around it across its arc:
/// the arc runs through the rim vertex where the hole touches the face's
/// outline. Undoing the merge replayed one cut through that boundary vertex
/// and refused ("path point is not on the face"). The minimized spec lives
/// in `tangent_loop_specs.rs`; this replays the exact case under every
/// harness invariant.
#[test]
fn merge_of_a_clipped_tangent_disc_unwinds() {
    let seeds: Vec<BoxSeed> = vec![
        ((0.0, 0.0), (2.2346752169091917, 1.0), 3.276568943736587),
        ((0.0, 0.0), (1.0, 1.0), 0.5),
        ((0.0, -3.507174267747308), (1.0, 1.0), 0.5),
    ];
    let ops = vec![
        DocOp::SliceDefMember {
            inst_sel: 66360466712461141,
            member_sel: 6596232982742202595,
            origin: (0.25223245092388275, -6.952412369336496, -0.7718551660407071),
            direction: (
                -0.07185658133522942,
                -0.6647107021629067,
                -0.45456653437751626,
            ),
        },
        DocOp::BridgeLoop {
            obj_sel: 1354774533126141447,
            face_sel: 4288230138990121280,
            ts: (0.15190935456390073, 0.8690961128556175),
            frac: 0.963971482904148,
            circle: false,
        },
        DocOp::Duplicate {
            obj_sel: 4165769603963804333,
            offset: (4.536789387423562, 6.760316351437082, -1.1763973855480325),
        },
        DocOp::Undo,
        DocOp::TangentCircle {
            obj_sel: 14941672070845138833,
            face_sel: 3591855991437777439,
            side: 9786374422185185391,
            t: 0.5028176080342001,
            frac: 0.3829567238869083,
            twin: false,
        },
        DocOp::MergeCoplanar {
            obj_sel: 3269655902696552490,
            edge_sel: 5049629487782488163,
            from_features: false,
        },
    ];
    run_doc_case(seeds, ops).expect("the clipped-disc merge unwinds and replays");
}

proptest! {
    /// Random document-op sequences keep every visible object valid, survive
    /// save/load, and the document log unwinds and replays reproducibly.
    #[test]
    fn document_sequences_preserve_invariants_and_roundtrip(
        seeds in proptest::collection::vec(
            ((-6.0..6.0f64, -6.0..6.0f64), (1.0..6.0f64, 1.0..6.0f64), 0.5..5.0f64),
            2..4,
        ),
        ops in proptest::collection::vec(arb_doc_op(), 1..14),
    ) {
        run_doc_case(seeds, ops)?;
    }

    /// World-sketch-focused sequences: longer runs of ground drawing, island
    /// and vertex moves, erasing, offsetting, and re-faceting, interleaved
    /// with undo/redo, over a document that starts with sketch-extruded
    /// boxes. Every invariant of the general harness applies, plus: every
    /// sketch region profiles (or is a pinch) and every save/load/save is
    /// byte-identical.
    #[test]
    fn world_sketch_sequences_preserve_invariants_and_roundtrip(
        seeds in proptest::collection::vec(
            ((-6.0..6.0f64, -6.0..6.0f64), (1.0..6.0f64, 1.0..6.0f64), 0.5..5.0f64),
            0..2,
        ),
        ops in proptest::collection::vec(arb_world_sketch_op(), 2..24),
    ) {
        run_doc_case(seeds, ops)?;
    }
}
