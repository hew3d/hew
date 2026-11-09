/**
 * CircleTool — two-click faceted-circle (regular N-gon) sketching.
 *
 * A "circle" here is a faceted regular N-gon (true arcs are out of scope —
 * see). It decomposes into N chained `draft_add_segment` calls inside a
 * circle bracket, committed as one draft (plane and face mode alike) —
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
 *   3. Second click: commit — N draft_add_segment calls chaining
 *      vertex[i] -> vertex[i+1], last one vertex[N-1] -> vertex[0] (using
 *      the SAME stored vertex[0] coords for exact closure), in
 *      one `runDraft`
 *   4. Esc between clicks: cancel stage 1
 *   Calls onCommit() after each successful commit so the viewport can
 *   refresh scene geometry and trigger re-render.
 *
 * Face mode (an eligible Object face is under the cursor):
 *   1. First click on an eligible face: anchor center (on face plane)
 *   2. Move: rubber-band N-gon preview projected onto that face plane
 *   3. Second click: commit — one `runDraft` on that face
 *      with N loop segments
 *   4. Esc: cancel
 *   Calls onFaceImprint(objectId) after each successful imprint so the
 *   viewport can refresh the scene.
 *
 * VCB: the radius is a SINGLE length (unlike Rectangle's W x D), so typed
 * entry mirrors LineTool's single-length VCB style (editLengthBuffer /
 * parseLengthToMeters), not Rectangle's editDimsBuffer/parseDimensions.
 */

import { t } from '@lingui/core/macro'
import * as THREE from 'three'
import type { Tool, Snap, SnapConstraint, EditContext } from './types'
import { editContextEq } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { V3 } from '../viewport/geoHelpers'
import { circlePolygonGround, circlePolygonFace, facePlaneBasis } from '../viewport/geoHelpers'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { makeFatSegments, disposeFatSegments } from '../viewport/fatLine'
import { displayedLength, isFreePick } from './displayedValue'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { editLengthBuffer, isLengthInputKey, nextIdlePlaneLock, axisLockColorName, radiusReadout } from './moveInput'
import { RetypeWindow, idleRetypeCapturesKey, retypeStaleMessage } from './retypeWindow'
import { segmentLength } from './lineInput'
import { DetachedMode, type OnDetachedChange } from './detachedMode'
import type { SketchTarget } from './sketchGesture'
import { runDraft, addDraftLoop, refreshAfterDraft, draftTargetFromSketchTarget } from './draftCommit'
import { cameraView } from './viewPlaneInference'
import { drawPlaneCue, idleHoverCue, drawPlaneThrough, isGroundPlane, SketchPickCache, resolveIdleDrawTarget, resolveClickDrawTarget, nextGestureLockPlane, groundNaturalTarget, type DrawPlane, type IdleDrawTarget, type CameraView } from './drawPlane'
import { getDrawingAxes } from './drawingAxes'
import { ineligibleFacePlane, FacePickCache, defaultFaceEligible, worldFaceNormal, worldFacePlane, snapOnObjectBoundary, type FaceEligible, type FaceThroughPick, cursorOnFrozenPlane, firstPointOnFace } from './faceDraw'
import { pickRenderMode } from './pickPolicy'
import { PlanePin } from './planePin'

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

/** The toast for a face circle the face could not hold whole: part of it
 *  leaves a sliver too thin for a face, so that part stays as drawing. (A
 *  circle touching the face's edge or another shape at one point is a face
 *  and never reaches this.) */
export const circleKeptAsDrawingMessage = (): string =>
  t`Part of the circle is too thin to make a face, so it stays as drawing. Move it a little so it sits clear, or draw it larger.`

export type OnMeasurement = (text: string) => void

/** Plane stage: waiting for first click, or waiting for second click, on a
 *  frozen `DrawPlane`/`SketchTarget`. `natural` (design §2a) is the plane/
 *  target this gesture would have anchored onto at its own first click had
 *  no idle lock been active — what a mid-gesture arrow-key lock reverts to
 *  when toggled back off (see `nextGestureLockPlane` in drawPlane.ts). */
type PlaneStage =
  | { kind: 'idle' }
  | {
      kind: 'anchored'
      plane: DrawPlane
      target: SketchTarget
      center: V3
      natural: { plane: DrawPlane; target: SketchTarget }
      /** Set when an arrow lock moved a FACE-anchored gesture onto this
       *  plane (GitHub #51): releasing the lock goes back to that face
       *  instead of to `natural`. */
      fromFace?: AnchoredFaceStage
    }

/** Face stage: idle, or anchored on a specific face plane */
type AnchoredFaceStage = {
  kind: 'anchored'
  object: bigint
  face: bigint
  normal: V3
  /** A world-space point that lies on the face plane (the first click position) */
  planePoint: V3
  center: V3
}
type FaceStage = { kind: 'idle' } | AnchoredFaceStage


/**
 * The just-committed circle, kept in a `RetypeWindow` (retypeWindow.ts) so a
 * radius typed AFTER the rim click resizes it in place — `center` stays,
 * `rim` fixes the direction the radius is measured along (and so the
 * facet phase), the same idiom Rectangle's post-click `W,D` follows.
 */
type RetypeSpec = {
  center: V3
  rim: V3
  /** Detached mode the circle was drawn with — a retyped radius re-commits
   *  with the same mode (separate-shapes design §6.1). */
  detached: boolean
} & (
  | { mode: 'plane'; plane: DrawPlane; target: SketchTarget }
  | { mode: 'face'; object: bigint; face: bigint; normal: V3 }
)

export class CircleTool implements Tool {
  /** Idle over empty space, the plane follows the view (`Tool.followsViewPlane`). */
  readonly followsViewPlane = true
  readonly name = 'Circle'
  /** Click-drag-release works as well as click-click (Tool.dragCommits). */
  readonly dragCommits = true

  /** Live camera feed (Tool.updateCamera): keeps the view direction the idle
   *  picker infers the drawing plane from. */
  updateCamera(camera: THREE.Camera): void {
    this._view = cameraView(camera)
  }

  /** The pointer left the canvas (Tool.onPointerLeave): nothing is hovered,
   *  so the idle hover's plane patch, its "Facing" hint line and its projected verdict go with it. A
   *  lock, a pin and an anchored gesture are choices, not hover, and stay;
   *  the next move (or a held-pointer replay) rebuilds the hover. */
  onPointerLeave(): void {
    this._idleHoverCue = null
    this._idleInferred = null
    this._idleProjected = false
  }

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    const idle = this.planeStage.kind === 'idle' && this.faceStage.kind === 'idle' && !this.retype.isOpen
    return this.sep.hint(this._stageHint(), idle)
  }

  private _stageHint(): string {
    if (!this.capturingInput() && this.idlePlaneLock === null && this.pin.current === null && this.retype.isOpen) {
      return t`Type an exact radius to resize the circle you just drew — or click the centre of the next one.`
    }
    if (this.planeStage.kind !== 'idle' || this.faceStage.kind !== 'idle') {
      return t`Click to set the radius — or type an exact radius.`
    }
    if (this.idlePlaneLock !== null) {
      const axis = axisLockColorName(this.idlePlaneLock)
      return t`Locked to the ${axis} plane — click to start; same arrow or Esc unlocks.`
    }
    if (this.pin.current !== null) {
      return t`Pinned to the hovered plane — click to start; Shift again or Esc unpins.`
    }
    if (this._idleInferred !== null && this._idleInferred.axis !== 2) {
      const axis = axisLockColorName(this._idleInferred.axis)
      return t`Facing the ${axis} plane — click the center to draw on it; ↑ for the ground, Shift pins it.`
    }
    return t`Click the circle's center — on the ground plane or any face or sketch.`
  }

  private planeStage: PlaneStage = { kind: 'idle' }
  private faceStage: FaceStage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnCircleCommit
  private onFaceImprint: OnFaceImprint
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement
  /** Detached mode (Ctrl/Cmd tap): the next circle joins nothing. */
  private readonly sep: DetachedMode

  /** The current editing context (component-edit-parity.md phase A1) — see
   *  LineTool's identical fields for the full rationale. */
  private _editContext: EditContext = { kind: 'top' }

  /** The entered OBJECT id, or null — unchanged meaning from the old
   *  `_activeContext` field. */
  private get _activeContext(): bigint | null {
    return this._editContext.kind === 'object' ? this._editContext.id : null
  }

  /** The entered component INSTANCE id, or null (component-edit-parity.md
   *  phase A2). */
  private get _activeInstance(): bigint | null {
    return this._editContext.kind === 'instance' ? this._editContext.id : null
  }

  /** VCB buffer — raw string being typed by the user (radius, in display
   *  units). While IDLE with the retype window open it is the post-click
   *  radius being typed for the circle just drawn. */
  private typed: string = ''

  /** The just-committed circle, resizable by a typed radius until the next
   *  pointer action, Escape, tool switch, or any other document mutation
   *  (see `RetypeSpec` and retypeWindow.ts). */
  private readonly retype: RetypeWindow<RetypeSpec>

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
   *  `onDocumentReset()`/`setEditContext()` already route through). */
  private idlePlaneLock: 0 | 1 | 2 | null = null

  /** Shift-pinned drawing plane (GitHub issue 14) — see planePin.ts. Idle
   *  hovers feed it the plane the next click would land on; a Shift press
   *  pins that plane for the whole gesture (and the next, until released).
   *  Mutually exclusive with `idlePlaneLock`: whichever is set last wins. */
  private readonly pin = new PlanePin()

  /** The last hover point seen while idle-locked (design §6 bullet 1) — feeds
   *  `activeDrawPlaneCue()`'s idle-locked case. Reset to null whenever the
   *  lock itself changes (a fresh lock has no hover yet) and by `cancel()`. */
  private _lastIdleHoverPoint: V3 | null = null

  /** The camera's forward direction and eye, refreshed every frame via
   *  `updateCamera` (view-plane inference: the idle picker infers the axis
   *  plane that faces the camera from it). `null` until the first frame, so a
   *  caller that never feeds a camera keeps the ground fallback. */
  private _view: CameraView | null = null
  /** `_view` as it stood at the gesture's first press, so a mid-gesture
   *  arrow release reverts to the plane an unlocked first click would have
   *  chosen even if the camera has orbited since. */
  private _gestureView: CameraView | null = null
  /** What the idle picker inferred from the view at the last hover (step 7:
   *  the axis plane facing the camera) with the ray it was resolved for, or
   *  null when the ground, a face, a sketch, a lock or a pin decided
   *  instead. Drives, for a vertical plane, the inference line of the
   *  status hint (the patch reads `_idleHoverCue`). */
  private _idleInferred: { axis: 0 | 1 | 2; plane: DrawPlane; ray: Ray } | null = null
  /** The plane the next idle click would land on at the last hover, with
   *  the ray it was resolved for, whenever no lock or pin decided instead
   *  (`idleHoverCue`): the inferred plane, a hovered sketch's, or a hovered
   *  or leaned-toward face's. Drives the idle plane patch only; the status
   *  hint's inference line keys on `_idleInferred`. */
  private _idleHoverCue: { plane: DrawPlane; ray: Ray } | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnCircleCommit,
    onToast: OnToast,
    onFaceImprint: OnFaceImprint,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
    onDetachedChange: OnDetachedChange = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onCommit = onCommit
    this.onFaceImprint = onFaceImprint
    this.onToast = onToast
    this.sep = new DetachedMode(onMeasurement, onDetachedChange)
    this.onMeasurementCb = this.sep.report
    this.retype = new RetypeWindow(wasmScene)
  }

  /** The single editing-context channel (component-edit-parity.md phase A1;
   *  replaces `setActiveContext`). */
  setEditContext(ctx: EditContext): void {
    if (editContextEq(ctx, this._editContext)) return
    this._editContext = ctx
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
   * The eligible face a click at `snap` means, with its world plane. For a
   * snap ON an object's edge or corner (`snapOnObjectBoundary`) the plain
   * pick under the ray is a coin toss between the faces meeting there — or a
   * miss, which used to send the whole gesture to the GROUND plane (a
   * Circle started on a vertical edge's midpoint landed at z = 0) — so the
   * ranked boundary pick (`FacePickCache.faceThrough`) answers instead: the
   * most camera-facing eligible face whose plane holds the snapped point.
   * Any other snap keeps the plain pick, with the plane read from it.
   */
  private _facePickAt(snap: Snap | null, ray: Ray): FaceThroughPick | null {
    if (snapOnObjectBoundary(snap)) {
      return this._pickCache.faceThrough(
        this.wasmScene, ray, (object, instance) => this._isEligible(object, instance),
        this._activeInstance, [[snap.x, snap.y, snap.z]],
      )
    }
    const eligible = this._eligiblePickFor(ray)
    if (eligible === null) return null
    const normal = worldFaceNormal(this.wasmScene, eligible.object, eligible.face, this._activeInstance)
    if (normal === null) return null
    return { ...eligible, normal, point: snap !== null ? [snap.x, snap.y, snap.z] : [0, 0, 0] }
  }

  /**
   * Resolve the plane/target an IDLE gesture would anchor onto at `ray`
   * (design §1/§4): a top-level `pick_sketch` hit whose plane is non-ground
   * adopts that sketch (SKETCH MODE); otherwise the ground plane (PLANE
   * MODE, today's behavior). Only reachable when `_currentMode` has already
   * ruled out face mode (which takes priority), so no `_activeContext`
   * re-check is needed here.
   */
  private _resolveIdleTarget(ray: Ray, snap: Snap | null = null): IdleDrawTarget {
    return resolveIdleDrawTarget(this.wasmScene, this._sketchPickCache, ray, this._editContext, () => ineligibleFacePlane(this.wasmScene, this._pickCache, ray, (o, i) => this._isEligible(o, i), snap), snapOnObjectBoundary(snap) ? `${snap.x},${snap.y},${snap.z}` : '', this._view)
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
  private _resolveClickTarget(snap: Snap | null, ray: Ray): IdleDrawTarget | null {
    const pinned = this.pin.clickTarget(this._editContext)
    if (pinned !== null) return pinned
    return resolveClickDrawTarget(
      this.wasmScene, this._sketchPickCache, this.idlePlaneLock, snap, ray, this._editContext,
      () => ineligibleFacePlane(this.wasmScene, this._pickCache, ray, (o, i) => this._isEligible(o, i), snap),
      snapOnObjectBoundary(snap) ? `${snap.x},${snap.y},${snap.z}` : '',
      this._view,
    )
  }

  /** The cursor's position on `plane`. On the ground plane this is EXACTLY
   *  `[snap.x, snap.y, 0]` (no basis math, snap required) — the legacy fast
   *  path, bit-identical to before this module existed. On any other plane:
   *  the snap (already plane-constrained via `snapConstraint`) if present,
   *  else ray∩plane. */
  private _planeCursor(snap: Snap | null, ray: Ray, plane: DrawPlane): V3 | null {
    if (plane.ground) {
      if (snap === null) return null
      // Records whether the z about to be discarded was actually carrying
      // information — see `snapProjected`.
      this._snapProjected = snap.z !== 0
      return [snap.x, snap.y, 0]
    }
    // A non-ground plane is frozen like a face: an off-plane snap (X-ray)
    // drops onto it, disclosed (`cursorOnFrozenPlane`).
    const at = cursorOnFrozenPlane(snap, ray, plane.origin, plane.normal)
    this._snapProjected = at?.projected ?? false
    return at?.point ?? null
  }

  /** See `Tool.snapProjected`. Set by `_planeCursor` and `_faceCursor`,
   *  where the drawing plane replaces an off-plane snap (the ground drops its
   *  height; a face or sketch plane drops it straight onto itself). */
  snapProjected(): boolean {
    // Only a gesture under way projects anything: once the stage is back to
    // idle (a commit, Esc, a context change) the last verdict must not ride
    // along on ordinary hover cues.
    // An idle hover reports what its click would do (`_idleProjected`).
    const active = this.faceStage.kind !== 'idle' || this.planeStage.kind !== 'idle'
    return active ? this._snapProjected : this._idleProjected
  }

  /** Whether the idle hover's first click would drop its snap onto the
   *  hovered face (`firstPointOnFace`); set by `_trackIdleHover`. */
  private _idleProjected = false

  /** Whether the last `_planeCursor` / `_faceCursor` moved the snap onto its plane. */
  private _snapProjected = false

  /**
   * Decide which mode governs the NEXT pointer event (same contract as the
   * other draw tools): sticky mid-gesture; always face mode inside an
   * entered object context (scoped drawing — no top-level plane sketch from
   * inside); else decided by what's under the cursor.
   */
  private _currentMode(ray?: Ray, snap: Snap | null = null): 'face' | 'plane' {
    if (this.faceStage.kind === 'anchored') return 'face'
    if (this.planeStage.kind === 'anchored') return 'plane'
    // Inside an entered object context, drawing stays scoped to that
    // object's faces — a click elsewhere is ignored by the face handler
    // rather than falling through to a top-level plane sketch.
    if (this._activeContext !== null) return 'face'
    // An active idle plane lock or Shift pin beats face pick and
    // sketch-hover adoption (design §5.2) — the user already chose a plane.
    if (this.idlePlaneLock !== null || this.pin.current !== null) return 'plane'
    if (ray === undefined) return 'plane'
    return this._facePickAt(snap, ray) !== null ? 'face' : 'plane'
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
   * - Idle over nothing with the view facing a vertical (or moved-frame)
   *   axis plane (picker step 7): `fallbackPlane` only — a miss lands on
   *   that plane, but the kernel is not constrained, so geometry anywhere
   *   still snaps.
   * - Otherwise (ground mode): return null (unconstrained).
   */
  snapConstraint(ray: Ray): SnapConstraint | null {
    if (this.faceStage.kind === 'anchored') {
      // Already anchored: lock to the established face plane
      return {
        constraintPlane: {
          point: this.faceStage.planePoint,
          normal: this.faceStage.normal,
        },
        // Dropped onto the plane and disclosed (`cursorOnFrozenPlane`), so
        // X-ray and Wireframe may offer hidden points through it.
        seeThroughPoints: true,
      }
    }

    if (this.planeStage.kind === 'anchored') {
      if (this.planeStage.plane.ground) return null
      return {
        constraintPlane: {
          point: this.planeStage.plane.origin,
          normal: this.planeStage.plane.normal,
        },
        // Dropped onto the plane and disclosed (`cursorOnFrozenPlane`), so
        // X-ray and Wireframe may offer hidden points through it.
        seeThroughPoints: true,
      }
    }

    // Idle plane lock (design §5.2): the first click is FREE — no
    // constraint plane. The locked plane is derived FROM that click
    // (`_resolveClickTarget`), so constraining the snap here would be
    // circular. A lock also beats face pick / sketch-hover adoption, so
    // neither of those runs below while one is active.
    if (this.idlePlaneLock !== null) return null

    // A Shift-pinned plane (planePin.ts) is a FIXED plane: the cursor is
    // held to it wherever it goes, so it constrains the snap outright.
    const pinned = this.pin.constraint()
    if (pinned !== null) return pinned

    // Over nothing, the axis plane the view faces (picker step 7) is only
    // where a miss lands: it is never sent to the kernel as a constraint, so
    // a corner or edge anywhere in the scene still snaps (and then decides
    // the plane itself, steps 3-6).
    const hover = this._idleHoverResolution(ray)
    // Inside an entered object only its own faces take a click (`_currentMode`
    // stays in face mode there, and the hover patch shows nothing else), so
    // a sketch or plane from outside it must not filter the snap either.
    if (this._activeContext !== null && hover.onFace !== true) return null
    if (hover.inferredAxis !== undefined) {
      return { fallbackPlane: { point: hover.plane.origin, normal: hover.plane.normal } }
    }
    const plane = hover.plane
    if (!plane.ground) {
      // Hidden points through the hovered face only in X-ray, where the
      // first click can adopt or be dropped onto that face; Wireframe has no
      // faces to adopt.
      return { constraintPlane: { point: plane.origin, normal: plane.normal }, seeThroughPoints: pickRenderMode() === 'xray' }
    }
    return null
  }

  /**
   * The plane an idle click at `ray` would land on — the hovered eligible
   * face's, a hovered non-ground sketch's, else the ground — as a
   * `DrawPlane`, and the plane a Shift press pins (`onPointerMove` records
   * it into `pin` with the hover point). Falls back to the ground plane
   * for a stale face (no world normal), matching `_currentMode`'s miss.
   */
  private _idleHoverPlane(ray: Ray, snap: Snap | null = null): DrawPlane {
    return this._idleHoverResolution(ray, snap).plane
  }

  /** `_idleHoverPlane` with how it was decided: `inferredAxis` is set
   *  only when nothing was under the cursor and the plane is the axis
   *  plane the view faces (picker step 7, `resolveIdleDrawTarget`);
   *  `onFace` only when a face decided it (steps 3, 5, 6). */
  private _idleHoverResolution(ray: Ray, snap: Snap | null = null): { plane: DrawPlane; inferredAxis?: 0 | 1 | 2; onFace?: true } {
    // Hovering an object's edge or corner: the plain ray pick misses there
    // (strict at the boundary), so the ranked boundary pick answers which
    // face the point belongs to — a Shift press over a wall's edge
    // midpoint pins that WALL, not the ground the miss fell through to.
    if (snapOnObjectBoundary(snap)) {
      const through = this._facePickAt(snap, ray)
      if (through !== null) {
        const drawPlane = drawPlaneThrough(through.point, through.normal)
        if (drawPlane !== null) return { plane: drawPlane, onFace: true }
      }
    }
    const eligible = this._eligiblePickFor(ray)
    if (eligible !== null) {
      const plane = worldFacePlane(this.wasmScene, eligible.object, eligible.face, this._activeInstance)
      if (plane !== null) {
        const drawPlane = drawPlaneThrough(plane.point, plane.normal)
        if (drawPlane !== null) return { plane: drawPlane, onFace: true }
      }
    }
    return this._resolveIdleTarget(ray, snap)
  }

  /**
   * The drawing-plane cue the Viewport should render right now (design §6
   * bullet 1) — a grid patch on the active NON-ground plane, or null (ground
   * is covered by the world grid already). See `drawPlaneCue` in
   * `drawPlane.ts` for the cases (anchored non-ground, idle-locked with a
   * tracked hover, idle over a non-ground plane the next click would land
   * on); a Shift pin draws through the hover like an anchored plane.
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
    const pinned = this.pin.current
    if (pinned !== null) {
      // The pinned plane through wherever the cursor is now (its foot on
      // the plane — the snap is constrained to it), so the patch follows
      // the hover exactly as the arrow-lock preview does.
      return drawPlaneCue({
        anchoredPlane: pinned.plane,
        anchoredThrough: this._lastIdleHoverPoint ?? pinned.through,
        idleLock: null,
        idleHover: null,
      })
    }
    return drawPlaneCue({
      anchoredPlane: null,
      anchoredThrough: null,
      idleLock: this.idlePlaneLock,
      idleHover: this._lastIdleHoverPoint,
      frame: getDrawingAxes(this.wasmScene),
      idleHoverCue: this._idleHoverCue,
    })
  }

  /**
   * Shift pressed/released while IDLE toggles the plane pin (planePin.ts;
   * GitHub issue 14 — "press Shift to lock the plane does nothing"): the
   * plane the last hover recorded becomes the drawing plane until Shift is
   * pressed again or Escape releases it. A pin displaces an arrow-key lock
   * (the two are different shapes of the same choice). Mid-gesture Shift
   * does nothing here — the circle's plane is frozen at its first click.
   */
  setShiftHeld(held: boolean): void {
    if (this.capturingInput()) {
      this.pin.markShift(held) // keep the autorepeat guard honest, never toggle
      return
    }
    if (this.pin.setShiftHeld(held)) {
      this.idlePlaneLock = null
      this._lastIdleHoverPoint = null
      this._idleInferred = null
      this._idleHoverCue = null
    }
  }

  /** What step 7 of the idle picker inferred at this hover, or null when
   *  anything else decided: a face, an edge or corner, a sketch, or the
   *  plain ground. */
  private _inferredAt(ray: Ray, snap: Snap | null): { axis: 0 | 1 | 2; plane: DrawPlane; ray: Ray } | null {
    if (this._view === null || this._currentMode(ray, snap) === 'face') return null
    const resolved = this._resolveIdleTarget(ray, snap)
    return resolved.inferredAxis === undefined ? null : { axis: resolved.inferredAxis, plane: resolved.plane, ray }
  }

  /** Idle hover bookkeeping: the hover point for the plane cue (arrow lock
   *  and Shift pin previews both draw through it) and, for the pin, the
   *  plane the next click would land on (`_idleHoverPlane`). */
  private _trackIdleHover(snap: Snap | null, ray: Ray): void {
    this._snapProjected = false
    this._idleProjected = false
    this._idleInferred = null
    this._idleHoverCue = null
    // A lock or pin skips every face pick below (it beats face mode), yet
    // the boundary pick's hover memory (`FacePickCache`, view-plane design
    // §4.2 rule 2) must keep following the cursor, or a release at a corner
    // would remember the face it was over before the lock. The pick itself
    // is discarded; a boundary snap never feeds the memory.
    if ((this.idlePlaneLock !== null || this.pin.current !== null) && !snapOnObjectBoundary(snap)) this._eligiblePickFor(ray)
    if (snap === null) return
    if (this._currentMode(ray, snap) === 'face') {
      const face = this._facePickAt(snap, ray)
      if (face !== null) this._idleProjected = firstPointOnFace(this.wasmScene, face, this._activeInstance, snap, ray).projected
    }
    const through: V3 = [snap.x, snap.y, snap.z]
    this._lastIdleHoverPoint = through
    if (this.idlePlaneLock === null) this.pin.trackHover(this._idleHoverPlane(ray, snap), through)
    this._idleInferred = this.idlePlaneLock === null && this.pin.current === null ? this._inferredAt(ray, snap) : null
    this._idleHoverCue = this.idlePlaneLock === null && this.pin.current === null ? idleHoverCue(this._idleHoverResolution(ray, snap), ray, this._activeContext !== null) : null
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (this._currentMode(ray, snap) === 'face') {
      // Face mode
      if (this.faceStage.kind !== 'anchored') {
        this._trackIdleHover(snap, ray)
        this._clearPreview()
        // An open retype buffer owns the readout: the key router re-runs
        // this hover after every captured key and must not wipe it.
        if (this.typed === '') this.onMeasurementCb('')
        return
      }
      const { center, normal } = this.faceStage
      const cursorOnPlane = this._faceCursor(snap, ray)
      if (cursorOnPlane === null) {
        this._clearPreview()
        // An open retype buffer owns the readout: the key router re-runs
        // this hover after every captured key and must not wipe it.
        if (this.typed === '') this.onMeasurementCb('')
        return
      }
      this._lastFaceCursor = cursorOnPlane
      const verts = circlePolygonFace(center, cursorOnPlane, normal, faceSegments(center, cursorOnPlane))
      if (verts !== null) {
        this._drawRubberBandFace(verts, true)
        this._reportMeasurement(center, cursorOnPlane)
      } else {
        this._clearPreview()
        if (this.typed === '') this.onMeasurementCb('')
      }
    } else {
      // Plane mode
      if (this.planeStage.kind !== 'anchored') {
        this._trackIdleHover(snap, ray)
        this._clearPreview()
        // An open retype buffer owns the readout: the key router re-runs
        // this hover after every captured key and must not wipe it.
        if (this.typed === '') this.onMeasurementCb('')
        return
      }
      const { plane, center } = this.planeStage
      const cursor = this._planeCursor(snap, ray, plane)
      if (cursor === null) {
        this._clearPreview()
        // An open retype buffer owns the readout: the key router re-runs
        // this hover after every captured key and must not wipe it.
        if (this.typed === '') this.onMeasurementCb('')
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
          this._drawRubberBandFace(verts, false)
          this._reportMeasurement(center, cursor)
        } else {
          this._clearPreview()
          if (this.typed === '') this.onMeasurementCb('')
        }
      }
    }
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    // Any pointer action ends the retype window — the next circle has begun.
    this.disarmRetype()
    if (this.planeStage.kind === 'idle' && this.faceStage.kind === 'idle') this._gestureView = this._view
    if (this._currentMode(ray, snap) === 'face') {
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

  /**
   * True while a gesture is anchored OR an idle plane lock is armed — Escape
   * has tool-local work to do (clear the lock, or step the gesture back)
   * before a context-pop is appropriate (component-edit-parity.md phase A2;
   * see `toolHasArmedGesture` in tools/types.ts). `capturingInput()` alone
   * misses the idle-locked case: locked-but-idle is not "capturing input"
   * but IS armed for Escape's purposes.
   */
  hasArmedGesture(): boolean {
    return this.capturingInput() || this.idlePlaneLock !== null || this.pin.current !== null || this.typed !== ''
  }

  /**
   * Per-key refinement of the capture (see Tool.capturesKey): an anchored
   * gesture keeps the whole keyboard, exactly as before; the IDLE retype
   * window takes only what a typed radius needs (`idleRetypeCapturesKey`).
   */
  capturesKey(key: string): boolean {
    if (this.capturingInput()) return true
    if (!this.retype.isOpen) return false
    return idleRetypeCapturesKey(key, this.typed, isLengthInputKey)
  }

  /** Quietly close the retype window — the host calls this before an
   *  explicit undo/redo/delete (`disarmActivePostCommitWindow`). */
  disarmRetype(): void {
    this.retype.close()
    if (!this.capturingInput() && this.typed !== '') {
      this.typed = ''
      this.onMeasurementCb('')
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      // Idle with an active plane lock: Escape clears the lock FIRST — only
      // a second Escape (already idle, unlocked) falls through to today's
      // idle-Escape behavior (design §5.2).
      if (!this.capturingInput() && this.typed !== '' && this.retype.isOpen) {
        this.disarmRetype()
        return
      }
      if (!this.capturingInput()) this.retype.close()
      if (!this.capturingInput() && (this.idlePlaneLock !== null || this.pin.current !== null)) {
        this.idlePlaneLock = null
        this.pin.clear()
        this._lastIdleHoverPoint = null
        this._idleInferred = null
        this._idleHoverCue = null
        return
      }
      // Aborting an in-progress gesture keeps the plane lock: the lock is
      // an idle aiming choice, cleared only by an idle Escape or toggle
      // (parity across all four draw tools — LineTool's _endChain path).
      const lock = this.idlePlaneLock
      const pinned = this.pin.current
      this.cancel()
      this.idlePlaneLock = lock
      this.pin.restore(pinned)
      return
    }

    if (!this.capturingInput()) {
      // Idle retype window (see `RetypeSpec`): the keys `capturesKey` admits
      // edit the buffer; Enter resizes the just-committed circle.
      if (this.retype.isOpen && this.capturesKey(ev.key)) {
        if (ev.key === 'Enter') {
          const meters = parseLengthToMeters(this.typed)
          this.typed = ''
          this.onMeasurementCb('')
          if (meters !== null) this._retypeRadius(Math.abs(meters))
          return
        }
        this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
        this.onMeasurementCb(this.typed === '' ? '' : this._typedReadout())
        return
      }
      // Idle plane lock via arrow keys (design §5.2) — consumed by neither
      // hover nor preview, only by the next first click.
      if (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft' || ev.key === 'ArrowUp' || ev.key === 'ArrowDown') {
        // Keydown autorepeat would flip the lock on every repeat (a held
        // arrow alternating plane and face); only the first press counts.
        if (ev.repeat) return
        this.idlePlaneLock = nextIdlePlaneLock(this.idlePlaneLock, ev.key)
        this.pin.clear() // an arrow lock displaces a Shift pin, and vice versa
        // A fresh/changed lock has no tracked hover yet (design §6 bullet 1).
        this._lastIdleHoverPoint = null
        this._idleInferred = null
        this._idleHoverCue = null
      }
      return
    }

    // Mid-gesture plane re-lock (design §2a): once the center is placed, an
    // arrow key still re-locks the plane — through that ALREADY-PLACED
    // anchor, not the cursor — since nothing has reached the kernel yet (the
    // circle commits only on the second click).
    //
    // A FACE-anchored center takes the lock too (GitHub #51 — see
    // RectangleTool's identical block): the gesture leaves the face for the
    // locked plane through the center, and releasing the lock puts it back
    // on that face. Inside an entered object the face holds.
    if (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft' || ev.key === 'ArrowUp' || ev.key === 'ArrowDown') {
      // Keydown autorepeat would flip the lock on every repeat (a held
      // arrow alternating plane and face); only the first press counts.
      if (ev.repeat) return
      if (this.faceStage.kind === 'anchored') {
        if (this._activeContext !== null) return
        const fromFace = this.faceStage
        const { center } = fromFace
        const natural = groundNaturalTarget(this._editContext, center, this._gestureView, getDrawingAxes(this.wasmScene))
        const next = nextGestureLockPlane(null, ev.key, center, natural, getDrawingAxes(this.wasmScene), this._editContext)
        if (next.lock === null) return // Down with no lock: nothing to release
        this.idlePlaneLock = next.lock
        this.faceStage = { kind: 'idle' }
        this.planeStage = { kind: 'anchored', plane: next.plane, target: next.target, center, natural, fromFace }
        this._lastFaceCursor = null
        this._lastPlaneCursor = null
        this._clearPreview()
        this.onMeasurementCb('')
        return
      }
      if (this.planeStage.kind === 'anchored') {
        const { center, natural, fromFace } = this.planeStage
        const next = nextGestureLockPlane(
          this.idlePlaneLock, ev.key, center, natural, getDrawingAxes(this.wasmScene), this._editContext,
        )
        this.idlePlaneLock = next.lock
        if (next.lock === null && fromFace !== undefined) {
          this.planeStage = { kind: 'idle' }
          this.faceStage = fromFace
        } else {
          this.planeStage = { kind: 'anchored', plane: next.plane, target: next.target, center, natural, fromFace }
        }
        this._lastPlaneCursor = null
        this._lastFaceCursor = null
        this._clearPreview()
        this.onMeasurementCb('')
        return
      }
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
    this.retype.close()
    this.planeStage = { kind: 'idle' }
    this.faceStage = { kind: 'idle' }
    this.typed = ''
    this._lastPlaneCursor = null
    this._lastFaceCursor = null
    this.idlePlaneLock = null
    this.pin.clear()
    this._lastIdleHoverPoint = null
    this._idleInferred = null
    this._idleHoverCue = null
    this._clearPreview()
    this.onMeasurementCb('')
  }

  /**
   * A new/loaded document replaced the Scene, so every cached plane-mode
   * sketch handle is now stale (reusing one throws UnknownSketch). Drop them
   * all and reset. Called by the Viewport from `notifyLoaded`.
   */
  onDocumentReset(): void {
    this.cancel()
    this.sep.reset()
    // A new document's handles can reuse the old one's: no memoized pick
    // or hover memory may outlive it.
    this._pickCache.clear()
  }

  /** Ctrl/Cmd clean tap (Viewport-driven; a bare Ctrl never reaches onKey):
   *  flip Detached for the next circle (separate-shapes design §6.1). */
  toggleDetached(): void {
    this.sep.toggle()
  }

  /** Whether Detach is on. */
  get detachedOn(): boolean {
    return this.sep.on
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
    this.onMeasurementCb(radiusReadout(formatLength(radius)))
  }

  /**
   * Second click on a FREE pick (displayedValue.ts): commit the radius the
   * measurement box shows through the typed path, so the circle is exactly
   * what typing that radius would make. False (caller commits the raw
   * click) for a hard snap or a displayed radius of zero.
   */
  private _commitDisplayed(snap: Snap | null, center: V3, cursor: V3): boolean {
    if (!isFreePick(snap, center)) return false
    const r = displayedLength(Math.hypot(cursor[0] - center[0], cursor[1] - center[1], cursor[2] - center[2]))
    if (!(r > 0)) return false
    if (this.planeStage.kind === 'anchored') this._lastPlaneCursor = cursor
    else this._lastFaceCursor = cursor
    this._commitTyped(r)
    return true
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
      if (this._commitPlaneCircle(plane, target, center, rim)) {
        this.retype.arm({ mode: 'plane', plane, target, center, rim, detached: this.sep.on })
      }
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
      if (this._commitFaceCircle(object, face, center, rim, normal)) {
        this.retype.arm({ mode: 'face', object, face, normal, center, rim, detached: this.sep.on })
      }
    }
  }

  /**
   * Resize the just-committed circle to radius `r` through the shared
   * window: the new rim sits `r` along the committed center→rim direction,
   * so a retyped circle is indistinguishable from one clicked there.
   */
  private _retypeRadius(r: number): void {
    const spec = this.retype.spec
    if (spec === null) return
    const { center, rim } = spec
    const len = segmentLength(center, rim)
    if (len < 1e-9 || r < 1e-7) return
    const k = r / len
    const newRim: V3 = [
      center[0] + (rim[0] - center[0]) * k,
      center[1] + (rim[1] - center[1]) * k,
      center[2] + (rim[2] - center[2]) * k,
    ]
    const outcome = this.retype.apply(
      (hot) => this._commitHot(hot, newRim),
      (hot) => this._commitHot(hot, hot.rim),
      (hot) => ({ ...hot, rim: newRim }),
    )
    if (outcome === 'stale') this.onToast(retypeStaleMessage('circle'))
  }

  /** Lay a circle with `rim` down through the hot record's own commit path. */
  private _commitHot(hot: RetypeSpec, rim: V3): boolean {
    return hot.mode === 'plane'
      ? this._commitPlaneCircle(hot.plane, hot.target, hot.center, rim, hot.detached)
      : this._commitFaceCircle(hot.object, hot.face, hot.center, rim, hot.normal, hot.detached)
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

      // The unlocked resolution at this SAME click (design §2a) — reuse it
      // for free when no lock was active; fall back to ground WITHOUT
      // probing when one was (see RectangleTool's identical comment).
      // A Shift pin's first click resolves to the pinned plane itself, which
      // is exactly what a later arrow-lock release should revert to.
      const natural = this.idlePlaneLock !== null
        ? groundNaturalTarget(this._editContext, center, this._gestureView, getDrawingAxes(this.wasmScene))
        : resolved
      this.planeStage = { kind: 'anchored', plane, target, center, natural }
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

      // A free pick commits the DISPLAYED radius, exactly as if typed.
      if (this._commitDisplayed(snap, center, cursor)) return

      const committed = this._commitPlaneCircle(plane, target, center, cursor)
      this.planeStage = { kind: 'idle' }
      this.typed = ''
      this._lastPlaneCursor = null
      this._clearPreview()
      this.onMeasurementCb('')
      if (committed) this.retype.arm({ mode: 'plane', plane, target, center, rim: cursor, detached: this.sep.on })
    }
  }

  /** Commit a circle as ONE draft on `target`'s plane: the N facet segments
   *  chain inside a circle bracket (`draft_begin_curve_with`), so the whole
   *  circle is one selectable curve carrying its exact analytic
   *  center/radius (the true-curves design) wherever it lowers — face
   *  topology, an owned sketch, or a free sketch. Used by ground, non-ground
   *  plane and sketch mode (face mode: `_commitFaceVerts`). */
  private _commitPlaneCircle(plane: DrawPlane, target: SketchTarget, center: V3, rim: V3, detached = this.sep.on): boolean {
    const verts = plane.ground
      ? circlePolygonGround(
          [center[0], center[1]],
          [rim[0], rim[1]],
          groundSegments([center[0], center[1]], [rim[0], rim[1]]),
        )
      : circlePolygonFace(center, rim, plane.normal, faceSegments(center, rim))
    if (verts === null || verts.length === 0) return false // degenerate — ignore

    // World radius (a draft maps lengths through an instance pose itself).
    const radius = plane.ground
      ? Math.hypot(rim[0] - center[0], rim[1] - center[1])
      : segmentLength(center, rim)
    return this._runCircleDraft(draftTargetFromSketchTarget(target, detached, plane), verts, center, radius)
  }

  private _runCircleDraft(
    target: Parameters<typeof runDraft>[1],
    verts: V3[],
    center: V3,
    radius: number,
  ): boolean {
    try {
      const { result: regionsCreated, report } = runDraft(this.wasmScene, target, t`Circle`, (d) =>
        addDraftLoop(this.wasmScene, d, verts, { kind: 'circle', center, radius }),
      )
      refreshAfterDraft(report, {
        onObject: (o) => this.onFaceImprint(o),
        onSketch: (sketchHandle) => this.onCommit({ sketchHandle, regionsCreated }),
      })
      // An island the face could not hold (a sliver) stays as drawing; say
      // so now rather than leave the circle silently loose.
      if (target.kind === 'face' && report.keptAsSketch.length > 0) {
        this.onToast(circleKeptAsDrawingMessage())
      }
      return true
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      const message = kernelErrorMessage(code ?? 'Unknown', rawMsg)
      this.onToast(message, code ?? undefined)
      return false
    }
  }

  // ------------------------------------------------------------------ face mode

  private _onPointerDownFace(snap: Snap | null, ray: Ray): void {
    if (this.faceStage.kind === 'idle') {
      // First click: anchor on the eligible face under the cursor
      if (snap === null) return

      const eligible = this._facePickAt(snap, ray)
      if (eligible === null) return

      const { normal } = eligible
      // On the face's plane: a hidden point that is no part of the face
      // (X-ray) drops onto it, disclosed, like the later clicks.
      const first = firstPointOnFace(this.wasmScene, eligible, this._activeInstance, snap, ray)
      const center: V3 = first.point
      this._snapProjected = first.projected

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
      const { object, face, normal, center } = this.faceStage

      const cursorOnPlane = this._faceCursor(snap, ray)
      if (cursorOnPlane === null) return

      const verts = circlePolygonFace(center, cursorOnPlane, normal, faceSegments(center, cursorOnPlane))
      if (verts === null) return // degenerate — ignore
      if (this._commitDisplayed(snap, center, cursorOnPlane)) return

      this.faceStage = { kind: 'idle' }
      this.typed = ''
      this._lastFaceCursor = null
      this._clearPreview()
      this.onMeasurementCb('')

      if (this._commitFaceVerts(object, face, verts, center)) {
        this.retype.arm({ mode: 'face', object, face, normal, center, rim: cursorOnPlane, detached: this.sep.on })
      }
    }
  }

  /**
   * The cursor's position on the anchored face plane: the SNAPPED point when
   * one is available — `snapConstraint` already holds every face-mode snap
   * to this plane, so an `Endpoint`/`Midpoint`/`On Edge` chip is honoured
   * exactly, the rim landing ON the snapped point rather than a sub-pixel
   * off it — else the raw ray∩plane intersection (nothing snapped: past the
   * face's extent, or no kernel candidate). A snap that somehow sits off the
   * plane (never, by the constraint's construction — a defensive check
   * only) falls back to the ray too, so the imprint can never be handed an
   * off-plane rim. Before this, the rubber band and the commit both used
   * ray∩plane unconditionally while the chip claimed a snap the circle did
   * not take.
   */
  private _faceCursor(snap: Snap | null, ray: Ray): V3 | null {
    if (this.faceStage.kind !== 'anchored') return null
    const { planePoint, normal } = this.faceStage
    const at = cursorOnFrozenPlane(snap, ray, planePoint, normal)
    this._snapProjected = at?.projected ?? false
    return at?.point ?? null
  }

  /** Split the given face with a circle loop defined by center/rim/normal. */
  private _commitFaceCircle(object: bigint, face: bigint, center: V3, rim: V3, normal: V3, detached = this.sep.on): boolean {
    const verts = circlePolygonFace(center, rim, normal, faceSegments(center, rim))
    if (verts === null) return false // degenerate — ignore
    return this._commitFaceVerts(object, face, verts, center, detached)
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
  private _commitFaceVerts(object: bigint, face: bigint, verts: V3[], center: V3, detached = this.sep.on): boolean {
    const radius = Math.hypot(
      verts[0][0] - center[0],
      verts[0][1] - center[1],
      verts[0][2] - center[2],
    )
    return this._runCircleDraft(
      { kind: 'face', object, face, instance: this._activeInstance, ...(detached ? { detached: true as const } : {}) },
      verts,
      center,
      radius,
    )
  }

  // ------------------------------------------------------------------ preview

  /**
   * Draw a rubber-band N-gon from ground-plane vertices.
   * Used in ground (plane-mode) drawing.
   */
  private _drawRubberBandGround(verts: V3[]): void {
    this._clearPreview()
    this._drawRubberBandVerts(verts, false)
  }

  /**
   * Draw a rubber-band N-gon from explicit 3D vertices.
   * Used in face mode and non-ground plane mode — vertices already lie on
   * the target plane.
   */
  private _drawRubberBandFace(verts: V3[], joinsFace: boolean): void {
    this._clearPreview()
    this._drawRubberBandVerts(verts, joinsFace)
  }

  /**
   * Emit a LineSegments preview for a closed N-vertex loop. Vertices are used
   * exactly as given — the preview's depth bias (PREVIEW_LINE_STYLE,
   * depthPolicy.ts) settles coincidence with the ground/committed lines, so
   * no z-lift.
   *
   * @param verts  N world-space xyz vertices in order.
   */
  private _drawRubberBandVerts(verts: V3[], joinsFace: boolean): void {
    const n = verts.length
    const pts = new Float32Array(n * 2 * 3)
    for (let i = 0; i < n; i++) {
      const a = verts[i]
      const b = verts[(i + 1) % n]
      const base = i * 6
      pts[base + 0] = a[0]; pts[base + 1] = a[1]; pts[base + 2] = a[2]
      pts[base + 3] = b[0]; pts[base + 4] = b[1]; pts[base + 5] = b[2]
    }
    this.preview.add(makeFatSegments(pts, this.sep.previewStyle(joinsFace)))
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
