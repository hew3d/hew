/**
 * TapeMeasureTool — SketchUp-style Tape Measure, scoped to what the
 * current kernel/UI support: live distance readout, parallel construction
 * guide lines (from an existing edge), and standalone guide points.
 *
 * Gesture:
 *   1. First click:
 *      - On an edge (snap.elementKind === 'edge' for a live world Object,
 *        or 'sketch-edge' for a committed sketch line) → PARALLEL-GUIDE
 *        mode. Remembers the edge's two endpoints (via
 *        `wasmScene.edge_endpoints` / `sketch_edge_endpoints`) and the
 *        picked point on it.
 *      - On a world axis or an existing guide line ('on-axis'/'on-guide'
 *        snaps, which carry the analytic line's direction) → the same
 *        PARALLEL-GUIDE mode, sourced from the infinite analytic line.
 *      - Anywhere else → MEASURE mode. Remembers the picked point as P0.
 *   2. Pointer move previews:
 *      - Parallel mode, no resolved offset plane (`_resolveOffsetPlane`
 *        returns null — the common case): a dashed guide line through
 *        (edgePoint + offset) along the edge direction, where `offset` is
 *        the component of (cursor − edgePoint) perpendicular to the edge
 *        direction (so dragging the cursor off the edge "pulls" the guide
 *        sideways, as in SketchUp).
 *      - Parallel mode WITH a resolved offset plane (WP-4 — a mid-gesture
 *        arrow-key axis lock, a frozen sketch/idle-lock plane, or the face
 *        anchored at the gesture's first click, LIVE-tracked — engaged only
 *        while the cursor ray is still over that same face, released the
 *        instant it isn't): the guide instead offsets along that plane's
 *        in-plane direction `u`, via `signedOffsetAlong` — see
 *        `_resolveOffsetPlane`'s doc for the full precedence, and `onKey`
 *        for the arrow-key lock itself.
 *      - Measure mode: a dashed segment from P0 to the cursor, with a live
 *        `formatLength` readout via the measurement callback.
 *   3. Second click commits:
 *      - Parallel mode → `add_guide_line(origin, direction)`.
 *      - Measure mode, cursor on real geometry → just finalizes the readout
 *        (SketchUp does the same: measuring between two existing points
 *        doesn't drop a guide).
 *      - Measure mode, cursor in empty space → `add_guide_point(p1)` PLUS
 *        (tape-measure-rework WP-7 item 3) `add_guide_line(p0, direction)`
 *        when `p1 − p0` is non-degenerate — SketchUp drops both the guide
 *        POINT at the second click and the infinite guide LINE through the
 *        two measured points.
 *      - Either commit above is skipped entirely — but the readout still
 *        completes — while Ctrl/Cmd is held (tape-measure-rework WP-7
 *        item 1, `createGuides`/`setGuideCreationSuppressed`): SketchUp's
 *        measure-WITHOUT-a-guide mode.
 *   4. VCB numeric entry (typed digits while a stage is active) commits an
 *      exact typed distance along the current direction, mirroring MoveTool.
 *   5. Esc cancels the current stage; `cancel()` clears all preview state.
 *   6. Shift held mid-gesture (tape-measure-rework WP-7 item 2,
 *      `setShiftHeld`/`_tryShiftLatch`) latches an axis lock — `offsetLock`
 *      in `parallel`, `lockAxis` in `measure` — to whichever drawing axis
 *      the gesture is currently closest to; releasing Shift releases that
 *      lock again (an explicit arrow-set lock is left alone by both). Mirrors
 *      `MoveTool.setShiftHeld`, with an added "latch pending" retry for a
 *      genuinely degenerate direction at the moment Shift is pressed.
 *
 * Sketches on any plane (the sketch-planes design §6 bullet 2): if the
 * FIRST pick of a gesture hovers a committed sketch whose plane is
 * non-ground (`pick_sketch` + `planeFromSketch` — the same helpers the draw
 * tools use), OR an idle arrow-key plane lock is active, the gesture freezes
 * that plane (`_gesturePlane`) and `snapConstraint()` returns it for the
 * REST of the gesture — so a guide/measurement started on a tilted sketch
 * stays in that sketch's plane instead of resolving to the ground fallback
 * and refusing (rule 4). The idle lock uses the same `nextIdlePlaneLock`
 * (arrow keys) as the draw tools — TapeMeasure doesn't otherwise use arrow
 * keys, so it drops in with no conflict (unlike Protractor, which already
 * owns all four arrows for its own plane lock — see ProtractorTool.ts).
 * In PARALLEL-guide mode specifically (tape-measure-rework WP-4), a frozen
 * `_gesturePlane` that DISAGREES with the picked edge (from `_tryResolveEdge`)
 * is no longer simply dropped: `_resolveOffsetPlane` DERIVES the nearest
 * edge-containing plane from it instead (`offsetPlaneNormal` — `_gesturePlane`'s
 * normal projected into the plane perpendicular to the edge direction), only
 * falling back to the legacy unconstrained offset when that derivation itself
 * degenerates (the frozen normal runs exactly along the edge). The same
 * resolver also honors a mid-gesture arrow-key axis lock (`offsetLock`) and,
 * absent either, the face anchored at the gesture's first click — but ONLY
 * for as long as the LIVE cursor ray keeps landing on that SAME face (same
 * object/face/instance handles); the moment it doesn't, this case releases
 * back to the free 2-D offset (case 4) rather than re-adopting whatever
 * different face happens to be under the cursor, or staying frozen to the
 * first face regardless of where the cursor wanders (the fixed-first-click-
 * ray bug this replaces) — see `_resolveOffsetPlane`'s doc for the full
 * precedence.
 *
 * MEASURE mode gets its own inference machinery (tape-measure-rework WP-5):
 * `snapConstraint()` reports `anchor: p0` once a measure gesture has started
 * (unless `lockAxis` is held — see below), turning on the kernel's soft
 * 5°-cone `SnapKind::OnAxis` inference (`SOFT_AXIS_APERTURE_DEG` in
 * crates/inference/src/lib.rs) — a from-point measurement gets a
 * strong-but-overridable pull toward the three principal drawing axes,
 * ranking below a deliberate on-guide snap but above plain on-edge/on-face.
 * A mid-gesture arrow key additionally hard-locks the distance to a drawing
 * axis (`lockAxis`) — but, unlike MoveTool's own `SnapLock::Axis` contract,
 * this stays entirely TS-side (`_measurePoint`, a later fix on this branch):
 * while `lockAxis` is held, `snapConstraint()` returns `null` outright — no
 * `anchor`, no `lockAxis`, nothing at all — so the kernel runs a fully
 * free/unconstrained snap, magnetized to real geometry exactly as if no
 * lock were active, and `_measurePoint` itself projects that free result
 * onto the locked axis line through `p0` (mirrors PARALLEL mode's
 * `offsetLock`/`signedOffsetAlong` pattern). This replaces sending
 * `lockAxis` to the kernel's own `SnapLock::Axis`, which unconditionally
 * force-projects its winning candidate's POSITION onto the locked axis line
 * before returning it: correct for an edge that happens to run ALONG the
 * locked axis, but for one that runs ACROSS it the projection erases all
 * cursor motion, so the reported point never visibly moves off one fixed
 * spot regardless of where the real edge actually is. Every arrow is viable
 * here, unlike PARALLEL mode's `offsetLock`, since a bare point has no
 * source edge to run along. And when a `_gesturePlane` is frozen AND
 * `lockAxis` is NOT held, `snapConstraint()` ALSO passes
 * `offPlanePoints: true` — a plane lock stops being a hard FILTER and
 * becomes a PROJECTION: a snap that lands off the frozen plane (an
 * endpoint/midpoint/center that doesn't happen to lie exactly in it) is kept
 * reachable and projected onto the plane (`projectPointOntoPlane`) rather
 * than silently dropped, with `snapProjected()` reporting the projection so
 * the inference chip can say so (mirrors LineTool's plane-mode
 * `_planeCursor` disclosure, though for a different reason — LineTool
 * discards a Z the shape has to be planar without; here the plane is the
 * FROZEN gesture constraint, not something the target inherently can't
 * leave). An active `lockAxis` still suppresses the plane constraint
 * entirely for as long as it's held — the same precedence PARALLEL mode's
 * `offsetLock` already has over its own frozen-plane case — but now simply
 * because `snapConstraint()` returns `null` while it's held, leaving nothing
 * to combine a plane constraint with in the first place. With no frozen
 * plane, an off-plane point still becomes the literal destination unchanged
 * — today's angled-measurement behavior. The rescale-arm guard
 * (`_commitFromTyped`'s `p0OnGeometry && onGeometry` check) always reads
 * off the RAW snap's `snapOnGeometry`, never the projected point's — a
 * projection changes WHERE the point lands, not whether the thing it was
 * snapped to was real geometry.
 *
 * Explicitly OUT of scope for this slice (see ROADMAP):
 *   - Protractor (angle guides) —, a separate tool.
 *
 * Parallel guides from an edge on INSTANCED geometry (a component-instance
 * member's edge) work the same as a plain world Object's: `_tryResolveEdge`
 * sources the endpoints via `edge_endpoints_in_instance`/`sketch_edge_
 * endpoints_in_instance` (composing the instance's pose in), falling back to
 * the plain world-space call when the snap carries no instance and the tool
 * isn't inside an instance edit context either. A grouped object needs no
 * such handling — it's still `is_world()` in the kernel, so the plain
 * `edge_endpoints` path already covers it.
 *
 * The corner measurement widget persists (tape-measure-rework part 1,
 * Tape Measure ONLY — the other tools sharing the same widget still clear it
 * on every commit/abort): `_resetToIdle` no longer blanks the readout
 * unconditionally. A genuine commit (a parallel guide, a measurement, a
 * confirmed/declined rescale) FREEZES it at the last value instead — via the
 * `frozen` flag on `OnMeasurement`, which the Viewport forwards only for this
 * tool — so the number stays legible after the click, exactly where a user
 * would look to double-check what they just measured; an abort that
 * committed nothing (`cancel()`, a degenerate typed-offset bail in
 * `_commitFromTyped`) clears it instead, since there's nothing genuine left
 * to show. `_lastReadout`/`_pushReadout` funnel every readout through one
 * place so the freeze/clear split has a single point of truth.
 *
 * Typing a length is no longer confined to the WINDOW between the first and
 * second click (part 2): `_recall` remembers the last eligible two-click
 * measurement (both ends on real geometry, non-degenerate) after the gesture
 * has already returned to idle, and `onKey`'s idle branch accepts a fresh
 * typed buffer against it — Enter re-arms the same "resize the model?"
 * confirmation `_commitFromTyped`'s mid-gesture arm produces, just from cold
 * idle instead of a live gesture (`_armRescaleFromRecall`). `capturesKey`/
 * `hasArmedGesture` exist so the Viewport's key routing and Escape-vs-
 * context-pop precedence (component-edit-parity.md phase A2) both see this
 * idle recall as "armed" too, the same as a live gesture.
 */

import * as THREE from 'three'
import type { Tool, Snap, EditContext, SnapConstraint } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { editLengthBuffer, isLengthInputKey, pointAlong, nextIdlePlaneLock, arrowToAxis, AXIS_LOCK_COLOR_NAMES } from './moveInput'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { planeFromSketch, axisDrawPlane, isPointOnDrawPlane, SketchPickCache, instanceOf, GROUND_PLANE_EPS, type DrawPlane } from './drawPlane'
import { getDrawingAxes } from './drawingAxes'
import type { V3 } from '../viewport/geoHelpers'
import { crossV3, normalizeV3, dotV3, subV3 } from '../viewport/geoHelpers'
import { worldFaceNormal } from './faceDraw'
import {
  AXIS_ALONG_EDGE_SIN,
  AXIS_ALONG_EDGE_COS,
  offsetDirForAxis,
  offsetPlaneNormal,
  viableOffsetAxes,
  signedOffsetAlong,
  projectPointOntoPlane,
  stationOnAxisFromRay,
} from './tapeOffset'
import { axisColorForDirection, axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'
import { GUIDE_COLOR } from '../viewport/guideColors'

/** Whether a snap landed on real picked geometry, as opposed to a broad
 * empty-space fallback: 'ground' (ray∩ground) or 'plane' (ray∩constraint
 * plane — sketches on any plane, Phase 1). Guides anchored to geometry
 * survive re-inference; free-space landings do not count. */
function snapOnGeometry(snap: Snap): boolean {
  return snap.kind !== 'ground' && snap.kind !== 'plane'
}

export type OnGuideCreated = () => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string, frozen?: boolean) => void

/** Info describing an armed "resize the model?" confirmation (design
 *  tool-parity §3), passed to `OnRescaleArmed` so the Viewport/App can render
 *  the confirmation modal. `factor` is the value `rescale_document` will be
 *  called with on confirm — `typedDistance / currentDistance`. */
export interface RescaleConfirmInfo {
  /** The real, currently-measured distance between the two picked points. */
  currentDistance: number
  /** The length the user just typed. */
  typedDistance: number
  /** `typedDistance / currentDistance`. */
  factor: number
}

export type OnRescaleArmed = (info: RescaleConfirmInfo) => void
/** Fired when a rescale is actually applied (confirmRescale succeeded) — the
 *  Viewport must do a FULL scene refresh, unlike a guide-point commit
 *  (`OnGuideCreated`), since a rescale touches every object/sketch/guide.
 *  Carries the applied `factor` so the Viewport can also re-scale the
 *  CAMERA about the same world-origin pivot by the same factor (design
 *  tool-parity §3's "the view jumps around" fix) — a view-side adjustment
 *  the kernel has no part in and that has no undo of its own. */
export type OnRescaleApplied = (factor: number) => void

/** Half-length of the previewed parallel-guide line (meters). */
const GUIDE_HALF_LENGTH = 50

/** Half-length of the measure-stage locked-axis leader line (meters) —
 *  matches `CueLayer`'s own `GUIDE_HALF_LENGTH` (5) so the visual doesn't
 *  change size from what the kernel's axis-lock direction cue used to draw:
 *  since measure mode's `lockAxis` no longer reaches the kernel (the
 *  locked-axis-projection fix), `Snap.direction` no longer carries the
 *  locked axis, so `CueLayer` no longer draws this line on its own — this
 *  tool now draws it itself, in `_updatePreviewLine`. */
const AXIS_LEADER_HALF_LENGTH = 5

/** The arrow key that locks/releases each offset axis (0/1/2), for the
 *  `offsetLock` status hint — the reverse of `arrowToAxis`'s key→axis
 *  mapping (ArrowRight→0, ArrowLeft→1, ArrowUp→2). */
const OFFSET_LOCK_KEY_NAMES: readonly [string, string, string] = ['Right', 'Left', 'Up']

/** Adjacency tolerance (world meters) for `_resolveFaceAnchor`'s gate on a
 *  face pick actually passing through/near the source edge point, not
 *  merely being parallel to it: `|dot(faceNormal, hit − edgePoint)|` must
 *  fall within this to engage case 3. Deliberately generous relative to the
 *  expected f64 round-trip precision through wasm (a real click's hit point
 *  and the edge point it's being compared against are both computed from
 *  independent kernel calls, not the same one) — this is a coarse "is this
 *  face anywhere near the edge" gate, not a tight coplanarity check. */
const FACE_ADJACENCY_EPS = 1e-6

/** The face the parallel gesture's FIRST click landed on, when it is an
 *  eligible offset-plane source for the source edge. Everything derivable at
 *  click time is precomputed here, so the per-move test is a pure handle
 *  comparison. */
interface OffsetFaceAnchor {
  object: bigint
  face: bigint
  /** `null` for a world object (mirrors `worldFaceNormal`'s param). */
  instance: bigint | null
  /** World unit normal ALREADY projected exactly perpendicular to `edgeDir`
   *  via `offsetPlaneNormal` — case 3 needs no further derivation. */
  planeNormal: V3
  /** `normalize(planeNormal × edgeDir)` — the in-plane offset direction. */
  u: V3
}

/** A raw `pick_face` result, read off once and disposed — everything
 *  `_resolveFaceAnchor`/`_facePickMatchesAnchor` need, with `instance`
 *  already normalized from `undefined` to `null` (mirrors `worldFaceNormal`'s
 *  param) and `depth` (a world-meter ray-parameter distance) carried through
 *  for the adjacency gate. */
type RawFacePick = { object: bigint; face: bigint; instance: bigint | null; depth: number }

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'parallel'
      /** A point on the source edge, in world space (the pick that started this gesture). */
      edgePoint: [number, number, number]
      /** Unit direction of the source edge. */
      edgeDir: [number, number, number]
      /** Last computed guide origin (edgePoint + offset — perpendicular free
       *  offset, or along a resolved `offsetPlane`'s `u`, per `offsetPlane`). */
      origin: [number, number, number]
      /** The resolved offset plane for this gesture (WP-4), re-resolved by
       *  `_resolveOffsetPlane` on every pointer move and lock change — null
       *  means today's free 2-D `perpComponent` offset (no plane/axis
       *  constraint applies). See `_resolveOffsetPlane`'s doc for precedence. */
      offsetPlane: { origin: V3; normal: V3; u: V3 } | null
      /** The face the gesture's FIRST click landed on, resolved ONCE at
       *  `onPointerDown` — `null` when the first click hit no eligible face.
       *  `_resolveOffsetPlane`'s case 3 engages this only while the LIVE
       *  cursor ray keeps landing on this SAME face (`_facePickMatchesAnchor`)
       *  — it never re-adopts a different face mid-gesture. */
      faceAnchor: OffsetFaceAnchor | null
      /** The most recent `onPointerMove`'s ray, seeded with the first-click
       *  ray at `onPointerDown` — `onKey`/`_tryShiftLatch`/`setShiftHeld` call
       *  `_updateParallelOrigin` with no ray of their own and need the
       *  last-known one to re-test the face anchor. */
      lastRay: Ray
      /** Last cursor point seen by `onPointerMove` (or `edgePoint` before the
       *  first move) — lets a mid-gesture arrow-key lock change (`onKey`)
       *  recompute `origin` immediately without waiting for a fresh pointer
       *  move. */
      lastCursor: [number, number, number]
    }
  | {
      kind: 'measure'
      /** First picked point, in world space. */
      p0: [number, number, number]
      /** Whether `p0` itself rests on real geometry (vs. empty space) — the
       *  rescale-confirm arm requires BOTH ends to be real, known points
       *  (design tool-parity §3): resizing to preserve an arbitrary
       *  empty-space-anchored distance has no meaningful "this distance". */
      p0OnGeometry: boolean
      /** Last cursor point (snapped), in world space. */
      p1: [number, number, number]
      /** Whether the cursor is currently resting on real geometry (vs. empty space). */
      onGeometry: boolean
    }
  | {
      kind: 'pendingRescale'
      /** The measurement's first point — `cancelRescale`'s fallback commit
       *  needs it to drop the guide point exactly as an ordinary typed
       *  measure commit would have. */
      p0: [number, number, number]
      /** The typed-exact endpoint `cancelRescale` drops a guide point at
       *  (mirrors the pre-existing typed-measure-commit target). */
      endpoint: [number, number, number]
      /** `rescale_document`'s argument on confirm. */
      factor: number
      /** Snapshot of `createGuides` at the moment this stage was ARMED (the
       *  Ctrl/Cmd "measure only" flag, WP-7 item 1) — `cancelRescale`'s
       *  fallback guide drop must honor what was true when the typed
       *  distance was entered and the confirmation dialog first appeared,
       *  not whatever `createGuides` has drifted to live: Ctrl/Cmd is
       *  dispatched from window-level listeners that keep running while the
       *  dialog is open and aren't gated on this stage. The retroactive
       *  (`fromRecall`) arm has no fallback commit to snapshot for, so this
       *  field's value is unused on that path — don't read it there. */
      createGuidesAtArm: boolean
      /** True when this arm came from a RECALLED measurement (typed while
       *  idle, after the gesture already committed) rather than a live
       *  gesture. Governs `cancelRescale()`: a live arm falls back to the
       *  guide-point commit the typed distance would have produced; a
       *  recalled arm has no pending gesture to fall back to and creates
       *  nothing on cancel. */
      fromRecall: boolean
    }

/** v − (v·d)d for unit d: the component of v perpendicular to d. */
function perpComponent(
  v: [number, number, number],
  d: [number, number, number],
): [number, number, number] {
  const dot = v[0] * d[0] + v[1] * d[1] + v[2] * d[2]
  return [v[0] - dot * d[0], v[1] - dot * d[1], v[2] - dot * d[2]]
}

export class TapeMeasureTool implements Tool {
  readonly name = 'Tape Measure'

  /** The current editing context (component-edit-parity.md phase A1) —
   *  consulted by `_tryResolveEdge`: a `sketch-edge` snap with no `instance`
   *  of its own (e.g. from `pick_sketch`, which doesn't report one) still
   *  needs a placing instance to resolve a definition-owned sketch edge's
   *  world-space endpoints, so an `'instance'` context supplies its `id` as
   *  the fallback. */
  private _editContext: EditContext = { kind: 'top' }
  setEditContext(ctx: EditContext): void {
    this._editContext = ctx
  }

  /**
   * Ctrl/Cmd held/released (WP-7 item 1 — SketchUp's measure-only mode):
   * sets `createGuides = !held`. Only meaningful mid-gesture (`parallel`/
   * `measure` stages — see `_commitParallelGuide`/`_commitMeasure`), but
   * safe to call at any time: nothing reads `createGuides` outside those two
   * commit paths, so a call while idle (e.g. Ctrl already held before the
   * first click) simply arrives early and is harmless either way.
   */
  setGuideCreationSuppressed(held: boolean): void {
    this.createGuides = !held
  }

  /**
   * Shift held/released (WP-7 item 2 — mirrors `MoveTool.setShiftHeld`'s
   * toggle/release convention, with an added LATCH-PENDING retry this file's
   * gestures need that Move's don't: a Shift press can land on a genuinely
   * degenerate direction — the cursor still sitting on the source edge in
   * `parallel`, or `p1 === p0` in `measure` — right at the moment of the
   * press, with no dominant direction yet to latch onto).
   *
   * Applies ONLY to the `parallel`/`measure` stages; `idle`/`pendingRescale`
   * are fully inert — no latch, no pending, no state written at all — so
   * Shift held before the first click can never pre-lock the gesture that's
   * about to start.
   *
   * On press: an already-set lock (arrow or a prior Shift latch) is left
   * alone. Otherwise, try to latch immediately (`_tryShiftLatch`); if that
   * fails (degenerate direction right now), remember to keep trying
   * (`_shiftLatchPending`) — `onPointerMove` retries every subsequent move.
   *
   * On release: `_shiftLatchPending` always clears (Shift is no longer
   * held, so there's nothing left to retry). If the CURRENT lock has Shift
   * provenance (`_shiftAxisLock`), it's released too, and the stage
   * refreshes to show the unlocked state immediately. An ARROW-set lock
   * (`_shiftAxisLock` false) survives Shift release untouched — only an
   * explicit arrow or ArrowDown releases that one (see `onKey`).
   */
  setShiftHeld(held: boolean): void {
    if (this.stage.kind !== 'parallel' && this.stage.kind !== 'measure') return

    if (held) {
      const cur = this.stage.kind === 'parallel' ? this.offsetLock : this.lockAxis
      if (cur !== null) return
      if (!this._tryShiftLatch()) {
        this._shiftLatchPending = true
      }
      return
    }

    this._shiftLatchPending = false
    if (this._shiftAxisLock) {
      if (this.stage.kind === 'parallel') {
        this.offsetLock = null
      } else {
        this.lockAxis = null
      }
      this._shiftAxisLock = false
      this._offsetAxisRejectedHint = null
      if (this.stage.kind === 'parallel') {
        this._updateParallelOrigin(this.stage.lastCursor)
      }
    }
  }

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    // WP-7 item 1: appended to every `parallel`/`measure` hint below while
    // guide creation is suppressed — including the axis-rejection hint,
    // which stays highest precedence (unchanged from before this WP; see
    // the `parallel` case).
    const suppressSuffix =
      (this.stage.kind === 'parallel' || this.stage.kind === 'measure') && !this.createGuides
        ? ' — measuring only, no guide will be created.'
        : ''

    switch (this.stage.kind) {
      case 'parallel': {
        if (this._offsetAxisRejectedHint !== null) return this._offsetAxisRejectedHint + suppressSuffix
        if (this.offsetLock !== null) {
          if (this._shiftAxisLock) {
            return (
              `Offset locked to ${AXIS_LOCK_COLOR_NAMES[this.offsetLock]} while Shift is held — ` +
              `release Shift, or press an arrow to keep it.` + suppressSuffix
            )
          }
          const key = OFFSET_LOCK_KEY_NAMES[this.offsetLock]
          return `Offset locked to ${AXIS_LOCK_COLOR_NAMES[this.offsetLock]} — ${key} again or Down to release.` + suppressSuffix
        }
        return 'Click to place the parallel guide — or type an exact offset.' + suppressSuffix
      }
      case 'measure': {
        if (this.lockAxis !== null) {
          if (this._shiftAxisLock) {
            return (
              `Measurement locked to ${AXIS_LOCK_COLOR_NAMES[this.lockAxis]} while Shift is held — ` +
              `release Shift, or press an arrow to keep it.` + suppressSuffix
            )
          }
          const key = OFFSET_LOCK_KEY_NAMES[this.lockAxis]
          return `Measurement locked to ${AXIS_LOCK_COLOR_NAMES[this.lockAxis]} — ${key} again or Down to release.` + suppressSuffix
        }
        return 'Click the second point to read the distance — or type an exact distance to drop a guide there.' + suppressSuffix
      }
      case 'pendingRescale':
        if (this.stage.fromRecall) {
          return 'Resize the model to the typed distance? Confirm in the dialog, or Esc to leave the model alone.'
        }
        return 'Resize the model to the typed distance? Confirm in the dialog, or Esc to just drop a guide instead.'
      default:
        // Idle precedence (tape-measure-rework part 2, first match wins): an
        // OPEN typed buffer beats everything else, since Enter/Esc act on it
        // right now; an idle plane lock is the next most specific state;
        // an eligible-but-untyped recall is next; plain idle is the fallback.
        if (this.typed !== '') {
          return 'Press Enter to resize the model to the typed distance — Esc to keep the measurement as it is.'
        }
        if (this.idlePlaneLock !== null) {
          return `Locked to the ${AXIS_LOCK_COLOR_NAMES[this.idlePlaneLock]} plane — click to start; same arrow or Esc unlocks.`
        }
        if (this._recall !== null) {
          return 'Type a distance and press Enter to resize the model to it — or click a point to measure from.'
        }
        return 'Click a point to measure from — or click an edge to drop a parallel guide.'
    }
  }

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onGuideCreated: OnGuideCreated
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement
  private onRescaleArmed: OnRescaleArmed
  private onRescaleApplied: OnRescaleApplied

  /** VCB buffer — raw string being typed by the user. */
  private typed: string = ''

  /** THREE.js LineSegments for the preview guide/segment. */
  private previewLine: THREE.LineSegments | null = null

  /** THREE.js LineSegments for the `parallel`-stage offset connector (WP-6):
   *  from `edgePoint` to `origin`, drawing the resolved offset itself.
   *  Always null outside `parallel` mode, and while `parallel` but the
   *  offset is degenerate (cursor still on the edge). See
   *  `_updatePreviewLine`/`_clearPreviewLine`. */
  private previewConnector: THREE.LineSegments | null = null

  /** THREE.js LineSegments for the `measure`-stage locked-axis leader line
   *  (the locked-axis-projection fix, section E): since `lockAxis` no longer
   *  reaches the kernel, `CueLayer`'s own dashed direction cue (driven by
   *  the kernel-returned `Snap.direction`) no longer reflects the locked
   *  axis — this tool draws its own leader line through `p0` spanning
   *  `±AXIS_LEADER_HALF_LENGTH` along the locked axis instead. Always null
   *  outside `measure` mode, and while `measure` but `lockAxis` is null. See
   *  `_updatePreviewLine`/`_clearPreviewLine`. */
  private previewAxisGuide: THREE.LineSegments | null = null

  /** Per-pointer-event `pick_sketch` memo — see `SketchPickCache` in drawPlane.ts. */
  private readonly _sketchPickCache = new SketchPickCache()

  /** The plane the CURRENT gesture is frozen to (sketches on any plane,
   *  design §6 bullet 2) — resolved once at the first click, from either the
   *  idle arrow-key lock or a hovered non-ground sketch; null for an
   *  unconstrained (ground/free-space) gesture. Cleared back to null by
   *  `_resetToIdle()`. */
  private _gesturePlane: DrawPlane | null = null

  /** Idle plane lock (mirrors the draw tools' — design §5.2/§6 bullet 2):
   *  while FULLY idle, an arrow key locks the future plane's NORMAL to a
   *  world axis; the same arrow again, or Escape/ArrowDown, clears it.
   *  Consumed by the first click, which freezes `_gesturePlane` through the
   *  clicked point. Survives a completed gesture (cleared only by
   *  `cancel()`). */
  private idlePlaneLock: 0 | 1 | 2 | null = null

  /** Mid-gesture offset-axis lock (WP-4, mirrors `MoveTool.lockAxis`'s
   *  toggle/release convention): while the `parallel` stage is active, an
   *  arrow key locks the offset direction to that drawing axis (via
   *  `_resolveOffsetPlane`'s case 1); the same arrow again, or ArrowDown,
   *  releases it. Unlike `idlePlaneLock`, this is scoped to a single
   *  gesture — reset to null by `_resetToIdle()`, never survives past it. */
  private offsetLock: 0 | 1 | 2 | null = null

  /** Status-hint override (WP-4): set when an arrow key is pressed mid-
   *  `parallel`-gesture for an axis that runs along the source edge (a
   *  no-op on `offsetLock` itself) so the rejection reason is visible
   *  instead of the keypress being silently swallowed. Cleared whenever
   *  `offsetLock` changes or the gesture resets. */
  private _offsetAxisRejectedHint: string | null = null

  /** Mid-gesture axis lock (WP-5), `measure` stage only — mirrors
   *  `MoveTool.lockAxis`'s toggle/release convention exactly (see
   *  `onKey`): an arrow key hard-locks the measured distance to that
   *  drawing axis; the same arrow again, or ArrowDown, releases it. Unlike
   *  `offsetLock` above, a from-point measurement has no source edge to run
   *  along, so all three axes are always viable — every arrow press is
   *  accepted, never rejected. Scoped to a single gesture, like
   *  `offsetLock` — reset to null by `_resetToIdle()`, never survives past
   *  it. */
  private lockAxis: 0 | 1 | 2 | null = null

  /** See `Tool.snapProjected`. Set by `_measureTarget` in the `measure`
   *  stage (WP-5, a snap's position replaced by a plane projection), and by
   *  `_updateParallelOrigin` in the `parallel` stage (the tape-measure-
   *  rework offset-plane-projection fix — the offset kept is the cursor's
   *  component along the resolved plane's `u`, so the guide's origin does
   *  not pass through the literal cursor/snap position whenever it sits off
   *  the resolved plane). False outside both stages. */
  private _snapProjected = false

  /** Ctrl/Cmd-held "measure only" mode (SketchUp parity, WP-7 item 1): when
   *  false, the `parallel`/`measure` commit paths (`_commitParallelGuide`/
   *  `_commitMeasure`) skip their `add_guide_line`/`add_guide_point` kernel
   *  calls entirely — the measurement/readout still completes normally.
   *  Written by `setGuideCreationSuppressed` (dispatched from the Viewport's
   *  live Ctrl/Cmd held-state, mirroring `setShiftHeld` below); reset to
   *  `true` by `_resetToIdle()`. */
  private createGuides = true

  /** True when the ACTIVE stage's lock (`offsetLock` in `parallel`,
   *  `lockAxis` in `measure`) was set by a Shift latch (`_tryShiftLatch`)
   *  rather than an explicit arrow key (WP-7 item 2) — governs both
   *  `setShiftHeld`'s release behavior (an arrow-set lock survives Shift
   *  release; a Shift-set one doesn't) and `statusHint()`'s wording. Cleared
   *  by `_resetToIdle()` and by an explicit arrow key, which always
   *  supersedes Shift provenance even when it lands on the same axis. */
  private _shiftAxisLock = false

  /** True while Shift is held but no lock could be latched yet — a
   *  degenerate direction at press time (parallel: cursor still on the
   *  edge; measure: `p1 === p0`). Retried on every subsequent
   *  `onPointerMove` (WP-7 item 2) until `_tryShiftLatch` succeeds or Shift
   *  is released. Cleared by `_resetToIdle()`. */
  private _shiftLatchPending = false

  /** The last text pushed to the measurement widget, kept so a FREEZE
   *  (`_resetToIdle('freeze')`, the default) can re-push it verbatim instead
   *  of recomputing it, and so an idle Escape that clears the retroactive-
   *  rescale typed buffer (part 2) can restore exactly this text. Never
   *  itself the source of truth for what's ON SCREEN — `_pushReadout` is. */
  private _lastReadout = ''

  /** Funnels every readout through one place (tape-measure-rework part 1),
   *  so the freeze/clear split has a single point of truth: records what was
   *  just shown, then forwards both the text and whether it should survive
   *  `_resetToIdle`'s next clearing pass. */
  private _pushReadout(text: string, frozen: boolean): void {
    this._lastReadout = text
    this.onMeasurementCb(text, frozen)
  }

  /** A completed two-click measurement that is still a valid rescale
   *  reference: both ends landed on real geometry and the distance is
   *  non-degenerate. Non-null IS the eligibility fact — an ineligible
   *  measurement is simply never recorded. Survives `_resetToIdle`; cleared
   *  by `cancel()`, by the first click of any new gesture, by
   *  `confirmRescale()`, by `forgetRecall()`, and by `onDocumentReset()`. */
  private _recall: { p0: [number, number, number]; p1: [number, number, number]; dist: number } | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onGuideCreated: OnGuideCreated,
    onToast: OnToast,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
    onRescaleArmed: OnRescaleArmed = () => { /* no-op */ },
    onRescaleApplied: OnRescaleApplied = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onGuideCreated = onGuideCreated
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
    this.onRescaleArmed = onRescaleArmed
    this.onRescaleApplied = onRescaleApplied
  }

  // ── Optional Tool interface extensions ─────────────────────────────────

  capturingInput(): boolean {
    return this.stage.kind !== 'idle'
  }

  /** Which keys the IDLE stage owns for the retroactive-rescale buffer
   *  (tape-measure-rework part 2). Mirrors Viewport.tsx's own typed-FOV-entry
   *  rule: only a digit or `.` can OPEN the buffer; once open, the full
   *  length-input grammar (unit-suffix letters, quote/slash/space,
   *  Backspace) plus Enter. No recall (`_recall === null`) means idle owns no
   *  keys at all — nothing to type a length against. */
  private _idleCapturesKey(key: string): boolean {
    if (this._recall === null) return false
    if (this.typed !== '') return key === 'Enter' || isLengthInputKey(key)
    return /^[0-9.]$/.test(key)
  }

  /** See `Tool.capturesKey`. Every non-idle stage captures the whole
   *  keyboard, unchanged from before this file had a per-key refinement at
   *  all; only the `idle` stage's retroactive-rescale buffer (part 2) is
   *  selective about which keys it owns. */
  capturesKey(key: string): boolean {
    if (this.stage.kind !== 'idle') return true
    return this._idleCapturesKey(key)
  }

  /** See `Tool.hasArmedGesture`. A live gesture (`capturingInput()`) is
   *  armed, same as ever — but so is an idle typed buffer, an idle recall
   *  with nothing yet typed (a bare digit is still one keystroke away from
   *  reopening the rescale prompt), and a persisted idle plane lock: Escape
   *  must dismiss any of these BEFORE it's allowed to pop a component/group
   *  edit context (tape-measure-rework part 2 / component-edit-parity.md
   *  phase A2's precedence). */
  hasArmedGesture(): boolean {
    return this.capturingInput() || this.typed !== '' || this._recall !== null || this.idlePlaneLock !== null
  }

  /** See `Tool.snapProjected` — two meanings, one per stage that can produce
   *  a projected result:
   *  - `measure`: true when the last snap landed off a frozen `_gesturePlane`
   *    and was projected onto it (`_measureTarget`, WP-5). False whenever no
   *    plane is frozen or the snap already sat on the frozen plane.
   *  - `parallel`: true when the last cursor point sits off the resolved
   *    offset plane (any of `_resolveOffsetPlane`'s cases 1-3) — the offset
   *    kept is the cursor's component along the plane's in-plane direction
   *    `u`, not its literal position, so the guide's origin does not pass
   *    through the point the cursor is actually over (`_updateParallelOrigin`).
   *    False when no plane is resolved (free 2-D offset — case 4), or the
   *    cursor sits exactly on the resolved plane already.
   *  False outside both stages. */
  snapProjected(): boolean {
    return this._snapProjected
  }

  /**
   * Constrain snapping to the gesture's frozen plane (design §6 bullet 2):
   * - `parallel` mid-gesture (WP-4): `_resolveOffsetPlane`, resolved LIVE from
   *   `ray` (not a cached stage field — the whole point of the face-anchor
   *   fix is that case 3 tracks the CURRENT cursor ray, not a frozen one), if
   *   it found one — `constraintPlane` through `edgePoint` (always on the
   *   resolved plane by construction) PLUS `anchor: edgePoint` so the
   *   inference engine's anchor-dependent candidates resolve against it, PLUS
   *   `offPlanePoints: true` in every one of the three plane-yielding cases —
   *   an off-plane candidate (endpoint/midpoint/center/etc.) stays reachable
   *   instead of being filtered out; `_updateParallelOrigin` is what actually
   *   consumes it, keeping only the candidate's projection along the
   *   resolved `u` (see `snapProjected`'s doc) rather than re-homing onto a
   *   new plane through it or presenting the raw point as the literal
   *   target. Deliberately NEVER a `lockAxis` field here even when
   *   `offsetLock` is set: the kernel's axis-lock (`SnapLock::Axis`) is
   *   unconstrained by any plane and would fight `constraintPlane` — the
   *   offset direction is enforced purely by `constraintPlane` plus the
   *   TS-side `signedOffsetAlong` projection in `_updateParallelOrigin`.
   *   `offPlanePoints` does not change this reasoning: the kept offset
   *   direction (`u`) is, by construction, never a direction the kernel-side
   *   axis-lock or the TS-side projection above discards or restricts, so
   *   there is no analogous collapse risk here the way there is in `measure`
   *   stage below — this was verified analytically during design review and
   *   deliberately is NOT something to "test" by adding a `lockAxis` field to
   *   this branch. No resolved plane → unconstrained (today's free 2-D
   *   offset).
   * - `measure` mid-gesture (WP-5, revised by the locked-axis-projection
   *   fix): while `lockAxis` is null, ALWAYS `anchor: p0` — the point the
   *   gesture started from — so the inference engine's soft 5°-cone
   *   `SnapKind::OnAxis` candidates (a from-point measurement's pull toward
   *   the drawing axes) become available; when `_gesturePlane` is ALSO
   *   frozen, `constraintPlane` PLUS `offPlanePoints: true` too — the plane
   *   no longer FILTERS candidates, it PROJECTS them (`_measureTarget` does
   *   the actual projecting, once the snap comes back); no frozen plane →
   *   the `constraintPlane`/`offPlanePoints` pair is simply omitted (today's
   *   unconstrained/angled behavior). While `lockAxis` IS held, this returns
   *   `null` outright — no `anchor`, no `lockAxis`, no `constraintPlane`, no
   *   `offPlanePoints` at all. Both `anchor` and `lockAxis` have to be
   *   dropped TOGETHER, not just `lockAxis`: `anchor` independently gates
   *   the kernel's own soft-axis-preference candidates (a ~5° cone around
   *   each drawing axis, unrelated to which axis is explicitly locked here),
   *   which would otherwise both COMPETE with a real-geometry snap near the
   *   locked direction and — worse — return a numerically WRONG position for
   *   it (verified empirically: aiming dead-on at a real edge 2.0m away
   *   returned an axis-soft-cone candidate at 1.9795m instead). The
   *   kernel's own `SnapLock::Axis` (what `lockAxis` used to feed) then
   *   compounds this by unconditionally force-projecting whichever candidate
   *   wins onto the locked axis line before returning it — so the fix
   *   doesn't just drop `lockAxis`, it has `_measurePoint` project a
   *   genuinely free/unconstrained kernel result onto the locked axis
   *   itself, in TS. That projection must be applied AFTER an unbiased
   *   snap, not layered on top of the kernel's own axis-biased one, or the
   *   same competition/corruption problem recurs one level up — which is
   *   also why a held `lockAxis` suppresses `constraintPlane`/
   *   `offPlanePoints` too: there is nothing left in this method's return
   *   value to combine a plane constraint with in the first place.
   * - `pendingRescale` mid-gesture: `_gesturePlane`, if the first click
   *   resolved one (else unconstrained) — no `anchor`/`lockAxis`: the
   *   gesture is already over, only the confirmation dialog is live.
   * - Idle: the idle lock is FREE (no constraint — the locked plane is
   *   derived FROM the first click, same rationale as the draw tools);
   *   absent a lock, a top-level hover over a non-ground sketch previews
   *   its plane so the first click lands precisely on it.
   */
  snapConstraint(ray: Ray): SnapConstraint | null {
    if (this.stage.kind === 'parallel') {
      const { edgePoint, edgeDir, faceAnchor } = this.stage
      const resolved = this._resolveOffsetPlane(edgeDir, edgePoint, faceAnchor, ray)
      if (resolved !== null) {
        return {
          anchor: edgePoint,
          constraintPlane: { point: edgePoint, normal: resolved.normal },
          offPlanePoints: true,
        }
      }
      return null
    }

    if (this.stage.kind === 'measure') {
      if (this.lockAxis !== null) {
        // Tape-measure-rework locked-axis fix: an explicit axis lock no
        // longer reaches the kernel AT ALL — no `anchor`, no `lockAxis`, no
        // `constraintPlane`, no `offPlanePoints`. The kernel's own
        // `SnapLock::Axis` (what `lockAxis` used to feed) unconditionally
        // force-projects its winning candidate's POSITION onto the locked
        // axis line before returning it: correct for an edge that happens to
        // run ALONG the locked axis, but for one that runs ACROSS it the
        // projection erases all cursor motion, so the reported point never
        // visibly moves off one fixed spot regardless of where the real
        // edge actually is. `anchor` has to be dropped too, not just
        // `lockAxis`: `anchor` independently gates the kernel's own
        // soft-axis-preference candidates (a ~5° cone around each drawing
        // axis, unrelated to which axis is explicitly locked here), which
        // would otherwise both COMPETE with a real-geometry snap near the
        // locked direction and — worse — return a numerically WRONG
        // position for it (verified empirically: aiming dead-on at a real
        // edge 2.0m away returned an axis-soft-cone candidate at 1.9795m
        // instead). The fix instead has `_measurePoint` project a
        // genuinely free/unconstrained kernel result onto the locked axis
        // itself, in TS — that projection must be applied AFTER an unbiased
        // snap, not layered on top of the kernel's own axis-biased one, or
        // the same competition/corruption problem recurs one level up.
        return null
      }
      const constraint: SnapConstraint = { anchor: this.stage.p0 }
      if (this._gesturePlane !== null) {
        constraint.constraintPlane = { point: this._gesturePlane.origin, normal: this._gesturePlane.normal }
        constraint.offPlanePoints = true
      }
      return constraint
    }

    if (this.stage.kind !== 'idle') {
      // `pendingRescale`: no active gesture left to anchor/lock against.
      if (this._gesturePlane !== null) {
        return { constraintPlane: { point: this._gesturePlane.origin, normal: this._gesturePlane.normal } }
      }
      return null
    }

    if (this.idlePlaneLock !== null) return null

    const handle = this._sketchPickCache.pickFor(this.wasmScene, ray)
    if (handle !== null) {
      const plane = planeFromSketch(this.wasmScene, handle)
      if (plane !== null && !plane.ground) {
        return { constraintPlane: { point: plane.origin, normal: plane.normal } }
      }
    }
    return null
  }

  // ── Tool interface ──────────────────────────────────────────────────────

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'parallel') {
      this.stage.lastRay = ray
      // `_updateParallelOrigin` now owns `_snapProjected` for this stage (the
      // offset-plane-projection fix) — no unconditional reset here.
      this._updateParallelOrigin([snap.x, snap.y, snap.z])
      // WP-7 item 2: retry a Shift latch that was degenerate at press time
      // (cursor still on the edge) now that the cursor has moved.
      if (this._shiftLatchPending) this._tryShiftLatch()
      return
    }

    if (this.stage.kind === 'measure') {
      this.stage.p1 = this._measurePoint(snap, ray)
      this.stage.onGeometry = snapOnGeometry(snap)
      this._updatePreviewLine()
      this._reportDistanceOrTyped(this.stage.p0, this.stage.p1)
      // WP-7 item 2: same retry as the parallel branch above (degenerate
      // p1 === p0 at press time).
      if (this._shiftLatchPending) this._tryShiftLatch()
    }
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'idle') {
      // Starting any new gesture invalidates whatever was previously
      // recalled (tape-measure-rework part 2) — the old measurement's world
      // points are about to be superseded as "the last thing measured".
      this._recall = null

      // Freeze the gesture's plane (design §6 bullet 2), BEFORE branching
      // into parallel/measure mode, so it constrains snapping for either.
      this._gesturePlane = this._resolveGesturePlane(ray, [snap.x, snap.y, snap.z])

      const edge = this._tryResolveEdge(snap)
      if (edge !== null) {
        const { edgePoint, edgeDir } = edge
        // WP-4: a frozen `_gesturePlane` that disagrees with the picked edge
        // is no longer dropped here — `_resolveOffsetPlane` (called by
        // `_updateParallelOrigin` below) derives the nearest edge-containing
        // plane from it instead, falling back to unconstrained only if that
        // derivation itself degenerates. `offsetLock` always starts clear at
        // the top of a new gesture.
        this.offsetLock = null
        this._offsetAxisRejectedHint = null
        this.stage = {
          kind: 'parallel',
          edgePoint,
          edgeDir,
          origin: edgePoint,
          offsetPlane: null,
          faceAnchor: this._resolveFaceAnchor(ray, edgePoint, edgeDir),
          lastRay: ray,
          lastCursor: edgePoint,
        }
        this._updateParallelOrigin(edgePoint)
        return
      }

      // `p0` is the very point `_gesturePlane` (if any) was resolved
      // through, so it's on that plane by construction — no projection
      // needed here, unlike a later measure-stage snap.
      const p0: [number, number, number] = [snap.x, snap.y, snap.z]
      const p0OnGeometry = snapOnGeometry(snap)
      this.lockAxis = null
      this._snapProjected = false
      this.stage = { kind: 'measure', p0, p0OnGeometry, p1: p0, onGeometry: p0OnGeometry }
      this._updatePreviewLine()
      return
    }

    if (this.stage.kind === 'parallel') {
      this._commitParallelGuide(this.stage.origin, this.stage.edgeDir)
      return
    }

    if (this.stage.kind === 'measure') {
      // The raw snap's on-geometry-ness (`snapOnGeometry(snap)`), NEVER the
      // projected point's — see the module doc's WP-5 paragraph and the
      // rescale-arm guard in `_commitFromTyped`. `_measurePoint` here is
      // purely about WHERE the endpoint lands, not what it commits as — and
      // it's the SAME method `onPointerMove`'s preview just called, so a
      // click always commits to exactly the point the preview was showing.
      const p1 = this._measurePoint(snap, ray)
      // A real second click (not a typed distance — `_commitFromTyped` never
      // calls this) — remember it for the retroactive-rescale recall
      // (tape-measure-rework part 2) before committing.
      this._rememberMeasurement(this.stage.p0, this.stage.p0OnGeometry, p1, snapOnGeometry(snap))
      this._commitMeasure(this.stage.p0, p1, snapOnGeometry(snap))
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      // A pending rescale confirmation: Escape is the dialog's "Cancel" —
      // reverts to the normal guide behavior (design tool-parity §3), never
      // a bare abort that would silently drop the typed distance entirely.
      if (this.stage.kind === 'pendingRescale') {
        this.cancelRescale()
        return
      }
      // Idle with an OPEN retroactive-rescale typed buffer (tape-measure-
      // rework part 2): Escape dismisses the buffer FIRST, restoring the
      // frozen original reading, and leaves the recall itself intact so a
      // fresh digit can re-arm it — an extra Escape press only pops a
      // component/group edit context (via `hasArmedGesture`, below) once
      // this buffer (and any persisted reading/idle-plane-lock) is gone.
      // `this._recall` is guaranteed non-null here: only `_idleCapturesKey`
      // — which itself requires `_recall !== null` — can have opened the
      // buffer.
      if (this.stage.kind === 'idle' && this.typed !== '') {
        this.typed = ''
        this._pushReadout(formatLength(this._recall!.dist), true)
        return
      }
      // Idle with an active plane lock: Escape clears the lock FIRST — only
      // a second Escape (already idle, unlocked) is a no-op cancel (mirrors
      // the draw tools' idle plane lock — design §6 bullet 2).
      if (this.stage.kind === 'idle' && this.idlePlaneLock !== null) {
        this.idlePlaneLock = null
        return
      }
      // Aborting an in-progress gesture keeps the plane lock: the lock is an
      // idle aiming choice, cleared only by an idle Escape or toggle
      // (parity with the draw tools).
      const lock = this.idlePlaneLock
      this.cancel()
      this.idlePlaneLock = lock
      return
    }

    // While a rescale confirmation is pending, only Escape (above) and the
    // dialog's own buttons (`confirmRescale`/`cancelRescale`) resolve it —
    // no stray keystroke should start building a new VCB buffer underneath it.
    if (this.stage.kind === 'pendingRescale') return

    if (this.stage.kind === 'idle') {
      // An OPEN retroactive-rescale typed buffer (tape-measure-rework part 2)
      // owns the keyboard FIRST — before the arrow-key plane-lock check
      // below ever runs. Without this ordering, an arrow key pressed mid-
      // entry (e.g. the buffer shows "5", Enter not pressed yet) would
      // silently set `idlePlaneLock` and do nothing to the open buffer — no
      // feedback, no relationship to what the user was doing (unlike
      // `MoveTool.onKey`'s idle branch, which `return`s unconditionally
      // before reaching its own arrow-key logic whenever a competing idle
      // feature is active). Checking here first makes an arrow key a
      // harmless no-op while a buffer is open instead: `_idleCapturesKey`
      // doesn't own arrow keys once `typed !== ''` (only Enter/digit/unit-
      // suffix/Backspace are), so the guard below just returns.
      if (this.typed !== '') {
        if (!this._idleCapturesKey(ev.key)) return
        if (ev.key === 'Enter') {
          const meters = parseLengthToMeters(this.typed)
          if (meters !== null) this._armRescaleFromRecall(meters)
          return
        }
        this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
        if (this.typed === '') {
          // Buffer fully backspaced away — restore the frozen original reading.
          this._pushReadout(formatLength(this._recall!.dist), true)
        } else {
          this._pushReadout(this._typedReadout(), false)
        }
        return
      }

      // Idle plane lock via arrow keys (design §6 bullet 2) — consumed by
      // neither hover nor preview, only by the next first click. Only
      // reached with NO typed buffer open (see above): an arrow key can
      // legitimately lock/release the plane here, since there's no
      // in-progress typed entry for it to interfere with.
      if (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft' || ev.key === 'ArrowUp' || ev.key === 'ArrowDown') {
        this.idlePlaneLock = nextIdlePlaneLock(this.idlePlaneLock, ev.key)
        return
      }
      // Retroactive-rescale recall (tape-measure-rework part 2): only a key
      // `_idleCapturesKey` actually owns gets to touch the typed buffer —
      // everything else (tool-switch shortcuts, etc.) falls through
      // untouched. `typed === ''` here (the branch above already returned
      // otherwise), so this only ever OPENS a fresh buffer with a digit/`.`.
      if (!this._idleCapturesKey(ev.key)) return
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
      this._pushReadout(this._typedReadout(), false)
      return
    }

    // Mid-gesture offset-axis lock, `parallel` stage only (WP-4) — mirrors
    // MoveTool's mid-gesture `lockAxis` toggle/release.
    if (
      this.stage.kind === 'parallel' &&
      (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft' || ev.key === 'ArrowUp' || ev.key === 'ArrowDown')
    ) {
      const requested = arrowToAxis(ev.key)
      if (requested === null) {
        // ArrowDown → clear, same as MoveTool.
        this.offsetLock = null
        this._offsetAxisRejectedHint = null
        // WP-7 item 2: an explicit arrow always supersedes/clears Shift
        // provenance — this is a real lock change, not the rejection no-op
        // just below, which must leave a live Shift lock untouched.
        this._shiftAxisLock = false
        this._shiftLatchPending = false
        this._updateParallelOrigin(this.stage.lastCursor)
        return
      }

      const frame = getDrawingAxes(this.wasmScene)
      const viable = viableOffsetAxes(this.stage.edgeDir, frame)
      if (!viable[requested]) {
        // Runs along the edge — no-op the lock, but surface why the
        // keypress didn't do anything rather than swallowing it silently.
        // A pure hint-only no-op (WP-7 item 2): deliberately does NOT touch
        // `offsetLock`/`_shiftAxisLock`/`_shiftLatchPending` — a live Shift
        // lock survives a rejected arrow untouched.
        const others = ([0, 1, 2] as const).filter((i) => i !== requested && viable[i])
        const rejected = AXIS_LOCK_COLOR_NAMES[requested]
        const rejectedCap = rejected.charAt(0).toUpperCase() + rejected.slice(1)
        const otherNames = others.map((i) => AXIS_LOCK_COLOR_NAMES[i]).join(' or ')
        this._offsetAxisRejectedHint = `${rejectedCap} runs along this edge — lock ${otherNames} instead.`
        return
      }

      this.offsetLock = this.offsetLock === requested ? null : requested
      this._offsetAxisRejectedHint = null
      // WP-7 item 2: see the ArrowDown branch's comment above.
      this._shiftAxisLock = false
      this._shiftLatchPending = false
      this._updateParallelOrigin(this.stage.lastCursor)
      return
    }

    // Mid-gesture distance-axis lock, `measure` stage only (WP-5) — mirrors
    // MoveTool's mid-gesture `lockAxis` toggle/release exactly. Unlike the
    // `parallel`-stage handling above, there is no source edge to run
    // along, so every arrow is viable: no rejection case here.
    if (
      this.stage.kind === 'measure' &&
      (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft' || ev.key === 'ArrowUp' || ev.key === 'ArrowDown')
    ) {
      const requested = arrowToAxis(ev.key)
      this.lockAxis = requested === null || requested === this.lockAxis ? null : requested
      // WP-7 item 2: an explicit arrow always supersedes/clears Shift
      // provenance, same as the `parallel`-stage branch above.
      this._shiftAxisLock = false
      this._shiftLatchPending = false
      return
    }

    if (ev.key === 'Enter') {
      const meters = parseLengthToMeters(this.typed)
      if (meters !== null) {
        this._commitFromTyped(meters)
      }
      return
    }

    if (isLengthInputKey(ev.key)) {
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
      this._pushReadout(this._typedReadout(), false)
    }
  }

  /** The typed-buffer readout, suffixed for metric formats (imperial tokens
   * like `'`/`"` are already visible in the buffer itself). */
  private _typedReadout(): string {
    return typedReadout(this.typed)
  }

  cancel(): void {
    this.idlePlaneLock = null
    this._recall = null
    this._resetToIdle('clear')
  }

  /** The document changed under an idle recall (undo/redo/explicit delete):
   *  the saved world points may no longer describe anything real, so both
   *  the recall and its frozen reading go. Mirrors `MoveTool.disarmArray`'s
   *  contract for the same three commands.
   *
   *  Guarded against an ALREADY-ARMED rescale confirmation
   *  (`this.stage.kind === 'pendingRescale'`): that dialog is a user
   *  decision in progress, not idle state — it does NOT intercept
   *  Undo/Redo/Delete itself (only Escape does, via `onKey`), so a stray one
   *  of those while it's open must not silently corrupt it. Without this
   *  guard, clicking the dialog's Confirm afterward would apply the
   *  precomputed `factor` with zero re-validation — exactly what this method
   *  exists to prevent — and Cancel's `fromRecall` branch would find
   *  `_recall` already `null` and skip restoring the frozen reading,
   *  permanently blanking the widget instead. An Undo/Redo/Delete that
   *  happens while the confirmation is open leaves it completely untouched;
   *  the user still has to explicitly Confirm or Cancel/Escape it.
   *  `_recall`/the frozen readout are only ever affected by document
   *  mutations that happen while genuinely idle with nothing armed — the
   *  only state this method's contract makes sense for. */
  forgetRecall(): void {
    if (this.stage.kind === 'pendingRescale') return
    if (this._recall === null && this._lastReadout === '' && this.typed === '') return
    this._recall = null
    this.typed = ''
    this._pushReadout('', false)
  }

  /** A new/loaded document replaced the Scene — rewind completely, parity
   *  with the draw tools' own `onDocumentReset`. */
  onDocumentReset(): void {
    this.cancel()
  }

  /**
   * Resolve the plane the gesture starting at `clickedPoint` should freeze
   * to (design §6 bullet 2): an active idle lock (through the clicked
   * point) beats sketch-hover adoption, mirroring the draw tools'
   * `_resolveClickTarget`. Returns null for ground/free-space (no
   * constraint — today's behavior).
   */
  private _resolveGesturePlane(ray: Ray, clickedPoint: [number, number, number]): DrawPlane | null {
    if (this.idlePlaneLock !== null) {
      const plane = axisDrawPlane(this.idlePlaneLock, clickedPoint, getDrawingAxes(this.wasmScene))
      return plane.ground ? null : plane
    }
    const handle = this._sketchPickCache.pickFor(this.wasmScene, ray)
    if (handle !== null) {
      const plane = planeFromSketch(this.wasmScene, handle)
      if (plane !== null && !plane.ground) return plane
    }
    return null
  }

  /**
   * Resolve the offset PLANE (and along-plane direction `u`) the `parallel`
   * stage's offset should be measured in, for the source edge's unit
   * direction `d` through `edgePoint` (WP-4). Tries each source in order,
   * stopping at the first that produces a result; `null` means none apply —
   * the caller keeps today's free 2-D `perpComponent` offset unchanged.
   *
   * 1. Mid-gesture axis lock (`offsetLock`): the current drawing frame's
   *    axis at that index, projected off `d` via `offsetDirForAxis`. A
   *    `null` there (the axis runs along the edge — shouldn't normally
   *    happen, since `onKey` already rejects a non-viable axis before
   *    setting the lock, but defended here anyway) falls through to 2.
   * 2. Frozen idle plane lock (`_gesturePlane`): its normal projected off
   *    `d` via `offsetPlaneNormal` — the nearest edge-containing plane. When
   *    `_gesturePlane`'s normal is ALREADY perpendicular to `d` (the edge
   *    already lay in that plane), `offsetPlaneNormal` returns it UNCHANGED
   *    by construction, so this reduces to exactly the old plane-freeze
   *    behavior in that case. This REPLACES the old `_edgeLiesInPlane`-based
   *    "drop the constraint on disagreement" branch: disagreement now
   *    derives a plane instead, only falling through to 3 if that
   *    derivation itself degenerates (the frozen normal runs exactly along
   *    the edge — no nearby coplanar plane exists to derive).
   * 3. The face anchored at the gesture's FIRST click (`faceAnchor`,
   *    resolved once by `_resolveFaceAnchor` at `onPointerDown`): engages
   *    ONLY while the LIVE `ray` still picks that SAME face — same
   *    object/face/instance handles (`_facePickMatchesAnchor`). The instant
   *    the cursor moves off that face, this case releases to case 4 (the
   *    free 2-D offset) instead of staying frozen to the first face
   *    regardless of where the cursor wanders (the old fixed-first-click-ray
   *    bug), and it does NOT re-adopt a different face found under a later
   *    ray — only the one face anchored at the first click ever engages for
   *    the whole gesture, never a substitute.
   *
   * `origin` in the returned plane is always `edgePoint` itself — valid by
   * construction, since every case's `normal` is perpendicular to `d` and
   * `edgePoint` sits on the source edge.
   */
  private _resolveOffsetPlane(
    d: V3,
    edgePoint: V3,
    faceAnchor: OffsetFaceAnchor | null,
    ray: Ray,
  ): { origin: V3; normal: V3; u: V3 } | null {
    if (this.offsetLock !== null) {
      const frame = getDrawingAxes(this.wasmScene)
      const a = [frame.x, frame.y, frame.z][this.offsetLock]
      const u = offsetDirForAxis(d, a)
      if (u !== null) {
        const normal = normalizeV3(crossV3(d, u))
        if (normal !== null) {
          return { origin: edgePoint, normal, u }
        }
      }
      // Degenerate (shouldn't normally happen) — fall through to case 2.
    }

    if (this._gesturePlane !== null) {
      const normal = offsetPlaneNormal(d, this._gesturePlane.normal)
      if (normal !== null) {
        const u = normalizeV3(crossV3(normal, d))
        if (u !== null) {
          return { origin: edgePoint, normal, u }
        }
      }
    }

    if (faceAnchor !== null && this._facePickMatchesAnchor(ray, faceAnchor)) {
      return { origin: edgePoint, normal: faceAnchor.planeNormal, u: faceAnchor.u }
    }
    return null
  }

  /** Per-ray memo for `pick_face`, keyed on ray IDENTITY (`===`) — the
   *  Viewport builds a fresh `Ray` object per pointer event (see
   *  `SketchPickCache.pickFor` in drawPlane.ts for the same pattern), so
   *  reference identity is a valid per-event cache key. Keeps the same ray
   *  from being picked twice in a frame where both `onPointerMove` (via
   *  `_resolveFaceAnchor`'s callers) and `snapConstraint` consult case 3. */
  private _facePickCache: { ray: Ray; pick: RawFacePick | null } | null = null

  private _pickFaceFor(ray: Ray): RawFacePick | null {
    if (this._facePickCache !== null && this._facePickCache.ray === ray) {
      return this._facePickCache.pick
    }
    const pick = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    let raw: RawFacePick | null = null
    if (pick !== undefined) {
      try {
        raw = {
          object: pick.object(),
          face: pick.face(),
          instance: pick.instance() ?? null,
          depth: pick.depth(),
        }
      } finally {
        pick.free()
      }
    }
    this._facePickCache = { ray, pick: raw }
    return raw
  }

  /**
   * Resolve the face anchor for a `parallel` gesture's FIRST click (called
   * ONCE, from `onPointerDown`'s parallel branch only). `null` on any
   * failure — no eligible face to derive an offset plane from.
   *
   * 1. `pick_face` under `ray` — no hit → null.
   * 2. The hit face's world normal (`worldFaceNormal`) — unresolvable (a
   *    stale/degenerate instance pose) → null.
   * 3. Parallelism gate (existing check, unchanged): the normal must lie
   *    within `AXIS_ALONG_EDGE_SIN` of perpendicular to `edgeDir` — a face
   *    that runs along the edge can't usefully define an offset plane.
   * 4. Adjacency gate (NEW — closes a real gap the design review found): the
   *    hit point itself (`rayOrigin + pick.depth * rayDirection` — `depth`
   *    is a world-meter ray-parameter distance, and the ray's direction is
   *    already unit length, so this is an exact world-space point, verified
   *    against `crates/wasm-api/src/lib.rs`'s `FacePickJs::depth` and
   *    `Viewport.tsx`'s ray construction) must lie ON the face's plane
   *    through `edgePoint`, not just have a plane PARALLEL to the edge — a
   *    face that happens to be parallel but sits far from the edge (e.g. the
   *    far side of a different box) is rejected here, `FACE_ADJACENCY_EPS`
   *    tolerance.
   * 5/6. `offsetPlaneNormal`/cross-product `u` derivation, same as cases
   *    1/2 above — `null` on degeneracy.
   */
  private _resolveFaceAnchor(ray: Ray, edgePoint: V3, edgeDir: V3): OffsetFaceAnchor | null {
    const pick = this._pickFaceFor(ray)
    if (pick === null) return null

    const normal = worldFaceNormal(this.wasmScene, pick.object, pick.face, pick.instance)
    if (normal === null) return null

    if (Math.abs(dotV3(normal, edgeDir)) > AXIS_ALONG_EDGE_SIN) return null

    const hit: V3 = [
      ray.origin[0] + pick.depth * ray.direction[0],
      ray.origin[1] + pick.depth * ray.direction[1],
      ray.origin[2] + pick.depth * ray.direction[2],
    ]
    if (Math.abs(dotV3(normal, subV3(hit, edgePoint))) > FACE_ADJACENCY_EPS) return null

    const planeNormal = offsetPlaneNormal(edgeDir, normal)
    if (planeNormal === null) return null
    const u = normalizeV3(crossV3(planeNormal, edgeDir))
    if (u === null) return null

    return { object: pick.object, face: pick.face, instance: pick.instance, planeNormal, u }
  }

  /** Whether the LIVE `ray` still picks the SAME face `anchor` was resolved
   *  from — all three handles, including `instance` (the same object/face
   *  pair can recur under different instances of the same component
   *  definition, and those are NOT the same anchor). */
  private _facePickMatchesAnchor(ray: Ray, anchor: OffsetFaceAnchor): boolean {
    const pick = this._pickFaceFor(ray)
    return (
      pick !== null &&
      pick.object === anchor.object &&
      pick.face === anchor.face &&
      pick.instance === anchor.instance
    )
  }

  /**
   * Recompute the `parallel` stage's `offsetPlane` and `origin` for cursor
   * position `cursor`, then refresh the preview line and measurement
   * readout — shared by `onPointerMove` (a fresh cursor position) and
   * `onKey`'s mid-gesture axis-lock toggle (the last-seen cursor position,
   * so a lock change is visible immediately without waiting for the next
   * pointer move).
   *
   * When `_resolveOffsetPlane` resolves a plane, `origin` is
   * `edgePoint + signedOffsetAlong(cursor, edgePoint, u) * u` — the cursor
   * projected onto the resolved offset LINE (not just the plane), so the
   * guide always lands exactly along `u`. Otherwise, falls through to
   * today's free 2-D `perpComponent` offset, completely unchanged.
   */
  private _updateParallelOrigin(cursor: [number, number, number]): void {
    if (this.stage.kind !== 'parallel') return
    this.stage.lastCursor = cursor

    const resolved = this._resolveOffsetPlane(
      this.stage.edgeDir,
      this.stage.edgePoint,
      this.stage.faceAnchor,
      this.stage.lastRay,
    )
    this.stage.offsetPlane = resolved
    // Offset-plane-projection fix: the cursor's position off the resolved
    // plane is discarded — only its component along `u` is kept (below) —
    // so disclose that via `snapProjected()` whenever it's actually off the
    // plane, the same disclosure obligation `measure` stage already honors.
    this._snapProjected =
      resolved !== null &&
      Math.abs(dotV3(subV3(cursor, this.stage.edgePoint), resolved.normal)) > GROUND_PLANE_EPS

    let origin: [number, number, number]
    let offsetDist: number
    if (resolved !== null) {
      const t = signedOffsetAlong(cursor, this.stage.edgePoint, resolved.u)
      origin = [
        this.stage.edgePoint[0] + resolved.u[0] * t,
        this.stage.edgePoint[1] + resolved.u[1] * t,
        this.stage.edgePoint[2] + resolved.u[2] * t,
      ]
      offsetDist = Math.abs(t)
    } else {
      const rel = subV3(cursor, this.stage.edgePoint)
      const offset = perpComponent(rel, this.stage.edgeDir)
      origin = [
        this.stage.edgePoint[0] + offset[0],
        this.stage.edgePoint[1] + offset[1],
        this.stage.edgePoint[2] + offset[2],
      ]
      offsetDist = Math.sqrt(offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2])
    }

    this.stage.origin = origin
    this._updatePreviewLine()
    // No numeric readout for parallel mode — SketchUp shows the offset
    // distance from the original edge; report that here.
    this._reportOffsetOrTyped(offsetDist)
  }

  /**
   * Try to latch a Shift-provenance axis lock for the CURRENT stage (WP-7
   * item 2). Returns `true` iff it set a lock; never called when the
   * stage's own lock field is already non-null (`setShiftHeld`/
   * `onPointerMove` both guard that).
   *
   * `parallel`: picks the VIABLE drawing axis whose offset direction
   * (`offsetDirForAxis`) is closest to the CURRENT effective offset
   * direction `uEff` — the resolved `offsetPlane.u` if one exists (so a
   * Shift latch at zero offset still works from a frozen-plane/face-derived
   * plane, not just an existing axis lock), else the free 2-D
   * `perpComponent` of `cursor − edgePoint` off the edge — so it locks
   * whichever axis the guide is ALREADY closest to running along, not an
   * arbitrary default. False if `uEff` itself is degenerate (cursor still
   * on the edge) — the caller sets `_shiftLatchPending` in that case.
   *
   * `measure`: picks the drawing axis most nearly aligned with `p1 − p0`
   * (no viability gate — every axis is viable for a from-point measurement,
   * same as the arrow-key path). False if `p1 === p0` (no direction yet).
   */
  private _tryShiftLatch(): boolean {
    if (this.stage.kind === 'parallel') {
      const { edgeDir, edgePoint, offsetPlane, lastCursor } = this.stage
      const uEff = offsetPlane !== null
        ? offsetPlane.u
        : normalizeV3(perpComponent(subV3(lastCursor, edgePoint), edgeDir))
      if (uEff === null) return false

      const frame = getDrawingAxes(this.wasmScene)
      const viable = viableOffsetAxes(edgeDir, frame)
      const axes: readonly V3[] = [frame.x, frame.y, frame.z]
      let best: 0 | 1 | 2 | null = null
      let bestAbsDot = -1
      for (const i of [0, 1, 2] as const) {
        if (!viable[i]) continue
        const u_i = offsetDirForAxis(edgeDir, axes[i])
        if (u_i === null) continue // shouldn't happen — `viable[i]` already guards this
        const d = Math.abs(dotV3(uEff, u_i))
        if (d > bestAbsDot) {
          bestAbsDot = d
          best = i
        }
      }
      // At least two axes are always viable (an edge can run along at most
      // one drawing axis), so `best` is non-null once `uEff` is defined.
      if (best === null) return false

      this.offsetLock = best
      this._shiftAxisLock = true
      this._shiftLatchPending = false
      this._offsetAxisRejectedHint = null
      this._updateParallelOrigin(this.stage.lastCursor)
      return true
    }

    if (this.stage.kind === 'measure') {
      const rel = subV3(this.stage.p1, this.stage.p0)
      const frame = getDrawingAxes(this.wasmScene)
      const axes: readonly V3[] = [frame.x, frame.y, frame.z]
      let best: 0 | 1 | 2 = 0
      let bestAbsDot = -1
      for (const i of [0, 1, 2] as const) {
        const d = Math.abs(dotV3(rel, axes[i]))
        if (d > bestAbsDot) {
          bestAbsDot = d
          best = i
        }
      }
      if (bestAbsDot < 1e-9) return false

      this.lockAxis = best
      this._shiftAxisLock = true
      this._shiftLatchPending = false
      return true
    }

    return false
  }

  // ── Private helpers ─────────────────────────────────────────────────────

  /**
   * Resolve a snap to a reference line (a point on it + its unit direction),
   * or null if the snap carries no line to be parallel to.
   *
   * Three sources qualify:
   * - a live world-Object edge (`elementKind === 'edge'`),
   * - a committed sketch edge (`elementKind === 'sketch-edge'`),
   * - a world axis or existing guide line (`kind === 'on-axis'` /
   *   `'on-guide'`): these carry no element handle at all — the kernel
   *   resolves them ANALYTICALLY (an infinite line, camera-independent) and
   *   hands back the on-line point plus the line's direction on the snap
   *   itself, which is everything parallel mode needs. Note the rendered
   *   axis geometry is irrelevant here: its per-frame clipped extent is a
   *   draw concern, never what the snap resolves against.
   *
   * An `endpoint` snap is a vertex, not a line, so it intentionally falls
   * through to measure mode (a simplification — SketchUp lets you start a
   * parallel guide from a vertex-snapped point on an edge too, but
   * distinguishing "vertex that happens to sit on an edge" needs more
   * inference-engine plumbing than this slice adds).
   *
   * Component-instance members: a plain `edge_endpoints`/`sketch_edge_
   * endpoints` call refuses a definition-owned object/sketch (it isn't
   * world geometry with a single world-space pose of its own). When the
   * snap names a placing `instance` — or, for a sketch edge, the tool is
   * currently inside an `'instance'` edit context — the `*_in_instance`
   * variant is tried first, composing that instance's pose into the
   * endpoints. A group member needs none of this: it's still `is_world()`
   * in the kernel, so it already resolves via the plain calls below.
   */
  private _tryResolveEdge(
    snap: Snap,
  ): { edgePoint: [number, number, number]; edgeDir: [number, number, number] } | null {
    if ((snap.kind === 'on-axis' || snap.kind === 'on-guide') && snap.direction !== undefined) {
      const [dx, dy, dz] = snap.direction
      const len = Math.sqrt(dx * dx + dy * dy + dz * dz)
      if (len < 1e-9) return null // degenerate direction — fall back
      return {
        edgePoint: [snap.x, snap.y, snap.z],
        edgeDir: [dx / len, dy / len, dz / len],
      }
    }

    let endpoints: Float64Array | number[] | undefined
    if (snap.elementKind === 'edge' && snap.object !== undefined && snap.element !== undefined) {
      endpoints = snap.instance !== undefined
        ? this.wasmScene.edge_endpoints_in_instance(snap.instance, snap.object, snap.element)
        : this.wasmScene.edge_endpoints(snap.object, snap.element)
    } else if (
      snap.elementKind === 'sketch-edge' &&
      snap.sketch !== undefined &&
      snap.element !== undefined
    ) {
      // A committed sketch line works as a guide reference too — the most
      // common case: a parallel guide off a just-drawn rectangle's edge.
      // `snap.instance` (if the hover carried one) wins over the edit
      // context's instance; either way, a definition-owned sketch edge
      // needs a placing instance composed in to resolve world-space
      // endpoints at all.
      const instance = snap.instance ?? instanceOf(this._editContext) ?? undefined
      endpoints = instance !== undefined
        ? this.wasmScene.sketch_edge_endpoints_in_instance(instance, snap.sketch, snap.element)
        : undefined
      if (endpoints === undefined) {
        // No instance resolved at all, OR the in-instance call refused
        // (e.g. hovering a plain world sketch while inside an unrelated
        // instance edit context — a legitimate case, not an error): fall
        // back to the ordinary world-space lookup.
        endpoints = this.wasmScene.sketch_edge_endpoints(snap.sketch, snap.element)
      }
    }
    if (endpoints === undefined) return null // stale/consumed — fall back

    const [ax, ay, az, bx, by, bz] = endpoints
    const dx = bx - ax, dy = by - ay, dz = bz - az
    const len = Math.sqrt(dx * dx + dy * dy + dz * dz)
    if (len < 1e-9) return null // degenerate edge

    return {
      edgePoint: [snap.x, snap.y, snap.z],
      edgeDir: [dx / len, dy / len, dz / len],
    }
  }

  /**
   * Record (or drop) `_recall` for a real two-click measure commit
   * (tape-measure-rework part 2) — called only from `onPointerDown`'s
   * measure-stage second-click branch, never from `_commitFromTyped`'s typed
   * path (a typed endpoint is never "on geometry" by definition, so it would
   * never be eligible anyway). Eligible only when BOTH ends rest on real
   * geometry and the distance is non-degenerate — mirrors the same
   * `p0OnGeometry && onGeometry` gate `_commitFromTyped`'s live rescale-arm
   * already uses.
   */
  private _rememberMeasurement(
    p0: [number, number, number], p0OnGeometry: boolean,
    p1: [number, number, number], p1OnGeometry: boolean,
  ): void {
    const dx = p1[0] - p0[0], dy = p1[1] - p0[1], dz = p1[2] - p0[2]
    const dist = Math.sqrt(dx * dx + dy * dy + dz * dz)
    this._recall = (p0OnGeometry && p1OnGeometry && dist > 1e-6) ? { p0, p1, dist } : null
  }

  private _commitParallelGuide(origin: [number, number, number], dir: [number, number, number]): void {
    // WP-7 item 1: Ctrl/Cmd's measure-only mode skips the guide entirely —
    // there is nothing else this commit does besides create it, so a
    // suppressed commit is just a return to idle.
    if (this.createGuides) {
      try {
        this.wasmScene.add_guide_line(origin[0], origin[1], origin[2], dir[0], dir[1], dir[2])
        this.onGuideCreated()
      } catch (err) {
        const raw = err instanceof Error ? err.message : String(err)
        this.onToast(`Couldn't create guide line: ${raw}`)
      }
    }
    this._resetToIdle()
  }

  /**
   * Commit a measure-mode gesture. If the endpoint rests on real geometry,
   * SketchUp only finalizes the readout (no guide is created, since both ends
   * are already well-defined points). In empty space, drop BOTH a guide
   * POINT at `p1` and — mirroring SketchUp, WP-7 item 3 — an infinite guide
   * LINE through `p0` along `p1 − p0`, when that direction is non-degenerate
   * (the two points coincide — shouldn't normally happen, but defended
   * anyway; the point still drops on its own in that case, unchanged from
   * before this WP). Both calls are skipped together when Ctrl/Cmd's
   * measure-only mode (`createGuides`, WP-7 item 1) is active — the
   * measurement/readout above still completes either way.
   */
  private _commitMeasure(
    p0: [number, number, number],
    p1: [number, number, number],
    onGeometry: boolean,
  ): void {
    const dx = p1[0] - p0[0], dy = p1[1] - p0[1], dz = p1[2] - p0[2]
    const dist = Math.sqrt(dx * dx + dy * dy + dz * dz)
    this._pushReadout(formatLength(dist), false)

    if (!onGeometry && this.createGuides) {
      try {
        if (dist > 1e-9) {
          this.wasmScene.add_guide_line(p0[0], p0[1], p0[2], dx / dist, dy / dist, dz / dist)
        }
        this.wasmScene.add_guide_point(p1[0], p1[1], p1[2])
        this.onGuideCreated()
      } catch (err) {
        const raw = err instanceof Error ? err.message : String(err)
        this.onToast(`Couldn't create guide: ${raw}`)
      }
    }
    this._resetToIdle()
  }

  /** Commit an exact typed distance (Enter in the VCB), per stage. */
  private _commitFromTyped(dist: number): void {
    if (this.stage.kind === 'parallel') {
      const { edgePoint, edgeDir, origin, offsetPlane } = this.stage
      let dir: [number, number, number]
      if (offsetPlane !== null) {
        // A resolved offset plane/axis: commit along its OWN `u`, signed by
        // which side of `edgePoint` the current preview sits on — not the
        // raw normalized (origin − edgePoint) vector, which can drift off
        // the resolved plane by floating-point error even though `origin`
        // was itself computed as a scalar multiple of `u`.
        const rel = subV3(origin, edgePoint)
        const relLen = Math.sqrt(rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2])
        if (relLen < 1e-9) {
          // No offset direction yet (e.g. an axis lock set before the first
          // pointer move, so `origin` still equals `edgePoint`) — nothing to
          // commit. Without this guard, `dot(rel, u) < 0` defaults to `+1`
          // for a zero `rel`, committing to an arbitrary side rather than
          // canceling like the free-offset branch below already does for
          // the same degenerate input. Nothing committed — freeze would
          // show a misleading stale typed echo, so clear instead.
          this._resetToIdle('clear')
          return
        }
        const sign = dotV3(rel, offsetPlane.u) < 0 ? -1 : 1
        dir = [offsetPlane.u[0] * sign, offsetPlane.u[1] * sign, offsetPlane.u[2] * sign]
      } else {
        // Signed offset direction: from edgePoint toward the current origin
        // (i.e. whichever side of the edge the cursor is currently on).
        const rel: [number, number, number] = [
          origin[0] - edgePoint[0],
          origin[1] - edgePoint[1],
          origin[2] - edgePoint[2],
        ]
        const relLen = Math.sqrt(rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2])
        if (relLen < 1e-9) {
          // No offset direction yet (cursor sitting on the edge) — nothing
          // committed, so clear rather than freeze.
          this._resetToIdle('clear')
          return
        }
        dir = [rel[0] / relLen, rel[1] / relLen, rel[2] / relLen]
      }
      const newOrigin = pointAlong(edgePoint, dir, dist)
      this._commitParallelGuide(newOrigin, edgeDir)
      return
    }

    if (this.stage.kind === 'measure') {
      const { p0, p0OnGeometry, p1, onGeometry } = this.stage
      const rel: [number, number, number] = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]]
      const relLen = Math.sqrt(rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2])
      let dir: [number, number, number]
      if (relLen < 1e-9) {
        // Degenerate — the cursor is still essentially at p0. Under a held
        // axis lock, default to the locked axis's own POSITIVE direction
        // (mirrors MoveTool's `_commitFromTyped`: its signed-direction
        // computation degenerates to the same +axis default when `dest`
        // hasn't moved from `base` either) rather than hardcoded world X,
        // which would silently commit along the wrong axis whenever a Y or Z
        // lock is held. With no lock, the existing [1,0,0] default is
        // unchanged.
        if (this.lockAxis !== null) {
          const frame = getDrawingAxes(this.wasmScene)
          dir = [frame.x, frame.y, frame.z][this.lockAxis]
        } else {
          dir = [1, 0, 0]
        }
      } else {
        dir = [rel[0] / relLen, rel[1] / relLen, rel[2] / relLen]
      }
      const endpoint = pointAlong(p0, dir, dist)

      // Resize-the-model arm (design tool-parity §3, SketchUp's flow):
      // typing a length between two REAL, already-measured points — both
      // ends resting on real geometry, so "this distance" names something
      // concrete — arms a confirmation instead of the ordinary guide-point
      // commit below. An empty-space endpoint (either end) or a degenerate
      // current/typed length has no meaningful "this distance", and falls
      // through to that ordinary commit unchanged.
      if (p0OnGeometry && onGeometry && relLen > 1e-6 && dist > 1e-6) {
        this.stage = {
          kind: 'pendingRescale',
          p0,
          endpoint,
          factor: dist / relLen,
          createGuidesAtArm: this.createGuides,
          fromRecall: false,
        }
        this.typed = ''
        this._clearPreviewLine()
        // Freeze the widget at the TARGET/typed distance the confirmation
        // dialog is now asking about (tape-measure-rework part 1.5), rather
        // than blanking it.
        this._pushReadout(formatLength(dist), true)
        this.onRescaleArmed({ currentDistance: relLen, typedDistance: dist, factor: dist / relLen })
        return
      }

      // Typed-exact endpoints are, by definition, not resting on picked
      // geometry — always drop a guide point so the typed distance is preserved.
      this._commitMeasure(p0, endpoint, false)
    }
  }

  /**
   * Arm the "resize the model?" confirmation from an idle-stage RECALL
   * (tape-measure-rework part 2) — SketchUp's paradigm: click both points
   * normally, see the final distance sit in the corner widget, THEN type a
   * new length and press Enter, with no need for mouse and keyboard "in
   * play" at the same time. Mirrors `_commitFromTyped`'s live arm exactly,
   * just computed from `_recall`'s saved world points instead of the live
   * `measure` stage's fields.
   */
  private _armRescaleFromRecall(dist: number): void {
    const r = this._recall
    if (r === null) return
    if (dist <= 1e-6) {
      this.onToast('Type a length greater than zero to resize the model.')
      return
    }
    const dir: [number, number, number] = [
      (r.p1[0] - r.p0[0]) / r.dist, (r.p1[1] - r.p0[1]) / r.dist, (r.p1[2] - r.p0[2]) / r.dist,
    ]
    const factor = dist / r.dist
    this.stage = {
      kind: 'pendingRescale',
      p0: r.p0,
      endpoint: pointAlong(r.p0, dir, dist),
      factor,
      createGuidesAtArm: this.createGuides,
      fromRecall: true,
    }
    this.typed = ''
    this._clearPreviewLine()
    this._pushReadout(formatLength(dist), true)
    this.onRescaleArmed({ currentDistance: r.dist, typedDistance: dist, factor })
  }

  /**
   * Apply the armed rescale (the confirmation modal's "Confirm"). No-op if
   * nothing is armed (stray/late call — e.g. Escape and the dialog's own
   * button both resolving the same arm). Errors (a refused factor) toast;
   * either way the tool returns to idle.
   */
  confirmRescale(): void {
    if (this.stage.kind !== 'pendingRescale') return
    const { factor } = this.stage
    try {
      this.wasmScene.rescale_document(factor)
      this.onRescaleApplied(factor)
    } catch (err) {
      const raw = err instanceof Error ? err.message : String(err)
      this.onToast(`Couldn't resize the model: ${raw}`)
    }
    // The whole document just rescaled — the saved world points (if any
    // recall was still around) are now stale. Don't rescale them, just drop
    // them (tape-measure-rework part 2).
    this._recall = null
    this._resetToIdle()
  }

  /**
   * Decline the armed rescale (the confirmation modal's "Cancel", or Esc):
   * falls through to the NORMAL guide-point commit the typed distance would
   * have produced without the arm (design tool-parity §3 — "cancel reverts
   * to the normal guide behavior"). No-op if nothing is armed.
   *
   * The fallback commit uses `createGuidesAtArm` — the `createGuides`
   * snapshot taken when this stage was ARMED — rather than the live
   * `createGuides`, which may have drifted while the confirmation dialog was
   * open (Ctrl/Cmd toggles it from window-level listeners that don't gate on
   * this dialog). `_commitMeasure` resets `createGuides` back to `true` via
   * `_resetToIdle()` regardless of the snapshot's value, so overwriting it
   * here only governs this one fallback call.
   */
  cancelRescale(): void {
    if (this.stage.kind !== 'pendingRescale') return
    if (this.stage.fromRecall) {
      // A recalled arm has no pending gesture to fall back to — there is no
      // guide-point commit to make, since nothing was ever "in flight"; just
      // drop back to idle and restore the frozen original reading. The
      // recall itself survives (unchanged by this whole detour), so typing
      // the same or a different length again immediately re-arms it.
      const dist = this._recall?.dist ?? null
      this._resetToIdle('clear')
      if (dist !== null) this._pushReadout(formatLength(dist), true)
      return
    }
    const { p0, endpoint, createGuidesAtArm } = this.stage
    this.createGuides = createGuidesAtArm
    this._commitMeasure(p0, endpoint, false)
  }

  /**
   * Return to `idle`, clearing every gesture-scoped field. `readout`
   * (tape-measure-rework part 1) governs what happens to the measurement
   * widget: `'freeze'` (the default — every call site where something
   * genuinely committed: a guide, a measurement, an applied/declined
   * rescale) re-pushes `_lastReadout` with `frozen: true`, keeping the last
   * reading on screen; `'clear'` (an abort that committed nothing — `cancel()`,
   * a degenerate typed-offset bail in `_commitFromTyped`) blanks it instead,
   * since there's nothing genuine left to show.
   */
  private _resetToIdle(readout: 'freeze' | 'clear' = 'freeze'): void {
    this.stage = { kind: 'idle' }
    this.typed = ''
    // The frozen gesture plane is stale either way (committed or aborted) —
    // the NEXT gesture re-resolves it at its own first click. The idle lock
    // itself is NOT cleared here — it survives a completed gesture, same as
    // the draw tools; only `cancel()` (idle Escape / explicit reset) clears it.
    this._gesturePlane = null
    // The mid-gesture offset-axis lock is scoped to ONE gesture (unlike
    // `idlePlaneLock`) — always cleared here, whether the gesture committed
    // or was aborted.
    this.offsetLock = null
    this._offsetAxisRejectedHint = null
    // Same scoping as `offsetLock`, for the `measure` stage's own lock (WP-5).
    this.lockAxis = null
    // Scoped to ONE gesture too (WP-7 item 2) — a Shift latch/pending state
    // never survives past the gesture it applied to.
    this._shiftAxisLock = false
    this._shiftLatchPending = false
    // Ctrl/Cmd's measure-only suppression (WP-7 item 1) is also scoped to
    // ONE gesture — the next one starts creating guides again by default.
    this.createGuides = true
    this._snapProjected = false
    this._clearPreviewLine()
    this._pushReadout(readout === 'freeze' ? this._lastReadout : '', readout === 'freeze')
  }

  /**
   * Resolve the `measure`-stage TARGET point for a fresh snap (WP-5): with
   * no `_gesturePlane` frozen, the raw snap point unchanged — today's
   * angled-measurement behavior (an off-plane point legitimately becomes
   * the literal destination). With a `_gesturePlane` frozen, `snapConstraint`
   * ALSO passes `offPlanePoints: true` so precise off-plane point
   * candidates (endpoints, midpoints, centers…) stay reachable instead of
   * being filtered out by the plane constraint — but the measurement still
   * has to land ON the frozen plane, so an off-plane result is projected
   * onto it here. Updates `_snapProjected` (see `snapProjected`) to whether
   * this call's projection actually moved the point.
   *
   * While `lockAxis` is held, this always returns the raw snap point
   * unprojected: `snapConstraint()` no longer asks the kernel for
   * `constraintPlane`/`offPlanePoints` at all while a lock is held (see its
   * doc/comment), so there is no frozen-plane projection to apply here — the
   * raw snap IS the free result `_measurePoint` (below) itself then projects
   * onto the locked axis. `_measurePoint` is the entry point BOTH
   * `onPointerMove`/`onPointerDown` actually call; this method stays private
   * to it (and to itself, for the frozen-plane case with no lock held).
   */
  private _measureTarget(snap: Snap): [number, number, number] {
    const raw: [number, number, number] = [snap.x, snap.y, snap.z]
    // While `lockAxis` is held, `snapConstraint()` no longer asks the kernel
    // for `constraintPlane`/`offPlanePoints` at all (see its doc/comment) —
    // the raw snap point IS exactly what the free/unconstrained snap
    // produced, so projecting it onto `_gesturePlane` here would reproduce
    // the same plane-vs-lock conflict that precedence change exists to
    // avoid (`_measurePoint` projects it onto the locked axis instead).
    if (this.lockAxis !== null || this._gesturePlane === null || isPointOnDrawPlane(raw, this._gesturePlane)) {
      this._snapProjected = false
      return raw
    }
    this._snapProjected = true
    return projectPointOntoPlane(raw, this._gesturePlane.origin, this._gesturePlane.normal)
  }

  /**
   * Resolve the `measure`-stage TARGET point for a fresh snap, accounting
   * for a mid-gesture axis lock (the locked-axis-projection fix, section C)
   * — the single entry point BOTH `onPointerMove`'s preview and
   * `onPointerDown`'s commit call, so a click always commits to exactly the
   * point the preview was just showing.
   *
   * With no lock held, this is exactly `_measureTarget(snap)` — unchanged.
   *
   * With `lockAxis` held: `_measureTarget` already returns the raw,
   * unprojected FREE snap (see its doc) — `snapConstraint()` sent nothing
   * lock-related to the kernel, so that free result is magnetized to real
   * geometry exactly as if no lock were active. The reported station along
   * the locked axis is then:
   * - for real geometry (any snap kind but `'ground'`/`'plane'`): the free
   *   point's own component along the axis, `dot(free - p0, axis)`.
   * - for `'ground'`/`'plane'` (`SnapService`'s own ray/ground-plane
   *   fallback, engaged when nothing real is under the cursor — see
   *   `snapService.ts`): NOT the free point's own coordinates — those are
   *   only correct when the ray happens to be perpendicular to the axis, and
   *   are wrong (or nonexistent, for a ray that never reaches the ground at
   *   all) everywhere else. `stationOnAxisFromRay` derives the station
   *   directly from the cursor RAY against the locked axis line instead —
   *   the TS mirror of the kernel's own `closest_point_on_line_to_ray`.
   *
   * `_snapProjected` (see `snapProjected`) is set here to whether the
   * axis-projected point actually differs from the free point by more than
   * `1e-9` — true whenever the free snap doesn't happen to already lie
   * exactly on the locked axis, which in practice is most of the time a
   * locked axis is held over real geometry. That's the intended, disclosed
   * behavior (a locked result that doesn't literally land on the snapped
   * point discloses that fact), not a bug to suppress.
   */
  private _measurePoint(snap: Snap, ray: Ray): [number, number, number] {
    const free = this._measureTarget(snap)
    if (this.lockAxis === null || this.stage.kind !== 'measure') return free

    const { p0 } = this.stage
    const frame = getDrawingAxes(this.wasmScene)
    const axis = [frame.x, frame.y, frame.z][this.lockAxis]
    const t = snap.kind === 'ground' || snap.kind === 'plane'
      ? stationOnAxisFromRay(p0, axis, ray.origin, ray.direction)
      : dotV3(subV3(free, p0), axis)
    const projected = pointAlong(p0, axis, t)
    this._snapProjected = Math.hypot(
      free[0] - projected[0],
      free[1] - projected[1],
      free[2] - projected[2],
    ) > 1e-9
    return projected
  }

  private _reportOffsetOrTyped(offsetDist: number): void {
    if (this.typed !== '') {
      this._pushReadout(this._typedReadout(), false)
      return
    }
    this._pushReadout(formatLength(offsetDist), false)
  }

  private _reportDistanceOrTyped(p0: [number, number, number], p1: [number, number, number]): void {
    if (this.typed !== '') {
      this._pushReadout(this._typedReadout(), false)
      return
    }
    const dx = p1[0] - p0[0], dy = p1[1] - p0[1], dz = p1[2] - p0[2]
    this._pushReadout(formatLength(Math.sqrt(dx * dx + dy * dy + dz * dz)), false)
  }

  /**
   * Rebuild the preview line(s) in the preview group, dispatched by stage.
   * Removes the previous preview (if any) first.
   *
   * - `parallel` mode: the long guide-preview line through (origin, edgeDir),
   *   colored `GUIDE_COLOR` (neutral — it isn't the axis being measured,
   *   just the guide's own run direction), PLUS a connector segment from
   *   `edgePoint` to `origin` (WP-6) — this visually draws the resolved
   *   offset itself, so the direction/distance the guide got pulled from its
   *   source edge is legible at a glance. The connector is colored by the
   *   axis-color rule (`_axisColorFor`) and is skipped entirely when the
   *   offset is degenerate (cursor still effectively on the edge).
   * - `measure` mode: a single segment from p0 to p1, colored by the same
   *   axis-color rule as the connector above (an on-axis measurement reads
   *   as that axis's color; anything else falls back to `GUIDE_COLOR`), PLUS
   *   — while `lockAxis` is held (the locked-axis-projection fix, section
   *   E) — a leader-line segment through `p0` spanning
   *   `±AXIS_LEADER_HALF_LENGTH` along the locked axis, colored the same
   *   way. `CueLayer`'s own dashed direction cue used to draw this (driven
   *   by the kernel-returned `Snap.direction`, which `SnapLock::Axis` used
   *   to set), but since this lock no longer reaches the kernel at all,
   *   `Snap.direction` instead reflects the free snap's OWN natural
   *   direction (an edge's direction, or none) — so this tool draws its own
   *   leader line now, matching `CueLayer`'s prior visual size exactly.
   */
  private _updatePreviewLine(): void {
    this._clearPreviewLine()

    // The preview is a solid line (the placed guide renders dashed). A
    // screen-constant manual dash here would just duplicate SceneRenderer's
    // dash logic for a transient overlay; a thin solid line reads clearly and
    // avoids the metre-sized-dash problem entirely.
    if (this.stage.kind === 'parallel') {
      const { origin, edgeDir, edgePoint } = this.stage
      const nx = edgeDir[0] * GUIDE_HALF_LENGTH
      const ny = edgeDir[1] * GUIDE_HALF_LENGTH
      const nz = edgeDir[2] * GUIDE_HALF_LENGTH
      const guidePts = new Float32Array([
        origin[0] - nx, origin[1] - ny, origin[2] - nz,
        origin[0] + nx, origin[1] + ny, origin[2] + nz,
      ])
      this.previewLine = this._makePreviewSegment(guidePts, GUIDE_COLOR)

      const rel = subV3(origin, edgePoint)
      const offsetLen = Math.sqrt(rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2])
      if (offsetLen > 1e-9) {
        const dir: [number, number, number] = [rel[0] / offsetLen, rel[1] / offsetLen, rel[2] / offsetLen]
        const connectorPts = new Float32Array([
          edgePoint[0], edgePoint[1], edgePoint[2],
          origin[0], origin[1], origin[2],
        ])
        this.previewConnector = this._makePreviewSegment(connectorPts, this._axisColorFor(dir))
      }
    } else if (this.stage.kind === 'measure') {
      const { p0, p1 } = this.stage
      const rel: [number, number, number] = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]]
      const len = Math.sqrt(rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2])
      const color = len > 1e-9
        ? this._axisColorFor([rel[0] / len, rel[1] / len, rel[2] / len])
        : GUIDE_COLOR
      const pts = new Float32Array([p0[0], p0[1], p0[2], p1[0], p1[1], p1[2]])
      this.previewLine = this._makePreviewSegment(pts, color)

      if (this.lockAxis !== null) {
        // The kernel no longer draws this (see this method's doc) — an
        // ADDITIONAL segment through p0, not a replacement for the one above.
        const frame = getDrawingAxes(this.wasmScene)
        const axis = [frame.x, frame.y, frame.z][this.lockAxis]
        const nx = axis[0] * AXIS_LEADER_HALF_LENGTH
        const ny = axis[1] * AXIS_LEADER_HALF_LENGTH
        const nz = axis[2] * AXIS_LEADER_HALF_LENGTH
        const axisPts = new Float32Array([
          p0[0] - nx, p0[1] - ny, p0[2] - nz,
          p0[0] + nx, p0[1] + ny, p0[2] + nz,
        ])
        this.previewAxisGuide = this._makePreviewSegment(axisPts, this._axisColorFor(axis))
      }
    }
  }

  /**
   * The axis-color rule (WP-6): `direction` colored like a drawing axis when
   * it lies within `AXIS_ALONG_EDGE_DEG` (3°) of one — the same tolerance
   * `tapeOffset.ts` already uses for "this direction runs along that axis" —
   * else the neutral `GUIDE_COLOR` fallback. Theme- and drawing-axes-frame-
   * aware, unlike `MoveTool`'s hardcoded `AXIS_COLOR` or `CueLayer`'s
   * largest-absolute-component heuristic (neither respects a moved frame or
   * the current theme) — deliberately not reused here.
   */
  private _axisColorFor(direction: readonly [number, number, number]): number {
    return axisColorForDirection(
      direction,
      AXIS_ALONG_EDGE_COS,
      axisColorsForTheme(getResolvedTheme()),
      getDrawingAxes(this.wasmScene),
    )?.color ?? GUIDE_COLOR
  }

  /** Build one preview `LineSegments` (2-point) from `pts`, add it to the
   *  preview group, and return it — shared by the guide-preview line and the
   *  offset connector so both get identical construction discipline
   *  (`LineBasicMaterial`, `depthTest: false`) and disposal (`_clearPreviewLine`). */
  private _makePreviewSegment(pts: Float32Array, color: number): THREE.LineSegments {
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(pts, 3))
    const mat = new THREE.LineBasicMaterial({ color, depthTest: false })
    const line = new THREE.LineSegments(geo, mat)
    this.preview.add(line)
    return line
  }

  private _clearPreviewLine(): void {
    this.previewLine = this._disposePreviewSegment(this.previewLine)
    this.previewConnector = this._disposePreviewSegment(this.previewConnector)
    this.previewAxisGuide = this._disposePreviewSegment(this.previewAxisGuide)
  }

  /** Dispose one preview segment's geometry/material and remove it from the
   *  preview group, if present. Always returns null, so callers can write
   *  `this.field = this._disposePreviewSegment(this.field)`. */
  private _disposePreviewSegment(line: THREE.LineSegments | null): null {
    if (line === null) return null
    line.geometry.dispose()
    if (line.material instanceof THREE.Material) {
      line.material.dispose()
    }
    this.preview.remove(line)
    return null
  }
}
