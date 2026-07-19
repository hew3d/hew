/**
 * LineTool — SketchUp-style chained-segment line drawing.
 *
 * Mode is decided by what's under the cursor — NOT by whether an editing
 * context is active. Eligibility of face mode follows the shared plain-object
 * policy in `faceDraw.ts` (or a richer predicate the Viewport injects via
 * `setFaceEligibility`):
 *   - Top level: any PLAIN object's face is eligible — ungrouped, not part
 *     of a component instance. Groups/Components keep their explicit
 *     double-click editing step.
 *   - Inside a context: only the entered object's faces are eligible.
 * This is what makes the #1 workflow — draw a line on a solid's face, at top
 * level, to split that face — work without first entering the object.
 *
 * Plane mode (sketches on any plane — no eligible face under the cursor;
 * design doc §1/§4): the drawing plane is resolved once, at the FIRST click
 * of a chain, and frozen for the rest of the gesture:
 *   - A top-level hover over a committed sketch whose plane is non-ground
 *     (`pick_sketch` + `planeFromSketch`) adopts THAT sketch's plane —
 *     SKETCH MODE — and every segment lands in that one sketch
 *     (`SketchTarget.existing`).
 *   - Otherwise the plane is the ground plane — PLANE MODE, today's
 *     behavior — and segments land in the shared per-plane cached sketch
 *     (`SketchTarget.plane`; `begin_ground_sketch()` on a cache miss).
 *   On the ground plane every point is `[snap.x, snap.y, 0]` EXACTLY as
 *   before this generalization (no basis math) — bit-identical committed
 *   coordinates. On a non-ground plane, the cursor is the snap (already
 *   plane-constrained via `snapConstraint`) or, absent one, ray∩plane.
 *
 *   1. First click: anchor the first point.
 *   2. Each subsequent click commits ONE segment from the previous point to
 *      the new point via `sketch_add_segment`. The new point becomes the
 *      anchor for the next segment (chain forward).
 *   3. Rubber-band preview: a single line from the last placed point to the
 *      live cursor.
 *   4. Closing the loop: when a commit's regions_created() is non-empty, a
 *      face formed — call onCommit so the viewport refreshes, then end the
 *      chain (back to idle, ready to start a new line).
 *   5. Finishing without closing: Enter on an empty buffer, a double-click,
 *      or Escape ends the chain but KEEPS the committed sketch geometry —
 *      just resets the tool to idle. A second Escape (already idle) is a
 *      full cancel().
 *
 * Face mode (an eligible Object face is under the cursor):
 *   1. Idle: snapConstraint picks the hovered eligible face and returns its
 *      plane, so the first point lands on the plane.
 *   2. Each click accumulates a point on that (now-locked) face plane.
 *   3. Rubber-band preview: a line from the last point to the cursor,
 *      projected onto the plane.
 *   4. On finish (Enter / double-click / Escape with >= 2 points), the
 *      accumulated path is flattened into a Float64Array and passed to
 *      split_face(object, face, path) — the boundary-to-boundary face cut.
 *      onFaceImprint(object) refreshes the viewport. A typed kernel error is
 *      surfaced via the toast callback; no geometry fix-up is attempted.
 *
 * VCB length entry (both modes): capturingInput() is true once at least one
 * point is placed. Typing digits/./- (and imperial tokens) feeds
 * editLengthBuffer; Enter commits a segment of the exact typed length along
 * normalize(cursor - prev) (falling back to the last rubber-band direction;
 * ignored if no direction is available yet).
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { V3 } from '../viewport/geoHelpers'
import { rayPlaneIntersect, facePlaneBasis } from '../viewport/geoHelpers'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { makeFatSegments, disposeFatSegments, PREVIEW_LINE_STYLE } from '../viewport/fatLine'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { arrowToAxis, editLengthBuffer, isLengthInputKey, pointAlong, nextIdlePlaneLock, AXIS_LOCK_COLOR_NAMES } from './moveInput'
import { segmentLength, directionBetween } from './lineInput'
import { runSketchGesture, makeSketchPlaneCache, type SketchPlaneCache, type SketchTarget } from './sketchGesture'
import { groundDrawPlane, planeFromSketch, pointOnPlane, axisDrawPlane, drawPlaneCue, isGroundPlane, SketchPickCache, type DrawPlane } from './drawPlane'
import { FacePickCache, defaultFaceEligible, type FaceEligible } from './faceDraw'

export type OnLineCommit = (sketchHandle: bigint) => void
export type OnFaceImprint = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/** Plane chain: idle, or anchored on a frozen `DrawPlane`/`SketchTarget`
 *  with the last placed point (world-space; z = 0 exactly on the ground
 *  plane — see the module doc). */
type PlaneStage =
  | { kind: 'idle' }
  | { kind: 'anchored'; plane: DrawPlane; target: SketchTarget; anchor: V3 }

/** Face chain: idle, or anchored on a specific face plane with accumulated points. */
type FaceStage =
  | { kind: 'idle' }
  | {
      kind: 'anchored'
      object: bigint
      face: bigint
      normal: V3
      /** A world-space point that lies on the face plane (the first click). */
      planePoint: V3
      /** All points placed so far on the plane, in order. */
      points: V3[]
    }


export class LineTool implements Tool {
  readonly name = 'Line'

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    if (this.planeStage.kind !== 'idle' || this.faceStage.kind !== 'idle') {
      return 'Click the next point — type a length for an exact segment; double-click or Esc to finish.'
    }
    if (this.idlePlaneLock !== null) {
      return `Locked to the ${AXIS_LOCK_COLOR_NAMES[this.idlePlaneLock]} plane — click to start; same arrow or Esc unlocks.`
    }
    return 'Click to start a line — on the ground plane or any face or sketch.'
  }

  private planeStage: PlaneStage = { kind: 'idle' }
  private faceStage: FaceStage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnLineCommit
  private onFaceImprint: OnFaceImprint
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** Cached plane-mode sketch handles — the Viewport passes one cache
   *  shared by every draw tool, so mixed-tool profiles land in a single
   *  sketch per plane. */
  private readonly sketchCache: SketchPlaneCache

  /** The currently active editing context (entered object), or null at top level. */
  private _activeContext: bigint | null = null

  /** VCB buffer — raw string being typed by the user (length, in display units) */
  private typed: string = ''

  /** Last rubber-band cursor positions, tracked for typed-entry direction */
  private _lastPlaneCursor: V3 | null = null
  private _lastFaceCursor: V3 | null = null


  /** Current axis lock: 0=X, 1=Y, 2=Z, null=free. Mirrors MoveTool. */
  private lockAxis: 0 | 1 | 2 | null = null
  /** True when the *current* axis lock was set by holding Shift (vs. an arrow). */
  private shiftAxisLock: boolean = false

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

  /** Per-pointer-event `pick_face` memo — see `FacePickCache` in faceDraw.ts. */
  private readonly _pickCache = new FacePickCache()
  /** Per-pointer-event `pick_sketch` memo — see `SketchPickCache` in drawPlane.ts. */
  private readonly _sketchPickCache = new SketchPickCache()

  /** Run `pick_face` for `ray` and return the eligible {object, face} pair
   *  (or null), reusing a cached result for the same `ray` reference if one
   *  was already computed earlier in this same pointer event. */
  private _eligiblePickFor(ray: Ray): { object: bigint; face: bigint } | null {
    return this._pickCache.pickFor(this.wasmScene, ray, (object, instance) =>
      this._isEligible(object, instance))
  }

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnLineCommit,
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

  /**
   * Provide snap constraints: a constraint plane while drawing on a face or
   * a non-ground plane/sketch (so off-plane/occluded geometry is excluded),
   * and/or an axis lock (arrow keys / Shift, mirroring MoveTool) once a
   * chain is anchored.
   *
   * - Anchored (plane or face): always include `anchor` (the last placed
   *   point) — the inference engine derives anchor-dependent candidates
   *   from it (a Tangent snap is "the rim point where the segment from the
   *   anchor touches the circle", the true-curves design). With an
   *   axis locked, additionally include `lockAxis` so the snap collapses
   *   onto the locked line.
   * - Face-anchored: ALSO return the known face plane's `constraintPlane`
   *   (unconditionally — independent of any axis lock) so subsequent snaps
   *   stay on that plane.
   * - Plane-anchored on a NON-ground plane (sketch mode): same —
   *   `constraintPlane` from the frozen plane. Ground-anchored keeps
   *   today's unconstrained behavior (just the axis lock, if any).
   * - Idle: pick the hovered face (any eligible Object, scoped by
   *   `_activeContext` exactly like PushPullTool) and return its plane so
   *   the FIRST-click point lands precisely on the face; absent that, a
   *   top-level hover over a non-ground sketch returns ITS plane so the
   *   first click lands on it.
   * - No eligible face/sketch under the cursor and nothing anchored: return
   *   null (ground, unconstrained).
   */
  snapConstraint(ray: Ray): {
    anchor?: [number, number, number]
    lockAxis?: 0 | 1 | 2
    constraintPlane?: { point: [number, number, number]; normal: [number, number, number] }
  } | null {
    const lockPart: { anchor?: [number, number, number]; lockAxis?: 0 | 1 | 2 } = {}
    const anchorPoint = this._currentAnchor()
    if (anchorPoint !== null) {
      lockPart.anchor = anchorPoint
      if (this.lockAxis !== null) {
        lockPart.lockAxis = this.lockAxis
      }
    }

    if (this.faceStage.kind === 'anchored') {
      return {
        ...lockPart,
        constraintPlane: {
          point: this.faceStage.planePoint,
          normal: this.faceStage.normal,
        },
      }
    }

    if (this.planeStage.kind === 'anchored') {
      if (this.planeStage.plane.ground) {
        // No constraint plane while ground-anchored; just the axis lock (if any).
        return Object.keys(lockPart).length > 0 ? lockPart : null
      }
      return {
        ...lockPart,
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
    if (this.idlePlaneLock !== null) {
      return Object.keys(lockPart).length > 0 ? lockPart : null
    }

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

  /** The last placed point of whichever chain is currently anchored, or null. */
  private _currentAnchor(): [number, number, number] | null {
    if (this.faceStage.kind === 'anchored') {
      const { points } = this.faceStage
      return points[points.length - 1]
    }
    if (this.planeStage.kind === 'anchored') {
      return this.planeStage.anchor
    }
    return null
  }

  /** Optional richer eligibility, injected by the Viewport (which knows the
   *  full group/instance context path the tool can't see). Null = the shared
   *  default policy in faceDraw.ts. */
  private _faceEligible: FaceEligible | null = null
  setFaceEligibility(pred: FaceEligible | null): void {
    this._faceEligible = pred
  }

  /** Plain objects are directly drawable at the top level; inside an entered
   *  object context only that object's faces are ( scoped editing). Groups
   *  and Components keep their explicit editing step — see faceDraw.ts. */
  private _isEligible(objectHandle: bigint, instanceHandle: bigint | undefined): boolean {
    if (this._faceEligible !== null) return this._faceEligible(objectHandle, instanceHandle)
    return defaultFaceEligible(this.wasmScene, this._activeContext, objectHandle, instanceHandle)
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

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (this._currentMode(ray) === 'face') {
      this._onPointerMoveFace(snap, ray)
    } else {
      this._onPointerMovePlane(snap, ray)
    }
  }

  /**
   * Decide which mode governs the NEXT pointer event (same contract as
   * Rectangle/Circle/Arc):
   *   - Already anchored in one mode: stick with it (mid-chain).
   *   - Inside an entered object context: always face mode — drawing stays
   *     scoped to that object's faces, so a click elsewhere is ignored by
   *     the face handler rather than falling through to a TOP-LEVEL plane
   *     sketch from inside the context.
   *   - Otherwise idle at top level: face mode if an eligible Object face is
   *     under the cursor (via `pick_face`), else plane mode (which itself
   *     resolves sketch-vs-ground via `_resolveIdleTarget`).
   */
  private _currentMode(ray?: Ray): 'face' | 'plane' {
    if (this.faceStage.kind === 'anchored') return 'face'
    if (this.planeStage.kind === 'anchored') return 'plane'
    if (this._activeContext !== null) return 'face'
    // An active idle plane lock beats face pick and sketch-hover adoption
    // (design §5.2) — the user already chose a plane.
    if (this.idlePlaneLock !== null) return 'plane'
    if (ray === undefined) return 'plane'

    return this._eligiblePickFor(ray) !== null ? 'face' : 'plane'
  }

  private _onPointerMovePlane(snap: Snap | null, ray: Ray): void {
    if (this.planeStage.kind !== 'anchored') {
      // Idle-locked: track the hover snap for `activeDrawPlaneCue()` (design
      // §6 bullet 1) — the cue previews the plane through wherever the FIRST
      // click would land right now.
      if (this.idlePlaneLock !== null && snap !== null) {
        this._lastIdleHoverPoint = [snap.x, snap.y, snap.z]
      }
      this._clearPreview()
      if (this.typed === '') this.onMeasurementCb('')
      return
    }
    const { plane, anchor } = this.planeStage
    const cursor = this._planeCursor(snap, ray, plane)
    if (cursor === null) {
      this._clearPreview()
      if (this.typed === '') this.onMeasurementCb('')
      return
    }
    this._lastPlaneCursor = cursor
    this._clearPreview()
    this._drawRubberBandSegment(anchor, cursor)
    this._reportMeasurement(anchor, cursor)
    this._publishTransient(cursor)
  }

  private _onPointerMoveFace(snap: Snap | null, ray: Ray): void {
    if (this.faceStage.kind !== 'anchored') {
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
    this._lastFaceCursor = cursor
    const { points } = this.faceStage
    const last = points[points.length - 1]
    this._clearPreview()
    this._drawRubberBandSegment(last, cursor)
    this._reportMeasurement(last, cursor)
    this._publishTransient(cursor)
  }

  /**
   * The cursor's position on the locked face plane. Prefers the SNAPPED point
   * when one is available — the snap is already constrained to this face's
   * plane (see `snapConstraint`), so this is how the line snaps to the face's
   * edges/vertices/midpoints and honors an arrow/Shift axis lock. Falls back
   * to the raw ray∩plane intersection only when nothing snapped (e.g. the
   * cursor is past the face's extent). Returns null only if not face-anchored
   * or the ray is parallel to the plane.
   */
  private _faceCursor(snap: Snap | null, ray: Ray): V3 | null {
    if (this.faceStage.kind !== 'anchored') return null
    if (snap !== null) return [snap.x, snap.y, snap.z]
    const { planePoint, normal } = this.faceStage
    return rayPlaneIntersect(ray.origin, ray.direction, planePoint, normal)
  }

  /**
   * Publish the in-progress geometry as transient snap candidates so the
   * line being drawn can snap to its own just-placed points (Phase B).
   *
   * - Face mode: every consecutive pair of accumulated `points` (none of
   *   which touch the kernel sketch until `split_face` commits) plus a
   *   trailing rubber-band segment to `cursor` (if given).
   * - Plane mode: committed segments are already persistent via
   *   `reconcile`/`register_sketch`, so the only transient needed is the
   *   live rubber band from the anchor to `cursor`.
   *
   * Always clears the previous publish first (replace semantics).
   */
  private _publishTransient(cursor: V3 | null): void {
    this.wasmScene.clear_transient_segments()

    if (this.faceStage.kind === 'anchored') {
      const { points } = this.faceStage
      for (let i = 0; i < points.length - 1; i++) {
        this._publishSegment(points[i], points[i + 1])
      }
      if (cursor !== null) {
        this._publishSegment(points[points.length - 1], cursor)
      }
    } else if (this.planeStage.kind === 'anchored' && cursor !== null) {
      this._publishSegment(this.planeStage.anchor, cursor)
    }
  }

  /** One transient segment, in the `add_transient_segment(ax,ay,az,bx,by,bz)` ffi shape. */
  private _publishSegment(a: V3, b: V3): void {
    this.wasmScene.add_transient_segment(a[0], a[1], a[2], b[0], b[1], b[2])
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    // The phantom second pointerdown of a double-click (used to finish a
    // chain) is suppressed upstream in the Viewport by `ev.detail >= 2`, so a
    // genuine double-click places exactly one point then `onDoubleClick` ends
    // the chain. Every distinct click reaches here, regardless of cadence.
    if (this._currentMode(ray) === 'face') {
      this._onPointerDownFace(snap, ray)
    } else {
      this._onPointerDownPlane(snap, ray)
    }
  }

  /**
   * Typed VCB entry is available once the first point has been placed
   * (either plane or face mode) — mirrors RectangleTool.
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
      this._onEscape()
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

    // ── Axis lock via arrow keys (mirrors MoveTool) ──
    if (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft' || ev.key === 'ArrowUp' || ev.key === 'ArrowDown') {
      const requested = arrowToAxis(ev.key)
      if (requested === null || requested === this.lockAxis) {
        // ArrowDown, or pressing same arrow again → clear lock
        this.lockAxis = null
      } else {
        this.lockAxis = requested
      }
      // An explicit arrow lock supersedes any Shift-held lock.
      this.shiftAxisLock = false
      return
    }

    if (ev.key === 'Enter') {
      if (this.typed === '') {
        // Enter on an empty buffer finishes the chain without closing.
        this._endChain()
        return
      }
      const meters = parseLengthToMeters(this.typed)
      if (meters !== null) {
        this._commitTyped(meters)
      }
      return
    }

    if (isLengthInputKey(ev.key)) {
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
      this.onMeasurementCb(this._typedReadout())
    }
  }

  /**
   * Shift-held axis lock (mirrors MoveTool's behavior): pressing Shift
   * while a chain is anchored and the live rubber-band direction has a
   * dominant axis locks to it; releasing Shift clears that lock. An explicit
   * arrow lock takes precedence and is left alone.
   */
  setShiftHeld(held: boolean): void {
    if (!this.capturingInput()) return
    if (held) {
      if (this.lockAxis !== null) return
      const axis = this._dominantAxis()
      if (axis === null) return
      this.lockAxis = axis
      this.shiftAxisLock = true
    } else if (this.shiftAxisLock) {
      this.lockAxis = null
      this.shiftAxisLock = false
    }
  }

  /** The world axis the current anchor→cursor segment is most aligned with, or null. */
  private _dominantAxis(): 0 | 1 | 2 | null {
    const anchor = this._currentAnchor()
    if (anchor === null) return null
    const cursor = this.faceStage.kind === 'anchored'
      ? this._lastFaceCursor
      : this._lastPlaneCursor
    if (cursor === null) return null

    const dx = cursor[0] - anchor[0]
    const dy = cursor[1] - anchor[1]
    const dz = cursor[2] - anchor[2]
    const ax = Math.abs(dx), ay = Math.abs(dy), az = Math.abs(dz)
    const max = Math.max(ax, ay, az)
    if (max < 1e-9) return null
    if (max === ax) return 0
    if (max === ay) return 1
    return 2
  }

  /**
   * Double-click ends the current chain (keeping committed geometry), same
   * as Enter-on-empty-buffer/Escape. Returns true (handled) only while a
   * chain is actually in progress, so an idle Line tool still falls through
   * to the Viewport's default "enter context" double-click gesture.
   */
  onDoubleClick(_snap: Snap | null, _ray: Ray): boolean {
    if (!this.capturingInput()) return false
    this._endChain()
    return true
  }

  /** The typed-buffer readout, suffixed for metric formats (imperial tokens
   * like `'`/`"` are already visible in the buffer itself). */
  private _typedReadout(): string {
    return typedReadout(this.typed)
  }

  /** Escape: first ends the in-progress chain (keeping committed geometry);
   * a second Escape (already idle) is a full cancel(). */
  private _onEscape(): void {
    if (this.planeStage.kind === 'anchored' || this.faceStage.kind === 'anchored') {
      this._endChain()
    } else {
      this.cancel()
    }
  }

  /**
   * End the current chain WITHOUT discarding committed sketch geometry —
   * just resets the tool to idle, ready to start a new line. If the face
   * stage has >= 2 accumulated points, commit the cut first.
   */
  private _endChain(): void {
    if (this.faceStage.kind === 'anchored' && this.faceStage.points.length >= 2) {
      const { object, face, points } = this.faceStage
      this._commitFacePath(object, face, points)
    }
    this.planeStage = { kind: 'idle' }
    this.faceStage = { kind: 'idle' }
    this.typed = ''
    this._lastPlaneCursor = null
    this._lastFaceCursor = null
    this.lockAxis = null
    this.shiftAxisLock = false
    this._clearPreview()
    this.onMeasurementCb('')
    this.wasmScene.clear_transient_segments()
  }

  cancel(): void {
    this.planeStage = { kind: 'idle' }
    this.faceStage = { kind: 'idle' }
    this.typed = ''
    this._lastPlaneCursor = null
    this._lastFaceCursor = null
    this.lockAxis = null
    this.shiftAxisLock = false
    this.idlePlaneLock = null
    this._lastIdleHoverPoint = null
    this._clearPreview()
    this.onMeasurementCb('')
    this.wasmScene.clear_transient_segments()
  }

  /**
   * A new/loaded document replaced the Scene, so every cached plane-mode
   * sketch handle is now stale (reusing one throws UnknownSketch). Drop them
   * all and reset to idle. The Viewport calls this from `notifyLoaded`.
   * Re-pressing the Line shortcut while Line is already active does NOT
   * recreate the tool, so this hook — not tool re-instantiation — is what
   * clears the stale handles.
   */
  onDocumentReset(): void {
    this.sketchCache.clear()
    this.cancel()
  }

  /** Report the live segment-length measurement from the last point to the cursor. */
  private _reportMeasurement(last: V3, cursor: V3): void {
    if (this.typed !== '') {
      this.onMeasurementCb(this._typedReadout())
      return
    }
    this.onMeasurementCb(formatLength(segmentLength(last, cursor)))
  }

  /**
   * Commit a segment of the exact typed length from the last placed point,
   * along normalize(cursor - last) (falling back to the last rubber-band
   * direction if the cursor hasn't moved off the anchor yet). Ignored if no
   * direction is available at all.
   */
  private _commitTyped(distance: number): void {
    if (this.planeStage.kind === 'anchored') {
      const { plane, target, anchor } = this.planeStage
      const cursor = this._lastPlaneCursor ?? anchor
      const dir = directionBetween(anchor, cursor)
      if (dir === null) return
      const endpoint = pointAlong(anchor, dir, distance)
      this._commitPlaneSegment(plane, target, anchor, endpoint)
    } else if (this.faceStage.kind === 'anchored') {
      const { points } = this.faceStage
      const last = points[points.length - 1]
      const cursor = this._lastFaceCursor ?? last
      const dir = directionBetween(last, cursor)
      if (dir === null) return
      const endpoint = pointAlong(last, dir, distance)
      this._appendFacePoint(endpoint)
    }
  }

  // ------------------------------------------------------------------ plane mode

  private _onPointerDownPlane(snap: Snap | null, ray: Ray): void {
    if (this.planeStage.kind === 'idle') {
      // First click: resolve (and freeze) the plane/target, then set anchor
      // — no segment to commit yet.
      const resolved = this._resolveClickTarget(snap, ray)
      if (resolved === null) return
      const { plane, target } = resolved
      const anchor = this._planeCursor(snap, ray, plane)
      if (anchor === null) return

      this.planeStage = { kind: 'anchored', plane, target, anchor }
      this._lastPlaneCursor = null
      this.typed = ''
      this.onMeasurementCb('')
      this._publishTransient(null)
    } else {
      const { plane, target, anchor } = this.planeStage
      const cursor = this._planeCursor(snap, ray, plane)
      if (cursor === null) return
      this._commitPlaneSegment(plane, target, anchor, cursor)
    }
  }

  /** Commit one segment anchor -> cursor, then chain forward from cursor. */
  private _commitPlaneSegment(plane: DrawPlane, target: SketchTarget, anchor: V3, cursor: V3): void {
    // Skip degenerate zero-length segments. The ground branch keeps the
    // EXACT legacy per-axis check (independent x/y thresholds, ignoring z
    // since it's always 0) — bit-identical gating to before this module
    // existed; non-ground uses the Euclidean check face mode already uses.
    if (plane.ground) {
      if (Math.abs(anchor[0] - cursor[0]) < 1e-8 && Math.abs(anchor[1] - cursor[1]) < 1e-8) return
    } else if (segmentLength(anchor, cursor) < 1e-8) {
      return
    }

    try {
      // Each committed segment is its own gesture — one Cmd+Z undoes exactly
      // that segment, matching LineTool's chain-forward-per-click semantics.
      runSketchGesture(this.wasmScene, this.sketchCache, target, (sketch) => {
        const report = this.wasmScene.sketch_add_segment(
          sketch,
          anchor[0], anchor[1], anchor[2],
          cursor[0], cursor[1], cursor[2],
        )
        let closed: boolean
        try {
          closed = report.regions_created().length > 0
        } finally {
          report.free()
        }

        this.onCommit(sketch)

        if (closed) {
          // The loop closed into a face — end the chain (idle, ready for a new line).
          this.planeStage = { kind: 'idle' }
          this.typed = ''
          this._lastPlaneCursor = null
          this.lockAxis = null
          this.shiftAxisLock = false
          this._clearPreview()
          this.onMeasurementCb('')
          this.wasmScene.clear_transient_segments()
        } else {
          // Chain forward: the new point becomes the anchor for the next
          // segment, on the SAME frozen plane/target.
          this.planeStage = { kind: 'anchored', plane, target, anchor: cursor }
          this._lastPlaneCursor = null
          this.typed = ''
          this._clearPreview()
          this.onMeasurementCb('')
          this._publishTransient(null)
        }
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
      if (snap === null) return

      const eligible = this._eligiblePickFor(ray)
      if (eligible === null) return

      const { object: objectHandle, face: faceHandle } = eligible
      const normalArr = this.wasmScene.face_normal(objectHandle, faceHandle)
      const normal: V3 = [normalArr[0], normalArr[1], normalArr[2]]
      const anchor: V3 = [snap.x, snap.y, snap.z]

      this.faceStage = {
        kind: 'anchored',
        object: objectHandle,
        face: faceHandle,
        normal,
        planePoint: anchor,
        points: [anchor],
      }
      this._lastFaceCursor = null
      this.typed = ''
      this.onMeasurementCb('')
      this._publishTransient(null)
    } else {
      const cursor = this._faceCursor(snap, ray)
      if (cursor === null) return
      this._appendFacePoint(cursor)
    }
  }

  /** Append a point to the face-mode path, skipping a degenerate (zero-length) segment. */
  private _appendFacePoint(point: V3): void {
    if (this.faceStage.kind !== 'anchored') return
    const { points } = this.faceStage
    const last = points[points.length - 1]
    if (segmentLength(last, point) < 1e-8) return

    points.push(point)
    this._lastFaceCursor = null
    this.typed = ''
    this._clearPreview()
    this.onMeasurementCb('')
    this._publishTransient(null)
  }

  /** Cut `face` along the accumulated path (boundary-to-boundary). */
  private _commitFacePath(object: bigint, face: bigint, points: V3[]): void {
    const path = new Float64Array(points.length * 3)
    for (let i = 0; i < points.length; i++) {
      path[i * 3 + 0] = points[i][0]
      path[i * 3 + 1] = points[i][1]
      path[i * 3 + 2] = points[i][2]
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

  // ------------------------------------------------------------------ preview

  /**
   * Emit a LineSegments preview for a single segment. Endpoints are used
   * exactly as given — the preview's depth bias (PREVIEW_LINE_STYLE,
   * depthPolicy.ts) settles coincidence with the ground/committed lines, so
   * no z-lift.
   */
  private _drawRubberBandSegment(a: V3, b: V3): void {
    const pts = new Float32Array([...a, ...b])
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
