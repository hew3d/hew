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
//!   `state_hash`, at the post-sequence and maximal states;
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
    /// Boolean between two distinct visible objects.
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
    /// Boolean between the `a_sel`-th and `b_sel`-th LIVE members of the
    /// `comp_sel`-th component (`boolean_in_component`, component-edit-
    /// parity.md phase K2) — the shared-geometry combine, replacing both
    /// operands with a new member. `a_sel == b_sel` (same live member) is an
    /// accepted typed no-op (`DegenerateContact`), like the world `Boolean`
    /// op's own `a_sel == b_sel` case.
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
    Undo,
    Redo,
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
    prop_oneof![
        2 => comp().prop_map(|s| Vec3::new(s, s, s)),
        3 => (comp(), comp(), comp()).prop_map(|(x, y, z)| Vec3::new(x, y, z)),
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
        3 => (0u8..3, any::<usize>(), any::<usize>()).prop_map(|(kind, a_sel, b_sel)| {
            DocOp::Boolean { kind, a_sel, b_sel }
        }),
        2 => (any::<usize>(), arb_offset()).prop_map(|(obj_sel, offset)| {
            DocOp::Translate { obj_sel, offset }
        }),
        2 => (any::<usize>(), arb_offset()).prop_map(|(obj_sel, offset)| {
            DocOp::Duplicate { obj_sel, offset }
        }),
        2 => (any::<usize>(), (0.2..3.0f64, 0.2..3.0f64, 0.2..3.0f64)).prop_map(
            |(obj_sel, factors)| DocOp::NonUniformScale { obj_sel, factors }
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
        2 => Just(DocOp::Undo),
        1 => Just(DocOp::Redo),
    ]
}

/// Fixed 3-material palette, seeded once (materials are never removed or
/// undone — `add_material` is deliberately not on the undo stack — so these
/// handles stay valid for the whole test). `mat_sel % 4` resolves into it:
/// `3` (the last slot) is the unpainted sentinel `None`.
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
/// - `DocumentError::ReplaceMaterialReplayStale`: this harness's own
///   full-log unwind-and-replay (twice) is exactly the shape that can churn
///   a `replace_material` batch's recorded face stale — split/merge one of
///   its target faces via an unrelated later op, then unwind past both —
///   the same drift `doc_fingerprint`'s doc comment describes for
///   `PaintFace`. The error is unambiguous (only this one replay path
///   produces it), so no `peek` is needed to confirm which action is
///   pending, unlike the `UnbuildPushPull` signature below.
fn is_known_inverse_guard_gap(doc: &Document, e: &DocumentError, redo: bool) -> bool {
    if matches!(e, DocumentError::Sketch(SketchError::RestoreConflicts)) {
        let pending = if redo {
            doc.peek_redo_action_kind()
        } else {
            doc.peek_undo_action_kind()
        };
        return matches!(pending, Some(PendingActionKind::CreatedObject));
    }
    if matches!(e, DocumentError::ReplaceMaterialReplayStale) {
        return true;
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
    Ok(())
}

/// Canonical, slot-order-independent document fingerprint: per visible
/// object, its sorted vertex positions (rounded well below any op tolerance)
/// and sorted face sizes; plus node/instance/component counts. Two documents
/// with equal fingerprints are the same model up to handle renaming.
///
/// Deliberately does NOT include face/object material state. A scratch
/// repro (split a face, paint one child, then unwind-and-replay the WHOLE
/// document log twice) pinned down a pre-existing, orthogonal gap: a
/// face-level attribute recorded against a `FaceId` survives its own
/// immediate undo/redo perfectly, but if an EARLIER structural op that
/// created that face is *itself* undone and redone in a later cycle, the
/// redo mints a fresh slotmap generation for the face — the paint action's
/// recorded handle goes stale and its restore silently no-ops (`if let
/// Some(f) = ...get_mut(face)`, the same guard `PaintFace`'s undo/redo has
/// always used). This affects `PaintFace` identically; it is not introduced
/// by `replace_material`. `replace_material`'s OWN batched undo/redo replay
/// no longer shares the silent-no-op half of this gap — a stale recorded
/// face now refuses the WHOLE batch typed
/// (`DocumentError::ReplaceMaterialReplayStale`, tolerated by
/// `is_known_inverse_guard_gap` below) rather than partially applying the
/// rest, so this harness's full double-unwind/replay can hit that typed
/// refusal and abandon the round-trip for the case instead of silently
/// drifting — see `document_specs.rs` for the exact repro and the local
/// round-trip check below for the immediate-bracketing (never-stale) case.
/// It is still not a safe place to assert material fidelity across the full
/// unwind/replay, since `PaintFace`'s own silent-no-op half of the gap is
/// unaffected and unaddressed here — a face-handle stability question that
/// reaches split/boolean/push-through undo broadly, well past paint-tool
/// Phase 1's scope.
fn doc_fingerprint(doc: &Document) -> String {
    let q = |c: f64| (c * 1e7).round() as i64;
    let object_print = |oid| {
        let obj = doc.object(oid).expect("live id resolves");
        let (pts, faces) = obj.to_polygons();
        let mut vs: Vec<(i64, i64, i64)> = pts.iter().map(|p| (q(p.x), q(p.y), q(p.z))).collect();
        vs.sort_unstable();
        let mut sizes: Vec<usize> = faces.iter().map(Vec::len).collect();
        sizes.sort_unstable();
        format!("{vs:?}|{sizes:?}")
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
        .map(|sid| sketch_print(doc.sketch(sid).expect("live id resolves")))
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
                .filter_map(|sid| doc.sketch(sid).map(sketch_print))
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
                    curve: None,
                },
            );
        }
        DocOp::Boolean { kind, a_sel, b_sel } => {
            let ids = doc.visible_object_ids();
            let (Some(a), Some(b)) = (nth(&ids, *a_sel), nth(&ids, *b_sel)) else {
                return Ok(true);
            };
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
            let _ = doc.duplicate_node(NodeId::Object(oid), &t);
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

        for (step, op) in ops.iter().enumerate() {
            if !apply_doc_op(&mut doc, step, op, &materials)? {
                return Ok(());
            }
            check_doc(&doc, step, "apply")?;
        }

        check_persistence(&doc, "post-sequence")?;

        // Unwind the whole document log, replay it, and do both again; both
        // ends must reproduce the same canonical fingerprint, and every
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
                n += 1;
            }
            Ok(true)
        };

        if !unwind(&mut doc, "first unwind")? {
            return Ok(());
        }
        let empty_print = doc_fingerprint(&doc);

        if !replay(&mut doc, "first replay")? {
            return Ok(());
        }
        let maximal_print = doc_fingerprint(&doc);
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
    }
}
