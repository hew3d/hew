/**
 * CircleTool — two-click faceted-circle (regular N-gon) sketching.
 *
 * A "circle" here is a faceted regular N-gon (true arcs are out of scope —
 * see). It decomposes into N chained `sketch_add_segment` calls (plane
 * mode) or one `split_face_inner` call with N loop points (face mode) —
 * mirroring exactly how RectangleTool works with 4 corners. No kernel change
 * is needed; push/pull then works for free once a closed loop forms a
 * region.
 *
 * Two modes:
 *
 * Mode is decided by what's under the cursor per pointer event (mirrors
 * LineTool; see `faceDraw.ts` for the shared plain-object eligibility
 * policy), never by `_activeContext` alone.
 *
 * Plane mode (no eligible face under the cursor — sketches on any plane,
 * design doc §1/§4): the drawing plane is resolved once, at the FIRST
 * click, and frozen for the rest of the gesture:
 *   - A top-level hover over a committed sketch whose plane is non-ground
 *     (`pick_sketch` + `planeFromSketch`) adopts THAT sketch's plane —
 *     SKETCH MODE — and every facet segment lands in that one sketch
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
 *      cursor (radius = distance from center to cursor; start angle = angle
 *      from center to cursor)
 *   3. Second click: commit — N sketch_add_segment calls chaining
 *      vertex[i] -> vertex[i+1], last one vertex[N-1] -> vertex[0] (using
 *      the SAME stored vertex[0] coords for exact closure), via
 *      `runSketchGesture`
 *   4. Esc between clicks: cancel stage 1
 *   Calls onCommit() after each successful commit so the viewport can
 *   refresh scene geometry and trigger re-render.
 *
 * Face mode (an eligible Object face is under the cursor):
 *   1. First click on an eligible face: anchor center (on face plane)
 *   2. Move: rubber-band N-gon preview projected onto that face plane
 *   3. Second click: commit — split_face_inner() on that face
 *      with N loop points
 *   4. Esc: cancel
 *   Calls onFaceImprint(objectId) after each successful imprint so the
 *   viewport can refresh the scene.
 *
 * VCB: the radius is a SINGLE length (unlike Rectangle's W x D), so typed
 * entry mirrors LineTool's single-length VCB style (editLengthBuffer /
 * parseLengthToMeters), not Rectangle's editDimsBuffer/parseDimensions.
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
import { editLengthBuffer, isLengthInputKey, nextIdlePlaneLock, AXIS_LOCK_COLOR_NAMES } from './moveInput'
import { segmentLength } from './lineInput'
import { runSketchGesture, makeSketchPlaneCache, type SketchPlaneCache, type SketchTarget } from './sketchGesture'
import { pointOnPlane, drawPlaneCue, isGroundPlane, SketchPickCache, resolveIdleDrawTarget, resolveClickDrawTarget, type DrawPlane } from './drawPlane'
import { FacePickCache, defaultFaceEligible, type FaceEligible } from './faceDraw'

import { segmentsPerTurn } from './arcMath'

/** Floor of the adaptive facet count (the true-curves design §6): small
 * circles are regular 24-gons; larger radii adapt up via `segmentsPerTurn`
 * so the chord sagitta stays within the draw-time budget. The analytic
 * center/radius rides the curve chain regardless. */
export const CIRCLE_SEGMENTS = 24

/** Adaptive facet count for a ground circle (center/rim in plane coords). */
function groundSegments(center: [number, number], rim: [number, number]): number {
  return segmentsPerTurn(Math.hypot(rim[0] - center[0], rim[1] - center[1]))
}

/** Adaptive facet count for an on-plane (non-ground) circle (center/rim in world coords). */
function faceSegments(center: V3, rim: V3): number {
  return segmentsPerTurn(
    Math.hypot(rim[0] - center[0], rim[1] - center[1], rim[2] - center[2]),
  )
}

export type CircleCommitResult = {
  sketchHandle: bigint
  /** Handles of regions created by the last segment (may be empty if not yet closed) */
  regionsCreated: bigint[]
}

export type OnCircleCommit = (result: CircleCommitResult) => void
export type OnFaceImprint = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

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


export class CircleTool implements Tool {
  readonly name = 'Circle'

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    if (this.planeStage.kind !== 'idle' || this.faceStage.kind !== 'idle') {
      return 'Click to set the radius — or type an exact radius.'
    }
    if (this.idlePlaneLock !== null) {
      return `Locked to the ${AXIS_LOCK_COLOR_NAMES[this.idlePlaneLock]} plane — click to start; same arrow or Esc unlocks.`
    }
    return "Click the circle's center — on the ground plane or any face or sketch."
  }

  private planeStage: PlaneStage = { kind: 'idle' }
  private faceStage: FaceStage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnCircleCommit
  private onFaceImprint: OnFaceImprint
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** Cached plane-mode sketch handles — the Viewport passes one cache
   *  shared by every draw tool, so mixed-tool profiles land in a single
   *  sketch per plane. */
  private readonly sketchCache: SketchPlaneCache

  /** The currently active editing context (entered object), or null at top level. */
  private _activeContext: bigint | null = null

  /** VCB buffer — raw string being typed by the user (radius, in display units) */
  private typed: string = ''

  /** Last rubber-band cursor positions, tracked for typed-entry direction */
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
    onCommit: OnCircleCommit,
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
      const { center, normal, planePoint } = this.faceStage
      // Project cursor ray onto face plane
      const cursorOnPlane = rayPlaneIntersect(ray.origin, ray.direction, planePoint, normal)
      if (cursorOnPlane === null) {
        this._clearPreview()
        this.onMeasurementCb('')
        return
      }
      this._lastFaceCursor = cursorOnPlane
      const verts = circlePolygonFace(center, cursorOnPlane, normal, faceSegments(center, cursorOnPlane))
      if (verts !== null) {
        this._drawRubberBandFace(verts)
        this._reportMeasurement(center, cursorOnPlane)
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
      const { plane, center } = this.planeStage
      const cursor = this._planeCursor(snap, ray, plane)
      if (cursor === null) {
        this._clearPreview()
        this.onMeasurementCb('')
        return
      }
      this._lastPlaneCursor = cursor
      if (plane.ground) {
        // EXACT legacy ground fast path — no basis math.
        const centerXY: [number, number] = [center[0], center[1]]
        const cursorXY: [number, number] = [cursor[0], cursor[1]]
        const verts = circlePolygonGround(centerXY, cursorXY, groundSegments(centerXY, cursorXY))
        if (verts.length > 0) {
          this._drawRubberBandGround(verts)
          this._reportMeasurement(center, cursor)
        } else {
          this._clearPreview()
          if (this.typed === '') this.onMeasurementCb('')
        }
      } else {
        const verts = circlePolygonFace(center, cursor, plane.normal, faceSegments(center, cursor))
        if (verts !== null) {
          this._drawRubberBandFace(verts)
          this._reportMeasurement(center, cursor)
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
      if (this.typed === '') return
      const meters = parseLengthToMeters(this.typed)
      if (meters !== null) {
        this._commitTyped(meters)
      }
      return
    }

    // Feed digits, dot, separators, Backspace into the buffer
    if (isLengthInputKey(ev.key)) {
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
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

  /** The typed-buffer readout, suffixed for metric formats (imperial tokens
   * like `'`/`"` are already visible in the buffer itself). */
  private _typedReadout(): string {
    return typedReadout(this.typed)
  }

  /** Report the live radius measurement from center to the cursor. */
  private _reportMeasurement(center: V3, cursor: V3): void {
    if (this.typed !== '') {
      this.onMeasurementCb(this._typedReadout())
      return
    }
    const radius = Math.hypot(cursor[0] - center[0], cursor[1] - center[1], cursor[2] - center[2])
    this.onMeasurementCb(`R ${formatLength(radius)}`)
  }

  /**
   * Commit an exact-radius circle from the typed VCB buffer, using the last
   * rubber-band cursor to pick the start-angle/direction (default +X if the
   * cursor hasn't moved yet). Dispatches to plane or face mode depending on
   * which stage is anchored.
   */
  private _commitTyped(radius: number): void {
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
        rim = [center[0] + dir[0] * radius, center[1] + dir[1] * radius, 0]
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
          center[0] + u[0] * dirU * radius + v[0] * dirV * radius,
          center[1] + u[1] * dirU * radius + v[1] * dirV * radius,
          center[2] + u[2] * dirU * radius + v[2] * dirV * radius,
        ]
      }

      this.planeStage = { kind: 'idle' }
      this.typed = ''
      this._lastPlaneCursor = null
      this._clearPreview()
      this.onMeasurementCb('')
      this._commitPlaneCircle(plane, target, center, rim)
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
        center[0] + u[0] * dirU * radius + v[0] * dirV * radius,
        center[1] + u[1] * dirU * radius + v[1] * dirV * radius,
        center[2] + u[2] * dirU * radius + v[2] * dirV * radius,
      ]

      this.faceStage = { kind: 'idle' }
      this.typed = ''
      this._lastFaceCursor = null
      this._clearPreview()
      this.onMeasurementCb('')
      this._commitFaceCircle(object, face, center, rim, normal)
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
      // Second click: commit the circle.
      const { plane, target, center } = this.planeStage
      const cursor = this._planeCursor(snap, ray, plane)
      if (cursor === null) return

      // Skip degenerate circles (zero radius). The ground branch keeps the
      // EXACT legacy per-axis check — bit-identical gating to before this
      // module existed; non-ground uses the Euclidean check face mode
      // already uses.
      if (plane.ground) {
        if (Math.hypot(cursor[0] - center[0], cursor[1] - center[1]) < 1e-7) return
      } else if (segmentLength(center, cursor) < 1e-7) {
        return
      }

      this._commitPlaneCircle(plane, target, center, cursor)
      this.planeStage = { kind: 'idle' }
      this.typed = ''
      this._lastPlaneCursor = null
      this._clearPreview()
      this.onMeasurementCb('')
    }
  }

  /** Commit a circle (N chained `sketch_add_segment` calls bracketed as one
   *  curve chain) into `target`'s sketch — used by both ground and
   *  non-ground plane/sketch mode (real face mode instead imprints via
   *  `split_face_inner_with_curve`, see `_commitFaceCircle`). */
  private _commitPlaneCircle(plane: DrawPlane, target: SketchTarget, center: V3, rim: V3): void {
    const verts = plane.ground
      ? circlePolygonGround(
          [center[0], center[1]],
          [rim[0], rim[1]],
          groundSegments([center[0], center[1]], [rim[0], rim[1]]),
        )
      : circlePolygonFace(center, rim, plane.normal, faceSegments(center, rim))
    if (verts === null || verts.length === 0) return // degenerate — ignore

    try {
      runSketchGesture(this.wasmScene, this.sketchCache, target, (sketch) => {
        let lastRegionsCreated: bigint[] = []
        // The whole circle is ONE curve chain — clicking any facet later
        // selects (and deletes) the circle as a unit — and it carries the
        // exact analytic circle the facets approximate (durable
        // center/radius — the true-curves design). `center[2]` is exactly 0
        // on the ground plane (the legacy fast path), or the plane-frame
        // world z on any other plane.
        const radius = plane.ground
          ? Math.hypot(rim[0] - center[0], rim[1] - center[1])
          : segmentLength(center, rim)
        this.wasmScene.sketch_begin_curve_with(sketch, center[0], center[1], center[2], radius)
        try {
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
        } finally {
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

      const verts = circlePolygonFace(center, cursorOnPlane, normal, faceSegments(center, cursorOnPlane))
      if (verts === null) return // degenerate — ignore

      this.faceStage = { kind: 'idle' }
      this.typed = ''
      this._lastFaceCursor = null
      this._clearPreview()
      this.onMeasurementCb('')

      this._commitFaceVerts(object, face, verts, center)
    }
  }

  /** Split the given face with a circle loop defined by center/rim/normal. */
  private _commitFaceCircle(object: bigint, face: bigint, center: V3, rim: V3, normal: V3): void {
    const verts = circlePolygonFace(center, rim, normal, faceSegments(center, rim))
    if (verts === null) return // degenerate — ignore
    this._commitFaceVerts(object, face, verts, center)
  }

  /**
   * Split the given face with a circle loop defined by N explicit world-space
   * vertices, carrying the drawn circle's analytic identity (center + radius)
   * onto the solid so a later push-through of the imprinted disk shades smooth
   * and offsets its radius, rather than leaving faceted tunnel walls
   * (the true-curves design, playtest fix C3). The radius is measured to
   * the loop's own first vertex, so it matches the imprinted points exactly
   * (the kernel refuses a claim that does not describe the loop).
   */
  private _commitFaceVerts(object: bigint, face: bigint, verts: V3[], center: V3): void {
    // Flatten the N vertices into a Float64Array of xyz triples
    const loopPts = new Float64Array(verts.length * 3)
    for (let i = 0; i < verts.length; i++) {
      loopPts[i * 3 + 0] = verts[i][0]
      loopPts[i * 3 + 1] = verts[i][1]
      loopPts[i * 3 + 2] = verts[i][2]
    }
    const radius = Math.hypot(
      verts[0][0] - center[0],
      verts[0][1] - center[1],
      verts[0][2] - center[2],
    )

    try {
      this.wasmScene.split_face_inner_with_curve(
        object,
        face,
        loopPts,
        new Float64Array([center[0], center[1], center[2]]),
        radius,
      )
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
   * Draw a rubber-band N-gon from ground-plane vertices.
   * Used in ground (plane-mode) drawing.
   */
  private _drawRubberBandGround(verts: V3[]): void {
    this._clearPreview()
    this._drawRubberBandVerts(verts)
  }

  /**
   * Draw a rubber-band N-gon from explicit 3D vertices.
   * Used in face mode and non-ground plane mode — vertices already lie on
   * the target plane.
   */
  private _drawRubberBandFace(verts: V3[]): void {
    this._clearPreview()
    this._drawRubberBandVerts(verts)
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
