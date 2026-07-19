/**
 * RectangleTool — two-click rectangle sketching.
 *
 * Two modes:
 *
 * Plane mode (activeContext === null, no eligible face): the drawing plane
 * is resolved once, at the first click, and frozen for the rest of the
 * gesture (sketches on any plane — design doc §1/§4):
 *   - A top-level hover over a committed sketch whose plane is non-ground
 *     (`pick_sketch` + `planeFromSketch`) adopts THAT sketch's plane —
 *     SKETCH MODE — and the rectangle's four segments land in that one
 *     sketch (`SketchTarget.existing`).
 *   - Otherwise the plane is the ground plane — PLANE MODE, today's
 *     behavior — segments land in the shared per-plane cached sketch
 *     (`SketchTarget.plane`; `begin_ground_sketch()` on a cache miss).
 *   On the ground plane every corner is computed by the EXACT legacy
 *   `rectangleCorners` (z = 0, no basis math) — bit-identical committed
 *   coordinates. On any other plane, corners come from `faceRectangleCorners`
 *   (the same helper face mode uses), and the cursor is the snap (already
 *   plane-constrained via `snapConstraint`) or, absent one, ray∩plane.
 *
 *   1. First click: anchor corner
 *   2. Move: rubber-band rectangle preview on the plane
 *   3. Second click: commit — four `sketch_add_segment` calls forming the
 *      rectangle, via `runSketchGesture`
 *   4. Esc between clicks: cancel stage 1
 *   Calls onCommit() after each successful commit so the viewport can
 *   refresh scene geometry and trigger re-render.
 *
 * Face mode (an eligible Object face is under the cursor — decided per
 * pointer event like LineTool, not by whether an editing context is active;
 * see `faceDraw.ts` for the shared plain-object eligibility policy):
 *   1. First click on an eligible face: anchor corner (on face plane)
 *   2. Move: rubber-band rectangle preview projected onto that face plane
 *   3. Second click: commit — split_face_inner() on that face
 *   4. Esc: cancel
 *   Calls onFaceImprint(objectId) after each successful imprint so the viewport
 *   can refresh the scene.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { V3 } from '../viewport/geoHelpers'
import { rectangleCorners, faceRectangleCorners, facePlaneBasis, rayPlaneIntersect } from '../viewport/geoHelpers'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { makeFatSegments, disposeFatSegments, PREVIEW_LINE_STYLE } from '../viewport/fatLine'
import { formatLength, parseDimensionsToMeters, typedReadout } from '../settings/units'
import { editDimsBuffer, nextIdlePlaneLock, AXIS_LOCK_COLOR_NAMES } from './moveInput'
import { runSketchGesture, makeSketchPlaneCache, type SketchPlaneCache, type SketchTarget } from './sketchGesture'
import { groundDrawPlane, planeFromSketch, pointOnPlane, axisDrawPlane, drawPlaneCue, isGroundPlane, SketchPickCache, type DrawPlane } from './drawPlane'
import { FacePickCache, defaultFaceEligible, type FaceEligible } from './faceDraw'

export type RectangleCommitResult = {
  sketchHandle: bigint
  /** Handles of regions created by the last segment (may be empty if not yet closed) */
  regionsCreated: bigint[]
}

export type OnRectangleCommit = (result: RectangleCommitResult) => void
export type OnFaceImprint = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/** Plane stage: waiting for first click, or waiting for second click, on a
 *  frozen `DrawPlane`/`SketchTarget`. */
type PlaneStage =
  | { kind: 'idle' }
  | { kind: 'anchored'; plane: DrawPlane; target: SketchTarget; anchor: V3 }

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
      anchor: V3
    }

export class RectangleTool implements Tool {
  readonly name = 'Rectangle'

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    if (this.planeStage.kind !== 'idle' || this.faceStage.kind !== 'idle') {
      return 'Click the opposite corner — or type exact dimensions.'
    }
    if (this.idlePlaneLock !== null) {
      return `Locked to the ${AXIS_LOCK_COLOR_NAMES[this.idlePlaneLock]} plane — click to start; same arrow or Esc unlocks.`
    }
    return 'Click the first corner — on the ground plane or any face or sketch.'
  }

  private planeStage: PlaneStage = { kind: 'idle' }
  private faceStage: FaceStage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnRectangleCommit
  private onFaceImprint: OnFaceImprint
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** Cached plane-mode sketch handles — the Viewport passes one cache
   *  shared by every draw tool, so mixed-tool profiles land in a single
   *  sketch per plane. */
  private readonly sketchCache: SketchPlaneCache

  /** The currently active editing context (entered object), or null at top level. */
  private _activeContext: bigint | null = null

  /** VCB buffer — raw string being typed by the user (W,D in display units) */
  private typed: string = ''

  /** Last rubber-band cursor positions, tracked for typed-entry sign/direction */
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
    onCommit: OnRectangleCommit,
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
   * other draw tools):
   *   - Already anchored in one mode: stick with it (mid-gesture).
   *   - Inside an entered object context: always face mode (drawing stays
   *     scoped to that object — no top-level plane sketch from inside).
   *   - Otherwise idle at top level: face mode if an eligible Object face is
   *     under the cursor (via `pick_face`), else plane mode (which itself
   *     resolves sketch-vs-ground via `_resolveIdleTarget`).
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
   *   and return its plane so the FIRST-click anchor lands precisely on the
   *   face, preventing the kernel from rejecting a non-planar rectangle;
   *   absent that, a top-level hover over a non-ground sketch returns ITS
   *   plane.
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
        anchoredThrough: this.planeStage.anchor,
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
      const { anchor, normal, planePoint } = this.faceStage
      // Project cursor ray onto face plane
      const cursorOnPlane = rayPlaneIntersect(ray.origin, ray.direction, planePoint, normal)
      if (cursorOnPlane === null) {
        this._clearPreview()
        this.onMeasurementCb('')
        return
      }
      this._lastFaceCursor = cursorOnPlane
      const corners = faceRectangleCorners(anchor, cursorOnPlane, normal)
      if (corners !== null) {
        this._drawRubberBandCorners(corners)
        this._reportMeasurement(corners)
      } else {
        this._clearPreview()
        if (this.typed === '') this.onMeasurementCb('')
      }
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
      const { plane, anchor } = this.planeStage
      const cursor = this._planeCursor(snap, ray, plane)
      if (cursor === null) {
        this._clearPreview()
        this.onMeasurementCb('')
        return
      }
      this._lastPlaneCursor = cursor
      if (plane.ground) {
        // EXACT legacy ground fast path — no basis math.
        const corners = rectangleCorners([anchor[0], anchor[1]], [cursor[0], cursor[1]])
        this._drawRubberBandCorners(corners)
        this._reportMeasurement(corners)
      } else {
        const corners = faceRectangleCorners(anchor, cursor, plane.normal)
        if (corners !== null) {
          this._drawRubberBandCorners(corners)
          this._reportMeasurement(corners)
        } else {
          this._clearPreview()
          if (this.typed === '') this.onMeasurementCb('')
        }
      }
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
   * Typed VCB entry is available once the first corner has been placed
   * (either plane or face mode) — see the Viewport key router, which
   * routes digit/letter/arrow keys here instead of tool-switch shortcuts
   * while this returns true.
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
      // (parity across all four draw tools — LineTool's _endChain path).
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
      // Each component goes through the length grammar, so explicit units
      // work (and can be mixed) regardless of the display format —
      // "1cm,100mm", "5',23\"" — while bare numbers stay in display units.
      const dims = parseDimensionsToMeters(this.typed)
      if (dims !== null) {
        this._commitTyped(dims[0], dims[1])
      }
      return
    }

    // Feed digits, dot, separators, explicit-unit tokens, Backspace into
    // the buffer (editDimsBuffer applies the grammar rules).
    if (
      (ev.key >= '0' && ev.key <= '9') ||
      ev.key === '.' ||
      ev.key === ',' ||
      ev.key === 'x' ||
      ev.key === 'X' ||
      ev.key === ' ' ||
      ev.key === 'Backspace' ||
      ev.key === "'" ||
      ev.key === '"' ||
      ev.key === '/' ||
      ev.key === '-' ||
      /^[mckftinMCKFTIN]$/.test(ev.key)
    ) {
      this.typed = editDimsBuffer(this.typed, ev.key)
      this.onMeasurementCb(typedReadout(this.typed))
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

  /** Report the live W × D measurement from the four rubber-band corners. */
  private _reportMeasurement(corners: readonly [V3, V3, V3, V3]): void {
    if (this.typed !== '') {
      this.onMeasurementCb(typedReadout(this.typed))
      return
    }
    const [c0, c1, c2] = corners
    const width = Math.hypot(c1[0] - c0[0], c1[1] - c0[1], c1[2] - c0[2])
    const depth = Math.hypot(c2[0] - c1[0], c2[1] - c1[1], c2[2] - c1[2])
    this.onMeasurementCb(`${formatLength(width)} × ${formatLength(depth)}`)
  }

  /**
   * Commit an exact width × depth rectangle from the typed VCB buffer, using
   * the current rubber-band cursor side (or +,+ default) to pick the growth
   * direction along each axis. Dispatches to plane or face mode depending
   * on which stage is anchored.
   */
  private _commitTyped(w: number, d: number): void {
    if (this.planeStage.kind === 'anchored') {
      const { plane, target, anchor } = this.planeStage
      // Sign of growth along each axis follows the last rubber-band cursor
      // position (so typing matches the direction the user was dragging);
      // default +,+ if the cursor hasn't moved yet.
      const cursor = this._lastPlaneCursor ?? anchor

      let corners: [V3, V3, V3, V3]
      if (plane.ground) {
        // EXACT legacy ground fast path — no basis math.
        const signX = cursor[0] - anchor[0] < 0 ? -1 : 1
        const signY = cursor[1] - anchor[1] < 0 ? -1 : 1
        const farCorner: [number, number] = [anchor[0] + signX * w, anchor[1] + signY * d]
        corners = rectangleCorners([anchor[0], anchor[1]], farCorner)
      } else {
        const basis = facePlaneBasis(plane.normal)
        if (basis === null) {
          this.cancel()
          return
        }
        const { u, v } = basis
        const dx = cursor[0] - anchor[0]
        const dy = cursor[1] - anchor[1]
        const dz = cursor[2] - anchor[2]
        const du = dx * u[0] + dy * u[1] + dz * u[2]
        const dv = dx * v[0] + dy * v[1] + dz * v[2]
        const signU = du < 0 ? -1 : 1
        const signV = dv < 0 ? -1 : 1

        const b: V3 = [anchor[0] + u[0] * signU * w, anchor[1] + u[1] * signU * w, anchor[2] + u[2] * signU * w]
        const c: V3 = [b[0] + v[0] * signV * d, b[1] + v[1] * signV * d, b[2] + v[2] * signV * d]
        const dd: V3 = [anchor[0] + v[0] * signV * d, anchor[1] + v[1] * signV * d, anchor[2] + v[2] * signV * d]
        corners = [anchor, b, c, dd]
      }

      this.planeStage = { kind: 'idle' }
      this.typed = ''
      this._lastPlaneCursor = null
      this._clearPreview()
      this.onMeasurementCb('')
      this._commitPlaneRectangle(target, corners)
    } else if (this.faceStage.kind === 'anchored') {
      const { object, face, normal, anchor } = this.faceStage
      const basis = facePlaneBasis(normal)
      if (basis === null) {
        this.cancel()
        return
      }
      const { u, v } = basis
      const cursor = this._lastFaceCursor ?? anchor
      const dx = cursor[0] - anchor[0]
      const dy = cursor[1] - anchor[1]
      const dz = cursor[2] - anchor[2]
      const du = dx * u[0] + dy * u[1] + dz * u[2]
      const dv = dx * v[0] + dy * v[1] + dz * v[2]
      const signU = du < 0 ? -1 : 1
      const signV = dv < 0 ? -1 : 1

      const b: V3 = [anchor[0] + u[0] * signU * w, anchor[1] + u[1] * signU * w, anchor[2] + u[2] * signU * w]
      const c: V3 = [b[0] + v[0] * signV * d, b[1] + v[1] * signV * d, b[2] + v[2] * signV * d]
      const dd: V3 = [anchor[0] + v[0] * signV * d, anchor[1] + v[1] * signV * d, anchor[2] + v[2] * signV * d]
      const corners: [V3, V3, V3, V3] = [anchor, b, c, dd]

      this.faceStage = { kind: 'idle' }
      this.typed = ''
      this._lastFaceCursor = null
      this._clearPreview()
      this.onMeasurementCb('')
      this._commitFaceCorners(object, face, corners)
    }
  }

  // ------------------------------------------------------------------ plane mode

  private _onPointerDownPlane(snap: Snap | null, ray: Ray): void {
    if (this.planeStage.kind === 'idle') {
      // First click: resolve (and freeze) the plane/target, then anchor.
      const resolved = this._resolveClickTarget(snap, ray)
      if (resolved === null) return
      const { plane, target } = resolved
      const anchor = this._planeCursor(snap, ray, plane)
      if (anchor === null) return
      this.planeStage = { kind: 'anchored', plane, target, anchor }
      this._lastPlaneCursor = null
    } else {
      // Second click: commit the rectangle.
      const { plane, target, anchor } = this.planeStage
      const cursor = this._planeCursor(snap, ray, plane)
      if (cursor === null) return

      if (plane.ground) {
        // Skip degenerate rectangles (same point or zero area) — the exact
        // legacy ground check.
        if (
          Math.abs(anchor[0] - cursor[0]) < 1e-8 ||
          Math.abs(anchor[1] - cursor[1]) < 1e-8
        ) {
          return
        }
        this.planeStage = { kind: 'idle' }
        this.typed = ''
        this._lastPlaneCursor = null
        this._clearPreview()
        this.onMeasurementCb('')
        this._commitPlaneRectangle(target, rectangleCorners([anchor[0], anchor[1]], [cursor[0], cursor[1]]))
      } else {
        const corners = faceRectangleCorners(anchor, cursor, plane.normal)
        if (corners === null) return // degenerate — ignore
        this.planeStage = { kind: 'idle' }
        this.typed = ''
        this._lastPlaneCursor = null
        this._clearPreview()
        this.onMeasurementCb('')
        this._commitPlaneRectangle(target, corners)
      }
    }
  }

  /** Commit a rectangle loop (four `sketch_add_segment` calls) into
   *  `target`'s sketch — used by both ground and non-ground plane/sketch
   *  mode (real face mode instead imprints via `split_face_inner`, see
   *  `_commitFaceCorners`). */
  private _commitPlaneRectangle(target: SketchTarget, corners: [V3, V3, V3, V3]): void {
    try {
      runSketchGesture(this.wasmScene, this.sketchCache, target, (sketch) => {
        // Four edges: 0→1, 1→2, 2→3, 3→0
        const edges = [
          [corners[0], corners[1]],
          [corners[1], corners[2]],
          [corners[2], corners[3]],
          [corners[3], corners[0]],
        ] as const

        let lastRegionsCreated: bigint[] = []
        for (const [p, q] of edges) {
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
      const anchor: V3 = [snap.x, snap.y, snap.z]

      this.faceStage = {
        kind: 'anchored',
        object: eligible.object,
        face: eligible.face,
        normal,
        planePoint: anchor,
        anchor,
      }
      this._lastFaceCursor = null
    } else {
      // Second click: commit the face imprint
      const { object, face, normal, planePoint, anchor } = this.faceStage

      // Project the click ray onto the face plane for the cursor position
      const cursorOnPlane = rayPlaneIntersect(ray.origin, ray.direction, planePoint, normal)
      if (cursorOnPlane === null) return

      const corners = faceRectangleCorners(anchor, cursorOnPlane, normal)
      if (corners === null) return // degenerate — ignore

      this.faceStage = { kind: 'idle' }
      this.typed = ''
      this._lastFaceCursor = null
      this._clearPreview()
      this.onMeasurementCb('')

      this._commitFaceCorners(object, face, corners)
    }
  }

  /** Split the given face with a rectangle loop defined by 4 explicit world-space corners. */
  private _commitFaceCorners(object: bigint, face: bigint, corners: [V3, V3, V3, V3]): void {
    // Flatten the 4 corners into a Float64Array of xyz triples
    const loopPts = new Float64Array(4 * 3)
    for (let i = 0; i < 4; i++) {
      loopPts[i * 3 + 0] = corners[i][0]
      loopPts[i * 3 + 1] = corners[i][1]
      loopPts[i * 3 + 2] = corners[i][2]
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

  // ------------------------------------------------------------------ preview

  /**
   * Emit a LineSegments preview for a closed 4-corner loop. Corners are used
   * exactly as given — the preview's depth bias (PREVIEW_LINE_STYLE,
   * depthPolicy.ts) settles coincidence with the ground/committed lines, so
   * no z-lift.
   *
   * @param corners  Four world-space xyz corners in order.
   */
  private _drawRubberBandCorners(corners: readonly [V3, V3, V3, V3]): void {
    this._clearPreview()
    const [c0, c1, c2, c3] = corners
    const pts = new Float32Array([
      ...c0, ...c1,
      ...c1, ...c2,
      ...c2, ...c3,
      ...c3, ...c0,
    ])
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
