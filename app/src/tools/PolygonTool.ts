/**
 * PolygonTool — two-click regular N-gon sketching, center then circumradius.
 *
 * A regular polygon is just N plain straight edges — unlike CircleTool's
 * faceted-circle approximation, it carries NO analytic curve metadata (there
 * is no "true polygon" to approximate; the facets ARE the geometry). It
 * decomposes into N chained `sketch_add_segment` calls (plane mode) or one
 * `split_face_inner` call with N loop points (face mode) — identical to how
 * RectangleTool commits its four corners, and structurally identical to
 * CircleTool's own plane-mode commit path minus the analytic curve chain. No
 * kernel change is needed; push/pull then works for free once a closed loop
 * forms a region.
 *
 * Two modes:
 *
 * Mode is decided by what's under the cursor per pointer event (mirrors
 * LineTool/CircleTool; see `faceDraw.ts` for the shared plain-object
 * eligibility policy), never by `_activeContext` alone.
 *
 * Plane mode (no eligible face under the cursor — sketches on any plane,
 * design doc §1/§4, same as every other draw tool): the drawing plane is
 * resolved once, at the FIRST click, and frozen for the rest of the gesture:
 *   - A top-level hover over a committed sketch whose plane is non-ground
 *     (`pick_sketch` + `planeFromSketch`) adopts THAT sketch's plane —
 *     SKETCH MODE — and every edge lands in that one sketch
 *     (`SketchTarget.existing`).
 *   - Otherwise the plane is the ground plane — PLANE MODE, today's
 *     behavior — segments land in the shared per-plane cached sketch
 *     (`SketchTarget.plane`; `begin_ground_sketch()` on a cache miss).
 *   On the ground plane the center/rim/facets are computed by the EXACT
 *   legacy `circlePolygonGround` (z = 0, no basis math) — bit-identical
 *   committed coordinates. On any other plane, facets come from
 *   `circlePolygonFace` (the same helper face mode uses), and the cursor is
 *   the snap (already plane-constrained via `snapConstraint`) or, absent
 *   one, ray∩plane.
 *
 *   1. First click: anchor center (snapped on the resolved plane)
 *   2. Move: rubber-band N-gon preview whose first vertex passes through the
 *      cursor (circumradius = distance from center to cursor; start angle =
 *      angle from center to cursor)
 *   3. Second click: commit — N sketch_add_segment calls chaining
 *      vertex[i] -> vertex[i+1], last one vertex[N-1] -> vertex[0] (using
 *      the SAME stored vertex[0] coords for exact closure), via
 *      `runSketchGesture`
 *   4. Esc between clicks: cancel stage 1
 *   Calls onCommit() after each successful commit so the viewport can
 *   refresh scene geometry and trigger re-render.
 *
 * With a draw tool idle, an arrow key locks the next gesture's drawing plane
 * by its normal (design §5.2, same mechanism as every other draw tool) — see
 * `_resolveClickTarget`/`activeDrawPlaneCue`.
 *
 * Face mode (an eligible Object face is under the cursor):
 *   1. First click on an eligible face: anchor center (on face plane)
 *   2. Move: rubber-band N-gon preview projected onto that face plane
 *   3. Second click: commit — split_face_inner() on that face with N loop
 *      points
 *   4. Esc: cancel
 *   Calls onFaceImprint(objectId) after each successful imprint so the
 *   viewport can refresh the scene.
 *
 * Sides: default 6, changed at any time during the gesture by typing `<n>s`
 * (SketchUp's Polygon convention, e.g. `8s`) and Enter — clamped to
 * [MIN_POLYGON_SIDES, MAX_POLYGON_SIDES], stays anchored (does not commit),
 * and refreshes the live preview immediately from the last known cursor. The
 * side count persists across gestures (an instance field), and across tool
 * re-selection via `setSideCount`/`OnSideCountChange` — the Viewport wires
 * these to a session-lived value the same way PaintTool's current material
 * persists.
 *
 * VCB: the circumradius is a SINGLE length (unlike Rectangle's W x D), so
 * typed entry mirrors CircleTool's single-length VCB style
 * (editPolygonBuffer/parseLengthToMeters) — extended with the `<n>s`
 * side-count grammar (editPolygonBuffer/parsePolygonSideCount).
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { V3 } from '../viewport/geoHelpers'
import { circlePolygonGround, circlePolygonFace, facePlaneBasis, rayPlaneIntersect } from '../viewport/geoHelpers'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { makeFatSegments, disposeFatSegments, PREVIEW_LINE_STYLE } from '../viewport/fatLine'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { editPolygonBuffer, isPolygonInputKey, parsePolygonSideCount, nextIdlePlaneLock, AXIS_LOCK_COLOR_NAMES } from './moveInput'
import { segmentLength } from './lineInput'
import { runSketchGesture, makeSketchPlaneCache, type SketchPlaneCache, type SketchTarget } from './sketchGesture'
import { groundDrawPlane, planeFromSketch, pointOnPlane, axisDrawPlane, drawPlaneCue, isGroundPlane, SketchPickCache, type DrawPlane } from './drawPlane'
import { FacePickCache, defaultFaceEligible, type FaceEligible } from './faceDraw'

/** SketchUp parity default (design §1). */
export const DEFAULT_POLYGON_SIDES = 6
/** Below this, a polygon degenerates toward a line/point — refused by clamp, not the kernel. */
export const MIN_POLYGON_SIDES = 3
/** Above this, a circle is the right tool (design §1) — clamp rather than let facet count run away. */
export const MAX_POLYGON_SIDES = 120

/** A circumradius below this (meters) is a degenerate polygon: every commit
 * path treats it as a no-op that STAYS in the gesture (design §4, "radius
 * below the snap tolerance → no-op"), never a silent teardown that drops the
 * placed center. Matches the tolerance the click/typed guards and
 * `circlePolygonGround`/`circlePolygonFace`'s own last-line checks use. */
const DEGENERATE_RADIUS_M = 1e-7

function clampSides(n: number): number {
  return Math.max(MIN_POLYGON_SIDES, Math.min(MAX_POLYGON_SIDES, Math.round(n)))
}

export type PolygonCommitResult = {
  sketchHandle: bigint
  /** Handles of regions created by the last segment (may be empty if not yet closed) */
  regionsCreated: bigint[]
}

export type OnPolygonCommit = (result: PolygonCommitResult) => void
export type OnFaceImprint = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void
/** Fired when the typed `<n>s` grammar changes the side count, so the
 *  Viewport can persist it (session-lived, like PaintTool's material). */
export type OnSideCountChange = (sides: number) => void

/** Plane stage: waiting for first click, or waiting for second click, on a
 *  frozen `DrawPlane`/`SketchTarget`. */
type PlaneStage =
  | { kind: 'idle' }
  | { kind: 'anchored'; plane: DrawPlane; target: SketchTarget; center: V3 }

/** Face stage: idle, or anchored on a specific face plane */
type FaceStage =
  | { kind: 'idle' }
  | {
      kind: 'anchored'
      object: bigint
      face: bigint
      normal: V3
      /** A world-space point that lies on the face plane (the first click position) */
      planePoint: V3
      center: V3
    }

export class PolygonTool implements Tool {
  readonly name = 'Polygon'

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    if (this.planeStage.kind !== 'idle' || this.faceStage.kind !== 'idle') {
      return 'Click to set the radius — or type an exact radius, or Ns for N sides.'
    }
    if (this.idlePlaneLock !== null) {
      return `Locked to the ${AXIS_LOCK_COLOR_NAMES[this.idlePlaneLock]} plane — click to start; same arrow or Esc unlocks.`
    }
    return "Click the polygon's center — on the ground plane or any face or sketch."
  }

  private planeStage: PlaneStage = { kind: 'idle' }
  private faceStage: FaceStage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnPolygonCommit
  private onFaceImprint: OnFaceImprint
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement
  private onSideCountChangeCb: OnSideCountChange

  /** The current side count. Persists across gestures within this tool
   *  instance's lifetime; the Viewport carries it across tool re-selection
   *  via `setSideCount`/`OnSideCountChange`. */
  private sides: number = DEFAULT_POLYGON_SIDES

  /** Cached plane-mode sketch handles — the Viewport passes one cache
   *  shared by every draw tool, so mixed-tool profiles land in a single
   *  sketch per plane. */
  private readonly sketchCache: SketchPlaneCache

  /** The currently active editing context (entered object), or null at top level. */
  private _activeContext: bigint | null = null

  /** VCB buffer — raw string being typed by the user (radius or `<n>s`, in display units) */
  private typed: string = ''

  /** Last rubber-band cursor positions, tracked for typed-entry direction
   *  and for refreshing the preview immediately after a side-count change. */
  private _lastPlaneCursor: V3 | null = null
  private _lastFaceCursor: V3 | null = null

  /** Idle plane lock (design §5.2): while FULLY idle (no anchored stage),
   *  an arrow key locks the future plane's NORMAL to a world axis (0=X/red,
   *  1=Y/green, 2=Z/blue — `arrowToAxis`); the same arrow again, or
   *  Escape/ArrowDown, clears it. An ACTIVE lock overrides face pick and
   *  sketch-hover adoption on the next click (SketchUp: an explicit lock
   *  beats inference) — see `_currentMode`/`_resolveClickTarget`. Survives a
   *  completed gesture (cleared only by `cancel()`, which
   *  `onDocumentReset()`/`setActiveContext()` already route through). */
  private idlePlaneLock: 0 | 1 | 2 | null = null

  /** The last hover point seen while idle-locked (design §6 bullet 1) — feeds
   *  `activeDrawPlaneCue()`'s idle-locked case. Reset to null whenever the
   *  lock itself changes (a fresh lock has no hover yet) and by `cancel()`. */
  private _lastIdleHoverPoint: V3 | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnPolygonCommit,
    onToast: OnToast,
    onFaceImprint: OnFaceImprint,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
    sketchCache: SketchPlaneCache = makeSketchPlaneCache(),
    onSideCountChange: OnSideCountChange = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onCommit = onCommit
    this.onFaceImprint = onFaceImprint
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
    this.sketchCache = sketchCache
    this.onSideCountChangeCb = onSideCountChange
  }

  /** The current side count. */
  get sideCount(): number {
    return this.sides
  }

  /** Set the side count (clamped), without firing `OnSideCountChange` — for
   *  the Viewport to inject a session-persisted value onto a freshly
   *  constructed tool instance (mirrors PaintTool's `setCurrentMaterial`). */
  setSideCount(n: number): void {
    this.sides = clampSides(n)
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
    const sketchHandle = this._sketchPickCache.pickFor(this.wasmScene, ray)
    if (sketchHandle !== null) {
      const plane = planeFromSketch(this.wasmScene, sketchHandle)
      if (plane !== null && !plane.ground) {
        return { plane, target: { kind: 'existing', handle: sketchHandle } }
      }
    }
    const plane = groundDrawPlane()
    return { plane, target: { kind: 'plane', plane } }
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
    if (this.idlePlaneLock !== null) {
      if (snap === null) return null
      const clickedPoint: V3 = [snap.x, snap.y, snap.z]
      const plane = axisDrawPlane(this.idlePlaneLock, clickedPoint)
      return { plane, target: { kind: 'plane', plane } }
    }
    return this._resolveIdleTarget(ray)
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
    if (this.faceStage.kind === 'anchored') return 'face'
    if (this.planeStage.kind === 'anchored') return 'plane'
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
   * - Face mode, anchored: return the already-known face plane so subsequent
   *   snaps stay on that plane.
   * - Plane mode, anchored on a NON-ground plane (sketch mode): same —
   *   return the frozen plane. Ground-anchored: no constraint (today's
   *   behavior, unchanged).
   * - Idle: pick the hovered face (if an eligible one is under the cursor)
   *   and return its plane so the FIRST-click center lands precisely on the
   *   face, preventing the kernel from rejecting a non-planar loop; absent
   *   that, a top-level hover over a non-ground sketch returns ITS plane.
   * - Otherwise (ground mode): return null (unconstrained).
   */
  snapConstraint(ray: Ray): { constraintPlane?: { point: [number, number, number]; normal: [number, number, number] } } | null {
    if (this.faceStage.kind === 'anchored') {
      // Already anchored: lock to the established face plane
      return {
        constraintPlane: {
          point: this.faceStage.planePoint,
          normal: this.faceStage.normal,
        },
      }
    }

    if (this.planeStage.kind === 'anchored') {
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
   * a tracked hover).
   */
  activeDrawPlaneCue(): { plane: DrawPlane; through: V3 } | null {
    if (this.faceStage.kind === 'anchored') {
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
    if (this.planeStage.kind === 'anchored') {
      return drawPlaneCue({
        anchoredPlane: this.planeStage.plane,
        anchoredThrough: this.planeStage.center,
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
      // Face mode
      if (this.faceStage.kind !== 'anchored') {
        this._clearPreview()
        this.onMeasurementCb('')
        return
      }
      const { planePoint, normal } = this.faceStage
      // Project cursor ray onto face plane
      const cursorOnPlane = rayPlaneIntersect(ray.origin, ray.direction, planePoint, normal)
      if (cursorOnPlane === null) {
        this._clearPreview()
        this.onMeasurementCb('')
        return
      }
      this._lastFaceCursor = cursorOnPlane
      this._updateFacePreview()
    } else {
      // Plane mode
      if (this.planeStage.kind !== 'anchored') {
        // Idle-locked: track the hover snap for `activeDrawPlaneCue()`
        // (design §6 bullet 1).
        if (this.idlePlaneLock !== null && snap !== null) {
          this._lastIdleHoverPoint = [snap.x, snap.y, snap.z]
        }
        this._clearPreview()
        this.onMeasurementCb('')
        return
      }
      const { plane } = this.planeStage
      const cursor = this._planeCursor(snap, ray, plane)
      if (cursor === null) {
        this._clearPreview()
        this.onMeasurementCb('')
        return
      }
      this._lastPlaneCursor = cursor
      this._updateGroundPreview()
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
   * Typed VCB entry is available once the center has been placed (either
   * plane or face mode) — see the Viewport key router, which routes
   * digit/letter/arrow keys here instead of tool-switch shortcuts while this
   * returns true.
   */
  capturingInput(): boolean {
    return this.planeStage.kind === 'anchored' || this.faceStage.kind === 'anchored'
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
      // (parity across all draw tools — LineTool's _endChain path).
      const lock = this.idlePlaneLock
      this.cancel()
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

    if (ev.key === 'Enter') {
      if (this.typed === '') return

      // `<n>s` — side count. Tried first: it's the only grammar that
      // accepts a trailing `s`, so a completed match is unambiguous.
      const n = parsePolygonSideCount(this.typed)
      if (n !== null) {
        this.sides = clampSides(n)
        this.onSideCountChangeCb(this.sides)
        this.typed = ''
        // Hot preview: reflect the new side count immediately rather than
        // waiting for the next pointer move (design §3's "re-typing updates
        // a hot preview"). A no-op if the cursor hasn't moved yet.
        if (this.planeStage.kind === 'anchored') this._updateGroundPreview()
        else if (this.faceStage.kind === 'anchored') this._updateFacePreview()
        return
      }

      // Otherwise, a length — the circumradius.
      const meters = parseLengthToMeters(this.typed)
      if (meters !== null) {
        this._commitTyped(meters)
      }
      return
    }

    // Feed digits, dot, separators, explicit-unit tokens, `s`, Backspace
    // into the buffer.
    if (isPolygonInputKey(ev.key)) {
      this.typed = editPolygonBuffer(this.typed, ev.key, getLengthUnit())
      this.onMeasurementCb(this._typedReadout())
    }
  }

  cancel(): void {
    this.planeStage = { kind: 'idle' }
    this.faceStage = { kind: 'idle' }
    this.typed = ''
    this._lastPlaneCursor = null
    this._lastFaceCursor = null
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
    this.cancel()
  }

  /** The typed-buffer readout. A buffer ending in a completed `<n>s` token
   *  never gets the length-unit suffix `typedReadout` would otherwise add to
   *  a bare trailing number — it isn't one. */
  private _typedReadout(): string {
    if (parsePolygonSideCount(this.typed) !== null) return this.typed
    return typedReadout(this.typed)
  }

  /** Report the live circumradius measurement from center to the cursor. */
  private _reportMeasurement(center: V3, cursor: V3): void {
    if (this.typed !== '') {
      this.onMeasurementCb(this._typedReadout())
      return
    }
    const radius = Math.hypot(cursor[0] - center[0], cursor[1] - center[1], cursor[2] - center[2])
    this.onMeasurementCb(`R ${formatLength(radius)}`)
  }

  /**
   * Commit an exact-circumradius polygon from the typed VCB buffer, using
   * the last rubber-band cursor to pick the start-angle/direction (default
   * +X if the cursor hasn't moved yet). Dispatches to plane or face mode
   * depending on which stage is anchored.
   */
  private _commitTyped(radius: number): void {
    // A typed radius is a magnitude — a fat-fingered `-5` means a radius of
    // 5, not a polygon flipped 180° about the center. A sub-tolerance radius
    // (e.g. `0`) is degenerate: no-op and STAY in the gesture rather than
    // resetting the stage below and losing the placed center with no
    // feedback (the commit helpers would silently no-op on it).
    const r = Math.abs(radius)
    if (r < DEGENERATE_RADIUS_M) return

    if (this.planeStage.kind === 'anchored') {
      const { plane, target, center } = this.planeStage
      let rim: V3

      if (plane.ground) {
        // EXACT legacy ground fast path — no basis math.
        const cursor = this._lastPlaneCursor ?? [center[0] + 1, center[1], 0]
        const dx = cursor[0] - center[0]
        const dy = cursor[1] - center[1]
        const len = Math.hypot(dx, dy)
        const dir: [number, number] = len < 1e-9 ? [1, 0] : [dx / len, dy / len]
        rim = [center[0] + dir[0] * r, center[1] + dir[1] * r, 0]
      } else {
        const basis = facePlaneBasis(plane.normal)
        if (basis === null) {
          this.cancel()
          return
        }
        const { u, v } = basis
        const cursor = this._lastPlaneCursor ?? center
        const dx = cursor[0] - center[0]
        const dy = cursor[1] - center[1]
        const dz = cursor[2] - center[2]
        const du = dx * u[0] + dy * u[1] + dz * u[2]
        const dv = dx * v[0] + dy * v[1] + dz * v[2]
        const len = Math.hypot(du, dv)
        const dirU = len < 1e-9 ? 1 : du / len
        const dirV = len < 1e-9 ? 0 : dv / len
        rim = [
          center[0] + u[0] * dirU * r + v[0] * dirV * r,
          center[1] + u[1] * dirU * r + v[1] * dirV * r,
          center[2] + u[2] * dirU * r + v[2] * dirV * r,
        ]
      }

      this.planeStage = { kind: 'idle' }
      this.typed = ''
      this._lastPlaneCursor = null
      this._clearPreview()
      this.onMeasurementCb('')
      this._commitPlanePolygon(plane, target, center, rim)
    } else if (this.faceStage.kind === 'anchored') {
      const { object, face, normal, center } = this.faceStage
      const basis = facePlaneBasis(normal)
      if (basis === null) {
        this.cancel()
        return
      }
      const { u, v } = basis
      const cursor = this._lastFaceCursor ?? center
      const dx = cursor[0] - center[0]
      const dy = cursor[1] - center[1]
      const dz = cursor[2] - center[2]
      const du = dx * u[0] + dy * u[1] + dz * u[2]
      const dv = dx * v[0] + dy * v[1] + dz * v[2]
      const len = Math.hypot(du, dv)
      const dirU = len < 1e-9 ? 1 : du / len
      const dirV = len < 1e-9 ? 0 : dv / len
      const rim: V3 = [
        center[0] + u[0] * dirU * r + v[0] * dirV * r,
        center[1] + u[1] * dirU * r + v[1] * dirV * r,
        center[2] + u[2] * dirU * r + v[2] * dirV * r,
      ]

      this.faceStage = { kind: 'idle' }
      this.typed = ''
      this._lastFaceCursor = null
      this._clearPreview()
      this.onMeasurementCb('')
      this._commitFacePolygon(object, face, rim, center, normal)
    }
  }

  // ------------------------------------------------------------------ plane mode

  private _onPointerDownPlane(snap: Snap | null, ray: Ray): void {
    if (this.planeStage.kind === 'idle') {
      // First click: resolve (and freeze) the plane/target, then set center.
      const resolved = this._resolveClickTarget(snap, ray)
      if (resolved === null) return
      const { plane, target } = resolved
      const center = this._planeCursor(snap, ray, plane)
      if (center === null) return

      this.planeStage = { kind: 'anchored', plane, target, center }
      this._lastPlaneCursor = null
    } else {
      // Second click: commit the polygon.
      const { plane, target, center } = this.planeStage
      const cursor = this._planeCursor(snap, ray, plane)
      if (cursor === null) return

      // Skip degenerate polygons (zero radius) — stay anchored. The ground
      // branch keeps the EXACT legacy per-axis check — bit-identical gating
      // to before this module existed; non-ground uses the Euclidean check
      // face mode already uses.
      if (plane.ground) {
        if (Math.hypot(cursor[0] - center[0], cursor[1] - center[1]) < DEGENERATE_RADIUS_M) return
      } else if (segmentLength(center, cursor) < DEGENERATE_RADIUS_M) {
        return
      }

      this._commitPlanePolygon(plane, target, center, cursor)
      this.planeStage = { kind: 'idle' }
      this.typed = ''
      this._lastPlaneCursor = null
      this._clearPreview()
      this.onMeasurementCb('')
    }
  }

  /** Commit a polygon (N plain `sketch_add_segment` calls — no curve chain,
   *  design §4/§8: a polygon's facets are its real geometry, not an
   *  approximation to suppress) into `target`'s sketch — used by both ground
   *  and non-ground plane/sketch mode (real face mode instead imprints via
   *  `split_face_inner`, see `_commitFacePolygon`). */
  private _commitPlanePolygon(plane: DrawPlane, target: SketchTarget, center: V3, rim: V3): void {
    const verts = plane.ground
      ? circlePolygonGround([center[0], center[1]], [rim[0], rim[1]], this.sides)
      : circlePolygonFace(center, rim, plane.normal, this.sides)
    if (verts === null || verts.length === 0) return // degenerate — ignore

    try {
      runSketchGesture(this.wasmScene, this.sketchCache, target, (sketch) => {
        let lastRegionsCreated: bigint[] = []
        for (let i = 0; i < verts.length; i++) {
          const p = verts[i]
          const q = verts[(i + 1) % verts.length]
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

  private _onPointerDownFace(snap: Snap | null, ray: Ray): void {
    if (this.faceStage.kind === 'idle') {
      // First click: anchor on the eligible face under the cursor
      if (snap === null) return

      const eligible = this._eligiblePickFor(ray)
      if (eligible === null) return

      const normalArr = this.wasmScene.face_normal(eligible.object, eligible.face)
      const normal: V3 = [normalArr[0], normalArr[1], normalArr[2]]
      const center: V3 = [snap.x, snap.y, snap.z]

      this.faceStage = {
        kind: 'anchored',
        object: eligible.object,
        face: eligible.face,
        normal,
        planePoint: center,
        center,
      }
      this._lastFaceCursor = null
    } else {
      // Second click: commit the face imprint
      const { object, face, normal, planePoint, center } = this.faceStage

      // Project the click ray onto the face plane for the cursor position
      const cursorOnPlane = rayPlaneIntersect(ray.origin, ray.direction, planePoint, normal)
      if (cursorOnPlane === null) return

      // Skip degenerate polygons (zero radius) — stay anchored. `center` and
      // `cursorOnPlane` both lie on the face plane, so the 3-D distance is
      // the in-plane circumradius. Guard BEFORE mutating faceStage,
      // mirroring the plane branch: without it a same-point second click
      // would reset to idle and _commitFacePolygon would silently no-op,
      // dropping the center.
      if (
        Math.hypot(
          cursorOnPlane[0] - center[0],
          cursorOnPlane[1] - center[1],
          cursorOnPlane[2] - center[2],
        ) < DEGENERATE_RADIUS_M
      ) {
        return
      }

      this.faceStage = { kind: 'idle' }
      this.typed = ''
      this._lastFaceCursor = null
      this._clearPreview()
      this.onMeasurementCb('')

      this._commitFacePolygon(object, face, cursorOnPlane, center, normal)
    }
  }

  /** Split the given face with an N-gon loop defined by center/rim/normal. */
  private _commitFacePolygon(object: bigint, face: bigint, rim: V3, center: V3, normal: V3): void {
    const verts = circlePolygonFace(center, rim, normal, this.sides)
    if (verts === null) return // degenerate — ignore

    // Flatten the N vertices into a Float64Array of xyz triples
    const loopPts = new Float64Array(verts.length * 3)
    for (let i = 0; i < verts.length; i++) {
      loopPts[i * 3 + 0] = verts[i][0]
      loopPts[i * 3 + 1] = verts[i][1]
      loopPts[i * 3 + 2] = verts[i][2]
    }

    try {
      // Plain imprint — no curve identity (design §4/§8), unlike Circle's
      // split_face_inner_with_curve.
      this.wasmScene.split_face_inner(object, face, loopPts)
      this.onFaceImprint(object)
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      const message = kernelErrorMessage(code ?? 'Unknown', rawMsg)
      this.onToast(message, code ?? undefined)
    }
  }

  // ------------------------------------------------------------------ preview

  /** Recompute and draw the plane-mode rubber-band N-gon from the anchored
   *  center and the last known cursor position — called from both
   *  `onPointerMove` and a live side-count change. A no-op while idle or
   *  before the cursor has moved. */
  private _updateGroundPreview(): void {
    if (this.planeStage.kind !== 'anchored' || this._lastPlaneCursor === null) return
    const { plane, center } = this.planeStage
    const cursor = this._lastPlaneCursor
    const verts = plane.ground
      ? circlePolygonGround([center[0], center[1]], [cursor[0], cursor[1]], this.sides)
      : circlePolygonFace(center, cursor, plane.normal, this.sides)
    if (verts !== null && verts.length > 0) {
      this._drawRubberBandVerts(verts)
      this._reportMeasurement(center, cursor)
    } else {
      this._clearPreview()
      if (this.typed === '') this.onMeasurementCb('')
    }
  }

  /** Face-mode counterpart of `_updateGroundPreview`. */
  private _updateFacePreview(): void {
    if (this.faceStage.kind !== 'anchored' || this._lastFaceCursor === null) return
    const { center, normal } = this.faceStage
    const cursor = this._lastFaceCursor
    const verts = circlePolygonFace(center, cursor, normal, this.sides)
    if (verts !== null) {
      this._drawRubberBandVerts(verts)
      this._reportMeasurement(center, cursor)
    } else {
      this._clearPreview()
      if (this.typed === '') this.onMeasurementCb('')
    }
  }

  /**
   * Emit a LineSegments preview for a closed N-vertex loop. Vertices are used
   * exactly as given — the preview's depth bias (PREVIEW_LINE_STYLE,
   * depthPolicy.ts) settles coincidence with the ground/committed lines, so
   * no z-lift.
   *
   * @param verts  N world-space xyz vertices in order.
   */
  private _drawRubberBandVerts(verts: V3[]): void {
    this._clearPreview()
    const n = verts.length
    const pts = new Float32Array(n * 2 * 3)
    for (let i = 0; i < n; i++) {
      const a = verts[i]
      const b = verts[(i + 1) % n]
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
