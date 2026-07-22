/**
 * ArcTool — SketchUp-style 2-point arc, drawn as a faceted polyline chain
 *. Mirrors CircleTool: no kernel change, the arc decomposes into N
 * chained `sketch_add_segment` calls (plane mode) or one `split_face` call
 * with the polyline path (face mode — the arc is an OPEN chain, so it uses
 * LineTool's boundary-to-boundary `split_face`, not CircleTool's closed-loop
 * `split_face_inner`).
 *
 * Three-click gesture (both modes):
 *   1. Click endpoint A (with inference snapping).
 *   2. Click endpoint B — the chord (axis/inference snapping applies).
 *      Rubber-band preview: the chord segment A→cursor.
 *   3. Move perpendicular to the chord to pull out the bulge (live faceted
 *      arc preview + radius readout); click to commit.
 *   Esc steps back one stage: bulge → chord (A kept), chord → idle.
 *
 * Mode selection mirrors CircleTool/LineTool exactly: decided per pointer
 * event by what's under the cursor — face mode when an ELIGIBLE Object face
 * is there (see `faceDraw.ts` for the shared plain-object policy; the
 * entered object only, inside a context), plane mode otherwise.
 *
 * Plane mode (sketches on any plane — design doc §1/§4): the drawing plane
 * is resolved once, at the FIRST click of a gesture (endpoint A), and
 * frozen for the rest of the gesture:
 *   - A top-level hover over a committed sketch whose plane is non-ground
 *     (`pick_sketch` + `planeFromSketch`) adopts THAT sketch's plane —
 *     SKETCH MODE — and the arc's segments land in that one sketch
 *     (`SketchTarget.existing`).
 *   - Otherwise the plane is the ground plane — PLANE MODE, today's
 *     behavior — segments land in the shared per-plane cached sketch
 *     (`SketchTarget.plane`; `begin_ground_sketch()` on a cache miss).
 *   On the ground plane every point is computed by the EXACT legacy 2D
 *   chord/sagitta math (z = 0, world X/Y basis) — bit-identical committed
 *   coordinates. On any other plane, the arc is built in the plane's own
 *   in-plane basis (`facePlaneBasis`/`arcPolylineOnPlane` — the same
 *   toolbox face mode already proved on arbitrary planes), and the cursor
 *   is the snap (already plane-constrained via `snapConstraint`) or, absent
 *   one, ray∩plane.
 *
 * Face mode (an eligible Object face is under the cursor): all three points
 * lie in the picked face's plane via `snapConstraint`; commits imprint the
 * face (`split_face`/`split_face_inner`) instead of drawing into a sketch.
 *
 * Degenerate guards (constants in arcMath.ts — no inline epsilons):
 *   - zero/short chord (B on A): the B click is ignored.
 *   - flat bulge (|sagitta| < ARC_MIN_SAGITTA_M): the commit click is refused
 *     and the measurement line hints to pull out the bulge.
 *
 * VCB : typed length entry mirrors LineTool's
 * mechanism exactly (`editLengthBuffer` builds the raw string,
 * `parseLengthToMeters` resolves it on Enter, `capturingInput()` gates when
 * the Viewport routes keys here) but means something different at each
 * stage, since an arc has two distances to place, not one:
 *
 *   - Chord stage (A placed, B not yet): typed length commits endpoint B at
 *     exactly that distance from A along the LIVE cursor direction (the
 *     same semantics as LineTool's typed segment length). Refused — buffer
 *     kept, gesture unchanged — when there's no cursor direction yet (mouse
 *     hasn't moved off A) or the typed distance is below ARC_MIN_CHORD_M.
 *
 *   - Bulge stage (A, B placed): typed length sets |sagitta| to exactly that
 *     distance, on whichever side of the chord the cursor is CURRENTLY on
 *     (sign taken from the live cursor's sagitta). If the cursor sits
 *     exactly on the chord (sagitta ~0) or hasn't moved since B was placed,
 *     this falls back to the last nonzero side seen during this bulge stage;
 *     with no side ever established, the commit is refused (mirrors the
 *     pointer path's flat-bulge refusal — same hint text). This resolution
 *     order (live side, then last-seen side, then refuse) was picked over
 *     "always refuse without a live side" because it lets a user nudge the
 *     mouse to declare a side ONCE and then type an exact radius/sagitta
 *     repeatedly without having to keep the mouse perfectly off the chord.
 *
 * The typed buffer is cleared on every stage transition (pointer- or
 * VCB-driven) and the live measurement readout shows the typed text instead
 * of the pointer-derived one whenever the buffer is non-empty — identical
 * to LineTool's `_reportMeasurement` convention.
 *
 * Completion modes (SketchUp's Alt/Option arc↔pie toggle, plus a chord
 * close): pressing Alt mid-gesture cycles open → pie → segment. `open`
 * commits the bare arc as today. `pie` closes it to the arc center with two
 * radii; `segment` closes it with the chord — both commit a closed profile,
 * so a region (plane mode) or a face imprint (face mode, via
 * `split_face_inner` like CircleTool) appears immediately and push/pull
 * works. The live preview draws the closing edges and the radius readout
 * names the mode. The mode persists across commits until the tool is
 * switched (tools are recreated on every switch, so a fresh Arc tool always
 * starts `open`) or the document is replaced (`onDocumentReset`).
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { V3 } from '../viewport/geoHelpers'
import { facePlaneBasis, rayPlaneIntersect } from '../viewport/geoHelpers'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { makeFatSegments, disposeFatSegments, PREVIEW_LINE_STYLE } from '../viewport/fatLine'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { segmentLength, directionBetween } from './lineInput'
import { editLengthBuffer, isLengthInputKey, pointAlong, nextIdlePlaneLock, AXIS_LOCK_COLOR_NAMES } from './moveInput'
import { runSketchGesture, makeSketchPlaneCache, type SketchPlaneCache, type SketchTarget } from './sketchGesture'
import { pointOnPlane, drawPlaneCue, isGroundPlane, SketchPickCache, resolveIdleDrawTarget, resolveClickDrawTarget, type DrawPlane } from './drawPlane'
import { FacePickCache, defaultFaceEligible, type FaceEligible } from './faceDraw'
import {
  ARC_MIN_CHORD_M,
  ARC_MIN_SAGITTA_M,
  arcFromChord,
  arcPolylineOnPlane,
  chordSagitta,
  type Vec2,
} from './arcMath'

export type ArcCommitResult = {
  sketchHandle: bigint
  /** Handles of regions created by the last segment (may be empty if not yet closed) */
  regionsCreated: bigint[]
}

export type OnArcCommit = (result: ArcCommitResult) => void
export type OnFaceImprint = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/** Measurement-line hint shown when a commit click lands with a flat bulge. */
const FLAT_BULGE_HINT = 'Pull out the bulge'

/** How the committed arc is closed (Alt cycles, in this order). */
export type ArcCompletion = 'open' | 'pie' | 'segment'

const COMPLETION_CYCLE: readonly ArcCompletion[] = ['open', 'pie', 'segment']

/** Readout label per mode ('open' shows the bare radius). */
const COMPLETION_LABEL: Record<ArcCompletion, string> = {
  open: '',
  pie: 'Pie',
  segment: 'Segment',
}

/** Plane gesture: idle → endpoint A placed (chord) → chord (A,B) placed
 *  (bulge), on a frozen `DrawPlane`/`SketchTarget`. */
type PlaneStage =
  | { kind: 'idle' }
  | { kind: 'chord'; plane: DrawPlane; target: SketchTarget; a: V3 }
  | { kind: 'bulge'; plane: DrawPlane; target: SketchTarget; a: V3; b: V3 }

/** Face gesture: same stages, on a locked face plane. */
type FaceStage =
  | { kind: 'idle' }
  | {
      kind: 'chord'
      object: bigint
      face: bigint
      normal: V3
      /** A world-space point that lies on the face plane (the first click position) */
      planePoint: V3
      a: V3
    }
  | {
      kind: 'bulge'
      object: bigint
      face: bigint
      normal: V3
      planePoint: V3
      a: V3
      b: V3
    }

/** The arc chain a commit needs: the polyline (arc facets plus any
 *  completion-mode closing vertices), how many of the LEADING vertices
 *  belong to the arc's own curve chain (closing edges stay plain lines),
 *  and the arc's analytic circle (center + radius) to ride on the curve
 *  chain, if resolvable. */
type ArcChain = { chain: V3[]; curveSegments: number; geom: { center: V3; radius: number } | null }


export class ArcTool implements Tool {
  readonly name = 'Arc'

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    const stage = this.faceStage.kind !== 'idle' ? this.faceStage.kind : this.planeStage.kind
    if (stage === 'chord') {
      return "Click the arc's second endpoint — or type an exact chord length."
    }
    if (stage === 'bulge') {
      return 'Click to set the curve — Alt cycles open arc / pie / segment; or type an exact bulge.'
    }
    if (this.idlePlaneLock !== null) {
      return `Locked to the ${AXIS_LOCK_COLOR_NAMES[this.idlePlaneLock]} plane — click to start; same arrow or Esc unlocks.`
    }
    return "Click the arc's first endpoint — on the ground plane or any face or sketch."
  }

  private planeStage: PlaneStage = { kind: 'idle' }
  private faceStage: FaceStage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnArcCommit
  private onFaceImprint: OnFaceImprint
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** Cached plane-mode sketch handles — the Viewport passes one cache
   *  shared by every draw tool, so mixed-tool profiles land in a single
   *  sketch per plane. */
  private readonly sketchCache: SketchPlaneCache

  /** The currently active editing context (entered object), or null at top level. */
  private _activeContext: bigint | null = null

  /** Completion mode for committed arcs — Alt cycles it mid-gesture and it
   *  persists across commits (see module doc). */
  private completion: ArcCompletion = 'open'

  /** VCB buffer — raw string being typed by the user (length, in display units). */
  private typed: string = ''

  /** Last live cursor position seen this stage (plane/face — only one is
   *  ever populated at a time, mirroring LineTool's pair of fields). Used
   *  for the typed-commit chord direction and the bulge-side resolution. */
  private _lastPlaneCursor: V3 | null = null
  private _lastFaceCursor: V3 | null = null

  /** The last NONZERO sagitta sign seen during the current bulge stage —
   *  the fallback when a typed bulge commit's live cursor sits exactly on
   *  the chord (see module doc's VCB section). Reset every time a fresh
   *  bulge stage begins (or the gesture ends). */
  private _lastSagittaSign: -1 | 1 | null = null

  /** Idle plane lock (design §5.2): while FULLY idle (no chord/bulge
   *  stage), an arrow key locks the future plane's NORMAL to a world axis
   *  (0=X/red, 1=Y/green, 2=Z/blue — `arrowToAxis`); the same arrow again,
   *  or Escape/ArrowDown, clears it. An ACTIVE lock overrides face pick and
   *  sketch-hover adoption on the next click (SketchUp: an explicit lock
   *  beats inference) — see `_currentMode`/`_resolveClickTarget`. Survives
   *  a completed gesture (cleared only by `cancel()`, which
   *  `onDocumentReset()`/`setActiveContext()` already route through). */
  private idlePlaneLock: 0 | 1 | 2 | null = null

  /** The last hover point seen while idle-locked (design §6 bullet 1) — feeds
   *  `activeDrawPlaneCue()`'s idle-locked case. Reset to null whenever the
   *  lock itself changes (a fresh lock has no hover yet) and by `cancel()`. */
  private _lastIdleHoverPoint: V3 | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnArcCommit,
    onToast: OnToast,
    onFaceImprint: OnFaceImprint,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
    sketchCache: SketchPlaneCache = makeSketchPlaneCache(),
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onCommit = onCommit
    this.onFaceImprint = onFaceImprint
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
    this.sketchCache = sketchCache
  }

  /** Set the active editing context (entered object), or null for top level. */
  setActiveContext(id: bigint | null): void {
    if (id === this._activeContext) return  // re-asserting the same context must not abort an in-progress gesture
    this._activeContext = id
    this.cancel()
  }

  /** Per-pointer-event `pick_face` memo — see `FacePickCache` in faceDraw.ts. */
  private readonly _pickCache = new FacePickCache()
  /** Per-pointer-event `pick_sketch` memo — see `SketchPickCache` in drawPlane.ts. */
  private readonly _sketchPickCache = new SketchPickCache()

  /** Optional richer eligibility, injected by the Viewport (which knows the
   *  full group/instance context path the tool can't see). Null = the shared
   *  default policy in faceDraw.ts. */
  private _faceEligible: FaceEligible | null = null
  setFaceEligibility(pred: FaceEligible | null): void {
    this._faceEligible = pred
  }

  /** Plain objects are directly drawable at the top level; inside an entered
   *  object context only that object's faces are — see faceDraw.ts. */
  private _isEligible(objectHandle: bigint, instanceHandle: bigint | undefined): boolean {
    if (this._faceEligible !== null) return this._faceEligible(objectHandle, instanceHandle)
    return defaultFaceEligible(this.wasmScene, this._activeContext, objectHandle, instanceHandle)
  }

  /** The eligible face under `ray`, or null (memoized per pointer event). */
  private _eligiblePickFor(ray: Ray): { object: bigint; face: bigint } | null {
    return this._pickCache.pickFor(this.wasmScene, ray, (object, instance) =>
      this._isEligible(object, instance))
  }

  /**
   * Resolve the plane/target an IDLE gesture would anchor onto at `ray`
   * (design §1/§4): a top-level `pick_sketch` hit whose plane is non-ground
   * adopts that sketch (SKETCH MODE); otherwise the ground plane (PLANE
   * MODE, today's behavior). Only reachable when `_currentMode` has already
   * ruled out face mode (which takes priority), so no `_activeContext`
   * re-check is needed here.
   */
  private _resolveIdleTarget(ray: Ray): { plane: DrawPlane; target: SketchTarget } {
    return resolveIdleDrawTarget(this.wasmScene, this._sketchPickCache, ray)
  }

  /**
   * Resolve the plane/target the FIRST click of a gesture anchors onto
   * (design §5.2): an ACTIVE idle plane lock beats face pick and
   * sketch-hover adoption — the locked plane passes through `snap`'s point
   * (free/unconstrained, per `snapConstraint`'s idle-lock branch above), so
   * clicking a solid's corner starts a vertical sketch at that corner.
   * Falls back to `_resolveIdleTarget` (face/sketch/ground) when no lock is
   * active. Returns `null` only when a lock is active but there's no snap
   * point yet (nothing to click through).
   */
  private _resolveClickTarget(snap: Snap | null, ray: Ray): { plane: DrawPlane; target: SketchTarget } | null {
    return resolveClickDrawTarget(this.wasmScene, this._sketchPickCache, this.idlePlaneLock, snap, ray)
  }

  /** The cursor's position on `plane`. On the ground plane this is EXACTLY
   *  `[snap.x, snap.y, 0]` (no basis math, snap required) — the legacy fast
   *  path, bit-identical to before this module existed. On any other plane:
   *  the snap (already plane-constrained via `snapConstraint`) if present,
   *  else ray∩plane. */
  private _planeCursor(snap: Snap | null, ray: Ray, plane: DrawPlane): V3 | null {
    if (plane.ground) {
      if (snap === null) return null
      return [snap.x, snap.y, 0]
    }
    if (snap !== null) return [snap.x, snap.y, snap.z]
    return pointOnPlane(ray, plane)
  }

  /**
   * Decide which mode governs the NEXT pointer event (same contract as the
   * other draw tools): sticky mid-gesture; always face mode inside an
   * entered object context (scoped drawing — no top-level plane sketch from
   * inside); else decided by what's under the cursor.
   */
  private _currentMode(ray?: Ray): 'face' | 'plane' {
    if (this.faceStage.kind !== 'idle') return 'face'
    if (this.planeStage.kind !== 'idle') return 'plane'
    // Inside an entered object context, drawing stays scoped to that
    // object's faces — a click elsewhere is ignored by the face handler
    // rather than falling through to a top-level plane sketch.
    if (this._activeContext !== null) return 'face'
    // An active idle plane lock beats face pick and sketch-hover adoption
    // (design §5.2) — the user already chose a plane.
    if (this.idlePlaneLock !== null) return 'plane'
    if (ray === undefined) return 'plane'
    return this._eligiblePickFor(ray) !== null ? 'face' : 'plane'
  }

  /**
   * Provide a constraint plane for snap so off-plane/occluded geometry is
   * excluded while snapping during face-mode or non-ground plane/sketch-mode
   * drawing.
   *
   * - Face mode, gesture in progress: return the already-locked face plane.
   * - Plane mode, gesture in progress on a NON-ground plane (sketch mode):
   *   same — return the frozen plane. Ground-anchored: no constraint
   *   (today's behavior, unchanged).
   * - Idle: pick the hovered face (if an eligible one is under the cursor)
   *   and return its plane so the FIRST-click endpoint lands precisely on
   *   the face; absent that, a top-level hover over a non-ground sketch
   *   returns ITS plane.
   * - Otherwise (ground mode): return null (unconstrained).
   */
  snapConstraint(ray: Ray): { constraintPlane?: { point: [number, number, number]; normal: [number, number, number] } } | null {
    if (this.faceStage.kind !== 'idle') {
      // Gesture in progress: lock to the established face plane
      return {
        constraintPlane: {
          point: this.faceStage.planePoint,
          normal: this.faceStage.normal,
        },
      }
    }

    if (this.planeStage.kind !== 'idle') {
      if (this.planeStage.plane.ground) return null
      return {
        constraintPlane: {
          point: this.planeStage.plane.origin,
          normal: this.planeStage.plane.normal,
        },
      }
    }

    // Idle plane lock (design §5.2): the first click is FREE — no
    // constraint plane. The locked plane is derived FROM that click
    // (`_resolveClickTarget`), so constraining the snap here would be
    // circular. A lock also beats face pick / sketch-hover adoption, so
    // neither of those runs below while one is active.
    if (this.idlePlaneLock !== null) return null

    // Idle: face mode iff an eligible face is under the cursor
    const eligible = this._eligiblePickFor(ray)
    if (eligible !== null) {
      const a = this.wasmScene.face_plane(eligible.object, eligible.face)
      return {
        constraintPlane: {
          point: [a[0], a[1], a[2]],
          normal: [a[3], a[4], a[5]],
        },
      }
    }

    const { plane } = this._resolveIdleTarget(ray)
    if (!plane.ground) {
      return { constraintPlane: { point: plane.origin, normal: plane.normal } }
    }
    return null
  }

  /**
   * The drawing-plane cue the Viewport should render right now (design §6
   * bullet 1) — a grid patch on the active NON-ground plane, or null (ground
   * is covered by the world grid already). See `drawPlaneCue` in
   * `drawPlane.ts` for the two cases (anchored non-ground / idle-locked with
   * a tracked hover). "Anchored" here is either the chord or bulge stage —
   * both freeze the same plane/anchor for the rest of the gesture.
   */
  activeDrawPlaneCue(): { plane: DrawPlane; through: V3 } | null {
    if (this.faceStage.kind !== 'idle') {
      const basis = facePlaneBasis(this.faceStage.normal)
      if (basis === null) return null
      const anchoredPlane: DrawPlane = {
        origin: this.faceStage.planePoint,
        normal: this.faceStage.normal,
        u: basis.u,
        v: basis.v,
        ground: isGroundPlane(this.faceStage.planePoint, this.faceStage.normal),
      }
      return drawPlaneCue({
        anchoredPlane,
        anchoredThrough: this.faceStage.planePoint,
        idleLock: null,
        idleHover: null,
      })
    }
    if (this.planeStage.kind !== 'idle') {
      return drawPlaneCue({
        anchoredPlane: this.planeStage.plane,
        anchoredThrough: this.planeStage.a,
        idleLock: null,
        idleHover: null,
      })
    }
    return drawPlaneCue({
      anchoredPlane: null,
      anchoredThrough: null,
      idleLock: this.idlePlaneLock,
      idleHover: this._lastIdleHoverPoint,
    })
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (this._currentMode(ray) === 'face') {
      this._onPointerMoveFace(snap, ray)
    } else {
      this._onPointerMovePlane(snap, ray)
    }
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (this._currentMode(ray) === 'face') {
      this._onPointerDownFace(snap, ray)
    } else {
      this._onPointerDownPlane(snap, ray)
    }
  }

  /**
   * True while a gesture is in progress, so the Viewport routes keys here
   * (Escape stage-back, and — since the — typed VCB entry) instead
   * of treating letters as tool-switch shortcuts mid-gesture.
   */
  capturingInput(): boolean {
    return this.planeStage.kind !== 'idle' || this.faceStage.kind !== 'idle'
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      // Idle with an active plane lock: Escape clears the lock FIRST — only
      // a second Escape (already idle, unlocked) falls through to today's
      // idle-Escape behavior (design §5.2).
      if (!this.capturingInput() && this.idlePlaneLock !== null) {
        this.idlePlaneLock = null
        this._lastIdleHoverPoint = null
        return
      }
      // Aborting an in-progress gesture keeps the plane lock: the lock is
      // an idle aiming choice, cleared only by an idle Escape or toggle
      // (parity across all four draw tools — LineTool's _endChain path).
      const lock = this.idlePlaneLock
      this._stepBack()
      this.idlePlaneLock = lock
      return
    }

    if (!this.capturingInput()) {
      // Idle plane lock via arrow keys (design §5.2) — consumed by neither
      // hover nor preview, only by the next first click.
      if (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft' || ev.key === 'ArrowUp' || ev.key === 'ArrowDown') {
        this.idlePlaneLock = nextIdlePlaneLock(this.idlePlaneLock, ev.key)
        // A fresh/changed lock has no tracked hover yet (design §6 bullet 1).
        this._lastIdleHoverPoint = null
      }
      return
    }

    if (ev.key === 'Alt') {
      // SketchUp's Alt/Option toggle, extended with the chord close. Guard
      // key autorepeat so holding Alt doesn't spin through the cycle.
      if (ev.repeat) return
      const next = COMPLETION_CYCLE[(COMPLETION_CYCLE.indexOf(this.completion) + 1) % COMPLETION_CYCLE.length]
      this.completion = next
      // With a typed buffer live, keep it visible (the mode rides along as
      // its suffix); otherwise flash the mode name until the Viewport's
      // post-onKey pointer-move refresh replaces it with the full readout.
      if (this.typed !== '') {
        this.onMeasurementCb(this._measurementText(''))
      } else {
        this.onMeasurementCb(next === 'open' ? 'Arc' : COMPLETION_LABEL[next])
      }
      return
    }

    if (ev.key === 'Enter') {
      if (this.typed === '') return
      const meters = parseLengthToMeters(this.typed)
      if (meters !== null) this._commitTyped(meters)
      return
    }

    if (isLengthInputKey(ev.key)) {
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
      this.onMeasurementCb(this._typedReadout())
    }
  }

  /** The typed-buffer readout, suffixed for metric formats (mirrors LineTool). */
  private _typedReadout(): string {
    return typedReadout(this.typed)
  }

  /** Show the typed VCB buffer if one is being entered; otherwise the given
   *  live (pointer-derived) measurement text. Mid-gesture, a non-open
   *  completion mode suffixes its label so the mode stays visible whether
   *  the readout is typed or pointer-derived. */
  private _measurementText(live: string): string {
    const text = this.typed !== '' ? this._typedReadout() : live
    const label = COMPLETION_LABEL[this.completion]
    if (label !== '' && text !== '' && this.capturingInput()) return `${text} · ${label}`
    return text
  }

  /** Esc steps back one stage: bulge → chord (A kept), chord → idle. */
  private _stepBack(): void {
    if (this.planeStage.kind === 'bulge') {
      const { plane, target, a } = this.planeStage
      this.planeStage = { kind: 'chord', plane, target, a }
      this.typed = ''
      this._lastPlaneCursor = null
      this._lastSagittaSign = null
      this._clearPreview()
      this.onMeasurementCb('')
      return
    }
    if (this.faceStage.kind === 'bulge') {
      const { object, face, normal, planePoint, a } = this.faceStage
      this.faceStage = { kind: 'chord', object, face, normal, planePoint, a }
      this.typed = ''
      this._lastFaceCursor = null
      this._lastSagittaSign = null
      this._clearPreview()
      this.onMeasurementCb('')
      return
    }
    this.cancel()
  }

  cancel(): void {
    this.planeStage = { kind: 'idle' }
    this.faceStage = { kind: 'idle' }
    this.typed = ''
    this._lastPlaneCursor = null
    this._lastFaceCursor = null
    this._lastSagittaSign = null
    this.idlePlaneLock = null
    this._lastIdleHoverPoint = null
    this._clearPreview()
    this.onMeasurementCb('')
  }

  /**
   * A new/loaded document replaced the Scene, so every cached plane-mode
   * sketch handle is now stale (reusing one throws UnknownSketch). Drop them
   * all and reset. Called by the Viewport from `notifyLoaded`.
   */
  onDocumentReset(): void {
    this.sketchCache.clear()
    this.completion = 'open' // a fresh document starts with the default close
    this.cancel()
  }

  /**
   * Enter with a non-empty typed buffer: resolved per-stage (see module doc).
   */
  private _commitTyped(distance: number): void {
    if (this.planeStage.kind === 'chord') {
      this._commitTypedPlaneChord(distance)
    } else if (this.planeStage.kind === 'bulge') {
      this._commitTypedPlaneBulge(distance)
    } else if (this.faceStage.kind === 'chord') {
      this._commitTypedFaceChord(distance)
    } else if (this.faceStage.kind === 'bulge') {
      this._commitTypedFaceBulge(distance)
    }
  }

  /** Chord stage (plane mode, ground or non-ground): place B at `distance`
   *  from A along the live cursor direction. Refused (buffer kept) with no
   *  direction yet, or a sub-ARC_MIN_CHORD_M distance. */
  private _commitTypedPlaneChord(distance: number): void {
    if (this.planeStage.kind !== 'chord') return
    if (distance < ARC_MIN_CHORD_M) return
    const cursor = this._lastPlaneCursor
    if (cursor === null) return
    const { a } = this.planeStage
    const dir = directionBetween(a, cursor)
    if (dir === null) return
    const endpoint = pointAlong(a, dir, distance)
    this._placePlaneB(endpoint)
  }

  /** Bulge stage (plane mode): commit the arc with |sagitta| == `distance`,
   *  on the side resolved by `_resolvePlaneSagittaSign`. Refused (same hint
   *  as a flat pointer-commit) with no side resolvable, or a flat result. */
  private _commitTypedPlaneBulge(distance: number): void {
    if (this.planeStage.kind !== 'bulge') return
    const { plane, target, a, b } = this.planeStage
    const sign = this._resolvePlaneSagittaSign(plane, a, b)
    if (sign === null) {
      this.onMeasurementCb(FLAT_BULGE_HINT)
      return
    }
    const chain = this._planeChain(plane, a, b, sign * distance)
    if (chain === null) {
      this.onMeasurementCb(FLAT_BULGE_HINT)
      return
    }
    this._commitPlaneChain(target, chain)
    this.planeStage = { kind: 'idle' }
    this.typed = ''
    this._lastPlaneCursor = null
    this._lastSagittaSign = null
    this._clearPreview()
    this.onMeasurementCb('')
  }

  /** Chord stage (face): place B at `distance` from A along the live cursor
   *  direction, projected onto the locked face plane. */
  private _commitTypedFaceChord(distance: number): void {
    if (this.faceStage.kind !== 'chord') return
    if (distance < ARC_MIN_CHORD_M) return
    const cursor = this._lastFaceCursor
    if (cursor === null) return
    const { a } = this.faceStage
    const dir = directionBetween(a, cursor)
    if (dir === null) return
    const endpoint = pointAlong(a, dir, distance)
    this._placeFaceB(endpoint)
  }

  /** Bulge stage (face): commit the arc with |sagitta| == `distance` on the
   *  side resolved by `_resolveFaceSagittaSign`. */
  private _commitTypedFaceBulge(distance: number): void {
    if (this.faceStage.kind !== 'bulge') return
    const sign = this._resolveFaceSagittaSign()
    if (sign === null) {
      this.onMeasurementCb(FLAT_BULGE_HINT)
      return
    }
    const { object, face, a, b, normal } = this.faceStage
    const basis = facePlaneBasis(normal)
    const verts = basis === null ? null : arcPolylineOnPlane(a, b, sign * distance, basis.u, basis.v)
    if (verts === null) {
      this.onMeasurementCb(FLAT_BULGE_HINT)
      return
    }
    this.faceStage = { kind: 'idle' }
    this.typed = ''
    this._lastFaceCursor = null
    this._lastSagittaSign = null
    this._clearPreview()
    this.onMeasurementCb('')
    this._commitFace(object, face, normal, verts, sign * distance)
  }

  /**
   * Resolve the bulge side for a typed plane-mode commit: the sign of the
   * live cursor's sagitta if it's non-negligible, else the last nonzero side
   * seen this bulge stage, else null (refuse — see module doc). Ground uses
   * the exact legacy 2D `chordSagitta`; any other plane uses the generic
   * in-plane sagitta (`_faceChordSagitta`) — the same math face mode uses.
   */
  private _resolvePlaneSagittaSign(plane: DrawPlane, a: V3, b: V3): -1 | 1 | null {
    const cursor = this._lastPlaneCursor
    if (cursor !== null) {
      const s = plane.ground
        ? chordSagitta([a[0], a[1]], [b[0], b[1]], [cursor[0], cursor[1]])
        : this._faceChordSagitta(a, b, plane.normal, cursor)
      if (s !== null && Math.abs(s) >= ARC_MIN_SAGITTA_M) return Math.sign(s) as -1 | 1
    }
    return this._lastSagittaSign
  }

  /** Face-mode counterpart of `_resolvePlaneSagittaSign`. */
  private _resolveFaceSagittaSign(): -1 | 1 | null {
    if (this.faceStage.kind !== 'bulge') return null
    const { a, b, normal } = this.faceStage
    const cursor = this._lastFaceCursor
    if (cursor !== null) {
      const s = this._faceChordSagitta(a, b, normal, cursor)
      if (s !== null && Math.abs(s) >= ARC_MIN_SAGITTA_M) return Math.sign(s) as -1 | 1
    }
    return this._lastSagittaSign
  }

  // ------------------------------------------------------------------ plane mode

  private _onPointerMovePlane(snap: Snap | null, ray: Ray): void {
    if (this.planeStage.kind === 'idle') {
      // Idle-locked: track the hover snap for `activeDrawPlaneCue()` (design
      // §6 bullet 1).
      if (this.idlePlaneLock !== null && snap !== null) {
        this._lastIdleHoverPoint = [snap.x, snap.y, snap.z]
      }
      this._clearPreview()
      if (this.typed === '') this.onMeasurementCb('')
      return
    }
    const { plane } = this.planeStage
    const cursor = this._planeCursor(snap, ray, plane)
    if (cursor === null) {
      this._clearPreview()
      if (this.typed === '') this.onMeasurementCb('')
      return
    }
    this._lastPlaneCursor = cursor

    if (this.planeStage.kind === 'chord') {
      // Stage 2: rubber-band the chord A→cursor, report chord length.
      const { a } = this.planeStage
      this._clearPreview()
      this._drawSegments([a, cursor])
      this.onMeasurementCb(this._measurementText(formatLength(segmentLength(a, cursor))))
      return
    }

    // Stage 3: rubber-band the faceted arc (plus any completion-mode closing
    // edges) through the cursor's bulge side. Ground keeps the EXACT legacy
    // 2D path (chordSagitta/_groundChain/_reportRadius, no basis math) —
    // bit-identical to before this module existed; any other plane uses the
    // generic in-plane math (`_faceChordSagitta`/`_facePolyline`) face mode
    // already proved on arbitrary planes.
    const { a, b } = this.planeStage
    if (plane.ground) {
      const a2: Vec2 = [a[0], a[1]]
      const b2: Vec2 = [b[0], b[1]]
      const cursor2: Vec2 = [cursor[0], cursor[1]]
      const s = chordSagitta(a2, b2, cursor2)
      if (s !== null && Math.abs(s) >= ARC_MIN_SAGITTA_M) this._lastSagittaSign = Math.sign(s) as -1 | 1
      const verts = s === null ? null : (this._groundChain(a2, b2, s)?.chain ?? null)
      if (verts === null) {
        this._clearPreview()
        this._drawSegments([a, b])
        this.onMeasurementCb(this._measurementText(''))
        return
      }
      this._clearPreview()
      this._drawSegments(verts)
      this._reportRadius(a2, b2, s as number)
      return
    }

    const s = this._faceChordSagitta(a, b, plane.normal, cursor)
    if (s !== null && Math.abs(s) >= ARC_MIN_SAGITTA_M) this._lastSagittaSign = Math.sign(s) as -1 | 1
    const verts = this._facePolyline(a, b, plane.normal, cursor)
    if (verts === null || s === null) {
      this._clearPreview()
      this._drawSegments([a, b])
      this.onMeasurementCb(this._measurementText(''))
      return
    }
    this._clearPreview()
    this._drawSegments(verts.concat(this._closingVerts(verts, this._faceCenter(a, b, plane.normal, s))))
    this._reportRadiusFromChain(verts)
  }

  private _onPointerDownPlane(snap: Snap | null, ray: Ray): void {
    if (this.planeStage.kind === 'idle') {
      // First click: resolve (and freeze) the plane/target, then anchor A.
      const resolved = this._resolveClickTarget(snap, ray)
      if (resolved === null) return
      const { plane, target } = resolved
      const a = this._planeCursor(snap, ray, plane)
      if (a === null) return
      this.planeStage = { kind: 'chord', plane, target, a }
      this.typed = ''
      this._lastPlaneCursor = null
      return
    }

    const { plane } = this.planeStage
    const cursor = this._planeCursor(snap, ray, plane)
    if (cursor === null) return

    if (this.planeStage.kind === 'chord') {
      // Second click: endpoint B (the chord). Ignore a degenerate chord.
      this._placePlaneB(cursor)
      return
    }

    // Third click: commit at the cursor's sagitta. Refuse a flat bulge.
    const { target, a, b } = this.planeStage
    const s = plane.ground
      ? chordSagitta([a[0], a[1]], [b[0], b[1]], [cursor[0], cursor[1]])
      : this._faceChordSagitta(a, b, plane.normal, cursor)
    const chain = s === null ? null : this._planeChain(plane, a, b, s)
    if (chain === null) {
      this.onMeasurementCb(FLAT_BULGE_HINT)
      return
    }

    this._commitPlaneChain(target, chain)
    this.planeStage = { kind: 'idle' }
    this.typed = ''
    this._lastPlaneCursor = null
    this._lastSagittaSign = null
    this._clearPreview()
    this.onMeasurementCb('')
  }

  /** Place chord endpoint B (plane mode) — shared by the pointer-click and
   *  typed (`_commitTypedPlaneChord`) paths. Ignores a degenerate
   *  (sub-ARC_MIN_CHORD_M) chord. */
  private _placePlaneB(b: V3): void {
    if (this.planeStage.kind !== 'chord') return
    const { plane, target, a } = this.planeStage
    if (segmentLength(a, b) < ARC_MIN_CHORD_M) return
    this.planeStage = { kind: 'bulge', plane, target, a, b }
    this.typed = ''
    this._lastPlaneCursor = null
    this._lastSagittaSign = null
    this._clearPreview()
    this.onMeasurementCb('')
  }

  /** The faceted ground-plane polyline for chord a→b with sagitta s (z=0). */
  private _groundPolyline(a: Vec2, b: Vec2, s: number): V3[] | null {
    return arcPolylineOnPlane([a[0], a[1], 0], [b[0], b[1], 0], s, [1, 0, 0], [0, 1, 0])
  }

  /** Closing vertices appended after the arc polyline for the current
   *  completion mode: the chord back to A for 'segment', B→center→A for
   *  'pie', nothing for 'open' (or a 'pie' with no resolvable center). The
   *  appended A is `verts[0]` itself, so the chain closes with an exact
   *  coordinate match and the sticky rules merge it. */
  private _closingVerts(verts: V3[], center: V3 | null): V3[] {
    if (this.completion === 'segment') return [verts[0]]
    if (this.completion === 'pie' && center !== null) return [center, verts[0]]
    return []
  }

  /** 3D arc center for the ground chord a→b with sagitta s (z=0), or null
   *  on degenerate input. */
  private _groundCenter(a: Vec2, b: Vec2, s: number): V3 | null {
    const arc = arcFromChord(a, b, s)
    return arc === null ? null : [arc.center[0], arc.center[1], 0]
  }

  /** 3D arc center for the chord a→b with sagitta s on an arbitrary plane
   *  with unit `normal`, lifted through the plane's in-plane basis, or null
   *  on degenerate input. Pure — used by face mode AND non-ground plane
   *  mode (the design's "face mode already proved this on arbitrary
   *  planes" toolbox). */
  private _faceCenter(a: V3, b: V3, normal: V3, s: number): V3 | null {
    const basis = facePlaneBasis(normal)
    if (basis === null) return null
    const { u, v } = basis
    const dx = b[0] - a[0]
    const dy = b[1] - a[1]
    const dz = b[2] - a[2]
    const bu = dx * u[0] + dy * u[1] + dz * u[2]
    const bv = dx * v[0] + dy * v[1] + dz * v[2]
    const arc = arcFromChord([0, 0], [bu, bv], s)
    if (arc === null) return null
    const [cu, cv] = arc.center
    return [
      a[0] + u[0] * cu + v[0] * cv,
      a[1] + u[1] * cu + v[1] * cv,
      a[2] + u[2] * cu + v[2] * cv,
    ]
  }

  /** The committed ground chain: the arc polyline plus the closing vertices
   *  for the current completion mode. `curveSegments` counts the ARC's own
   *  segments — the part tagged as one curve chain; pie/segment closing
   *  edges stay plain lines. `geom` is the arc's analytic circle (center on
   *  the ground plane + radius), recorded on the curve chain so the kernel
   *  keeps the exact definition the facets approximate. */
  private _groundChain(a: Vec2, b: Vec2, s: number): ArcChain | null {
    const verts = this._groundPolyline(a, b, s)
    if (verts === null) return null
    const arc = arcFromChord(a, b, s)
    return {
      chain: verts.concat(this._closingVerts(verts, this._groundCenter(a, b, s))),
      curveSegments: verts.length - 1,
      geom: arc === null ? null : { center: [arc.center[0], arc.center[1], 0], radius: arc.radius },
    }
  }

  /** Non-ground plane-mode counterpart of `_groundChain`: the arc polyline
   *  built in the plane's own in-plane basis (`facePlaneBasis` +
   *  `arcPolylineOnPlane` — exactly the face-mode toolbox) plus the closing
   *  vertices for the current completion mode, with the analytic circle
   *  (center + radius) to ride the curve chain. Returns null on a
   *  degenerate basis or chord/sagitta (mirrors `_groundChain`). */
  private _nonGroundChain(a: V3, b: V3, normal: V3, s: number): ArcChain | null {
    const basis = facePlaneBasis(normal)
    if (basis === null) return null
    const verts = arcPolylineOnPlane(a, b, s, basis.u, basis.v)
    if (verts === null) return null
    const center = this._faceCenter(a, b, normal, s)
    return {
      chain: verts.concat(this._closingVerts(verts, center)),
      curveSegments: verts.length - 1,
      geom: center === null ? null : { center, radius: segmentLength(center, a) },
    }
  }

  /** The arc chain for chord a→b with sagitta s, dispatching to the EXACT
   *  legacy ground path (bit-identical arithmetic) or the generic
   *  arbitrary-plane path, per `plane.ground`. */
  private _planeChain(plane: DrawPlane, a: V3, b: V3, s: number): ArcChain | null {
    return plane.ground
      ? this._groundChain([a[0], a[1]], [b[0], b[1]], s)
      : this._nonGroundChain(a, b, plane.normal, s)
  }

  /** Commit the polyline chain as N sketch segments into `target`'s sketch;
   *  the first `curveSegments` of them are bracketed as ONE curve chain (the
   *  arc), so clicking any facet later selects the whole arc. When the arc's
   *  analytic circle is known it rides on the chain (durable center/radius —
   *  the true-curves design). Used by plane mode — ground AND any other
   *  plane (sketch mode) — via `runSketchGesture`; real face mode instead
   *  imprints via `split_face`/`split_face_inner` (see `_commitFace`). */
  private _commitPlaneChain(target: SketchTarget, { chain: verts, curveSegments, geom }: ArcChain): void {
    try {
      runSketchGesture(this.wasmScene, this.sketchCache, target, (sketch) => {
        let lastRegionsCreated: bigint[] = []
        if (geom !== null) {
          this.wasmScene.sketch_begin_curve_with(
            sketch,
            geom.center[0], geom.center[1], geom.center[2],
            geom.radius,
          )
        } else {
          this.wasmScene.sketch_begin_curve(sketch)
        }
        try {
          for (let i = 0; i < verts.length - 1; i++) {
            if (i === curveSegments) this.wasmScene.sketch_end_curve(sketch)
            const p = verts[i]
            const q = verts[i + 1]
            const report = this.wasmScene.sketch_add_segment(
              sketch,
              p[0], p[1], p[2],
              q[0], q[1], q[2],
            )
            try {
              const rc = report.regions_created()
              lastRegionsCreated = Array.from(rc)
            } finally {
              report.free()
            }
          }
        } finally {
          // The kernel also force-closes the bracket at gesture end; this
          // just keeps the tool honest on a mid-chain error.
          this.wasmScene.sketch_end_curve(sketch)
        }

        this.onCommit({ sketchHandle: sketch, regionsCreated: lastRegionsCreated })
      })
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      const message = kernelErrorMessage(code ?? 'Unknown', rawMsg)
      this.onToast(message, code ?? undefined)
    }
  }

  // ------------------------------------------------------------------ face mode

  /** The cursor's position on the locked face plane (snapped point when
   * available — it's already plane-constrained — else raw ray∩plane). */
  private _faceCursor(snap: Snap | null, ray: Ray): V3 | null {
    if (this.faceStage.kind === 'idle') return null
    if (snap !== null) return [snap.x, snap.y, snap.z]
    const { planePoint, normal } = this.faceStage
    return rayPlaneIntersect(ray.origin, ray.direction, planePoint, normal)
  }

  /** In-plane (u,v) signed sagitta of `cursor` relative to the chord a→b on
   * an arbitrary plane with unit `normal` (the shared projection step
   * `_facePolyline` and the bulge-side resolution both need). Pure — used by
   * face mode AND non-ground plane mode. Returns null when the plane has no
   * valid in-plane basis, or the chord is degenerate in-plane. */
  private _faceChordSagitta(a: V3, b: V3, normal: V3, cursor: V3): number | null {
    const basis = facePlaneBasis(normal)
    if (basis === null) return null
    const { u, v } = basis
    const project = (p: V3): Vec2 => {
      const dx = p[0] - a[0]
      const dy = p[1] - a[1]
      const dz = p[2] - a[2]
      return [dx * u[0] + dy * u[1] + dz * u[2], dx * v[0] + dy * v[1] + dz * v[2]]
    }
    return chordSagitta([0, 0], project(b), project(cursor))
  }

  /** In-plane (u,v) sagitta of `cursor` for the face chord a→b, plus the
   * face polyline it implies. Returns null when flat/degenerate. */
  private _facePolyline(a: V3, b: V3, normal: V3, cursor: V3): V3[] | null {
    const basis = facePlaneBasis(normal)
    if (basis === null) return null
    const s = this._faceChordSagitta(a, b, normal, cursor)
    if (s === null) return null
    return arcPolylineOnPlane(a, b, s, basis.u, basis.v)
  }

  private _onPointerMoveFace(snap: Snap | null, ray: Ray): void {
    if (this.faceStage.kind === 'idle') {
      this._clearPreview()
      if (this.typed === '') this.onMeasurementCb('')
      return
    }
    const cursor = this._faceCursor(snap, ray)
    if (cursor === null) {
      this._clearPreview()
      if (this.typed === '') this.onMeasurementCb('')
      return
    }

    if (this.faceStage.kind === 'chord') {
      const { a } = this.faceStage
      this._lastFaceCursor = cursor
      this._clearPreview()
      this._drawSegments([a, cursor])
      this.onMeasurementCb(this._measurementText(formatLength(segmentLength(a, cursor))))
      return
    }

    if (this.faceStage.kind !== 'bulge') return // (narrowing is lost across the _faceCursor call)
    const { a, b, normal } = this.faceStage
    this._lastFaceCursor = cursor
    const s = this._faceChordSagitta(a, b, normal, cursor)
    if (s !== null && Math.abs(s) >= ARC_MIN_SAGITTA_M) this._lastSagittaSign = Math.sign(s) as -1 | 1
    const verts = this._facePolyline(a, b, normal, cursor)
    this._clearPreview()
    if (verts === null || s === null) {
      this._drawSegments([a, b])
      this.onMeasurementCb(this._measurementText(''))
      return
    }
    // Preview the arc plus any completion-mode closing edges; the radius
    // readout still derives from the bare arc vertices.
    this._drawSegments(verts.concat(this._closingVerts(verts, this._faceCenter(a, b, normal, s))))
    this._reportRadiusFromChain(verts)
  }

  private _onPointerDownFace(snap: Snap | null, ray: Ray): void {
    if (this.faceStage.kind === 'idle') {
      // First click: anchor endpoint A on the eligible face under the cursor.
      if (snap === null) return

      const eligible = this._eligiblePickFor(ray)
      if (eligible === null) return

      const normalArr = this.wasmScene.face_normal(eligible.object, eligible.face)
      const normal: V3 = [normalArr[0], normalArr[1], normalArr[2]]
      const a: V3 = [snap.x, snap.y, snap.z]

      this.faceStage = {
        kind: 'chord',
        object: eligible.object,
        face: eligible.face,
        normal,
        planePoint: a,
        a,
      }
      this.typed = ''
      this._lastFaceCursor = null
      return
    }

    const cursor = this._faceCursor(snap, ray)
    if (cursor === null) return

    if (this.faceStage.kind === 'chord') {
      // Second click: endpoint B. Ignore a degenerate chord.
      this._placeFaceB(cursor)
      return
    }

    // Third click: commit the face cut. Refuse a flat bulge.
    if (this.faceStage.kind !== 'bulge') return // (narrowing is lost across the _faceCursor call)
    const { object, face, a, b, normal } = this.faceStage
    const s = this._faceChordSagitta(a, b, normal, cursor)
    const verts = s === null ? null : this._facePolyline(a, b, normal, cursor)
    if (verts === null || s === null) {
      this.onMeasurementCb(FLAT_BULGE_HINT)
      return
    }

    this.faceStage = { kind: 'idle' }
    this.typed = ''
    this._lastFaceCursor = null
    this._lastSagittaSign = null
    this._clearPreview()
    this.onMeasurementCb('')
    this._commitFace(object, face, normal, verts, s)
  }

  /** Place chord endpoint B (face) — shared by the pointer-click and typed
   *  (`_commitTypedFaceChord`) paths. Ignores a degenerate (sub-ARC_MIN_CHORD_M)
   *  chord. */
  private _placeFaceB(cursor: V3): void {
    if (this.faceStage.kind !== 'chord') return
    const { object, face, normal, planePoint, a } = this.faceStage
    if (segmentLength(a, cursor) < ARC_MIN_CHORD_M) return
    this.faceStage = { kind: 'bulge', object, face, normal, planePoint, a, b: cursor }
    this.typed = ''
    this._lastFaceCursor = null
    this._lastSagittaSign = null
    this._clearPreview()
    this.onMeasurementCb('')
  }

  /** Commit the face cut for the current completion mode: an open arc cuts
   * boundary-to-boundary (`split_face`); pie/segment close into a loop and
   * imprint like CircleTool (`split_face_inner`). A pie whose center is
   * unresolvable (degenerate basis — cannot happen when `verts` built) falls
   * back to the open cut. */
  private _commitFace(object: bigint, face: bigint, normal: V3, verts: V3[], s: number): void {
    if (this.completion === 'open') {
      this._commitFaceChain(object, face, verts)
      return
    }
    let loop = verts
    if (this.completion === 'pie') {
      const center = this._faceCenter(verts[0], verts[verts.length - 1], normal, s)
      if (center === null) {
        this._commitFaceChain(object, face, verts)
        return
      }
      loop = verts.concat([center])
    }
    this._commitFaceLoop(object, face, loop)
  }

  /** Imprint `verts` on `face` as a closed loop (the loop closes implicitly
   * from the last vertex back to the first — same convention as CircleTool's
   * `split_face_inner` commit). */
  private _commitFaceLoop(object: bigint, face: bigint, verts: V3[]): void {
    const loopPts = new Float64Array(verts.length * 3)
    for (let i = 0; i < verts.length; i++) {
      loopPts[i * 3 + 0] = verts[i][0]
      loopPts[i * 3 + 1] = verts[i][1]
      loopPts[i * 3 + 2] = verts[i][2]
    }

    try {
      this.wasmScene.split_face_inner(object, face, loopPts)
      this.onFaceImprint(object)
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      const message = kernelErrorMessage(code ?? 'Unknown', rawMsg)
      this.onToast(message, code ?? undefined)
    }
  }

  /** Cut `face` along the arc polyline (open, boundary-to-boundary — the
   * same `split_face` call LineTool's face chain commits with). */
  private _commitFaceChain(object: bigint, face: bigint, verts: V3[]): void {
    const path = new Float64Array(verts.length * 3)
    for (let i = 0; i < verts.length; i++) {
      path[i * 3 + 0] = verts[i][0]
      path[i * 3 + 1] = verts[i][1]
      path[i * 3 + 2] = verts[i][2]
    }

    try {
      const report = this.wasmScene.split_face(object, face, path)
      report.free()
      this.onFaceImprint(object)
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      const message = kernelErrorMessage(code ?? 'Unknown', rawMsg)
      this.onToast(message, code ?? undefined)
    }
  }

  // ------------------------------------------------------------------ measurement

  /** Report the live arc radius for a valid ground bulge (`_measurementText`
   *  suffixes the completion mode) — the EXACT legacy formula (straight from
   *  `arcFromChord`, no chain round-trip), kept bit-identical to before this
   *  module existed. */
  private _reportRadius(a: Vec2, b: Vec2, s: number): void {
    const arc = arcFromChord(a, b, s)
    if (arc === null) {
      this.onMeasurementCb(this._measurementText(''))
      return
    }
    this.onMeasurementCb(this._measurementText(`R ${formatLength(arc.radius)}`))
  }

  /** Report the radius from an already-built polyline (the bare arc, no
   *  completion-mode closing vertices): the distance from any interior
   *  vertex to the chord endpoints determines the circle, but it's simplest
   *  to recompute from the sagitta implied by the mid vertex. Used by face
   *  mode and non-ground plane mode. */
  private _reportRadiusFromChain(verts: V3[]): void {
    const a = verts[0]
    const b = verts[verts.length - 1]
    const mid = verts[Math.floor(verts.length / 2)]
    // Radius from three points: circumradius = (|AB|·|AM|·|BM|) / (4·area).
    const ab = segmentLength(a, b)
    const am = segmentLength(a, mid)
    const bm = segmentLength(b, mid)
    // Heron's formula for the triangle area.
    const p = (ab + am + bm) / 2
    const areaSq = p * (p - ab) * (p - am) * (p - bm)
    if (areaSq <= 0) {
      this.onMeasurementCb(this._measurementText(''))
      return
    }
    const radius = (ab * am * bm) / (4 * Math.sqrt(areaSq))
    this.onMeasurementCb(this._measurementText(`R ${formatLength(radius)}`))
  }

  // ------------------------------------------------------------------ preview

  /**
   * Emit a fat-line preview for an open polyline. Vertices are used exactly
   * as given — the preview's depth bias (PREVIEW_LINE_STYLE, depthPolicy.ts)
   * settles coincidence with the ground/committed lines, so no z-lift.
   *
   * @param verts  Ordered world-space vertices (>= 2).
   */
  private _drawSegments(verts: V3[]): void {
    const nSegs = verts.length - 1
    if (nSegs < 1) return
    const pts = new Float32Array(nSegs * 2 * 3)
    for (let i = 0; i < nSegs; i++) {
      const a = verts[i]
      const b = verts[i + 1]
      const base = i * 6
      pts[base + 0] = a[0]; pts[base + 1] = a[1]; pts[base + 2] = a[2]
      pts[base + 3] = b[0]; pts[base + 4] = b[1]; pts[base + 5] = b[2]
    }
    this.preview.add(makeFatSegments(pts, PREVIEW_LINE_STYLE))
  }

  private _clearPreview(): void {
    this.preview.traverse((child) => {
      disposeFatSegments(child)
      if (child instanceof THREE.LineSegments) {
        child.geometry.dispose()
        if (child.material instanceof THREE.Material) {
          child.material.dispose()
        }
      }
    })
    this.preview.clear()
  }
}
