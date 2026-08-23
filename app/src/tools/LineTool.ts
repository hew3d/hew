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
 *   On the ground plane with no active direction lock, every point is
 *   `[snap.x, snap.y, 0]` EXACTLY as before this generalization (no basis
 *   math) — bit-identical committed coordinates. WITH an active lock
 *   (`lockAxis`), the ground plane's cursor is the full 3D snap instead — a
 *   locked point (an up-arrow Z lock from a ground anchor, the canonical
 *   case) can genuinely leave z = 0, and `_commitPlaneSegment` re-homes the
 *   chain onto a real plane through the anchor when that happens (§2b),
 *   exactly as it already does for a lock leaving any other frozen plane —
 *   see `_planeCursor`'s doc. On a non-ground plane, the cursor is the snap
 *   (already plane-constrained via `snapConstraint`) or, absent one,
 *   ray∩plane.
 *
 *   1. First click: anchor the first point.
 *   2. Each subsequent click commits ONE segment from the previous point to
 *      the new point via `sketch_add_segment`. The new point becomes the
 *      anchor for the next segment (chain forward).
 *   3. Rubber-band preview: a single line from the last placed point to the
 *      live cursor.
 *   4. Closing the loop: when a commit's regions_created() is non-empty, a
 *      face formed — call onCommit so the viewport refreshes, then end the
 *      chain (back to idle, ready to start a new line). A chain that
 *      re-homed (§2b) onto a different sketch and then visually returns to
 *      an earlier sketch's vertex does NOT close this way — welding and
 *      region detection are strictly per-sketch in the kernel — so that
 *      case is instead caught and surfaced honestly via a toast rather than
 *      silently accepted as an open (but visually closed-looking) chain.
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
import type { Tool, Snap, SnapConstraint, EditContext } from './types'
import { editContextEq } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { V3 } from '../viewport/geoHelpers'
import { rayPlaneIntersect, facePlaneBasis } from '../viewport/geoHelpers'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { makeFatSegments, disposeFatSegments, PREVIEW_LINE_STYLE, type FatSegmentsOpts } from '../viewport/fatLine'
import { axisColorForDirection, axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { arrowToAxis, editLengthBuffer, isLengthInputKey, pointAlong, nextIdlePlaneLock, AXIS_LOCK_COLOR_NAMES } from './moveInput'
import { segmentLength, directionBetween, rehomePlaneNormal } from './lineInput'
import { runSketchGesture, makeSketchPlaneCache, type SketchPlaneCache, type SketchTarget } from './sketchGesture'
import { pointOnPlane, drawPlaneCue, isGroundPlane, isPointOnDrawPlane, SketchPickCache, resolveIdleDrawTarget, resolveClickDrawTarget, type DrawPlane } from './drawPlane'
import { getDrawingAxes } from './drawingAxes'
import { FacePickCache, defaultFaceEligible, worldFaceNormal, type FaceEligible } from './faceDraw'

export type OnLineCommit = (sketchHandle: bigint) => void
export type OnFaceImprint = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/** Tolerance for colouring the rubber band by axis (tool-parity playtest2
 *  §2c) — a label/colour decision, not a snap decision (the kernel already
 *  decided whether/what to snap; this only decides how to PAINT the result).
 *  Matches `inferenceColor.ts`'s `AXIS_LABEL_TOL_DOT` (10°) so the preview
 *  line and the inference tooltip chip never disagree about which axis a
 *  direction reads as. */
const LINE_AXIS_PREVIEW_TOL_DOT = Math.cos((10 * Math.PI) / 180)

/** Preview line width while an EXPLICIT direction lock (`lockAxis`) holds —
 *  wider than the default `PREVIEW_LINE_STYLE.widthPx`, so a hard lock reads
 *  heavier on screen than the kernel's own soft-axis inference (tool-parity
 *  playtest2 §2c: "soft vs hard must be distinguishable" — SketchUp uses a
 *  bolder line for an explicit hold; this mirrors that). Soft inference
 *  keeps the default width and gets only the colour change, so the two are
 *  distinguishable at a glance without either one looking alarming. */
const HARD_LOCK_PREVIEW_WIDTH_PX = 4.0

/** Point-coincidence tolerance for the cross-sketch "spans two planes"
 *  honest-refusal check (`_commitPlaneSegment`, playtest-2 review finding
 *  B) — mirrors the kernel's own `tol::POINT_MERGE` (`find_or_plan_vertex`,
 *  kernel/src/sketch.rs), the exact distance within which the kernel would
 *  treat two points in the SAME sketch as one vertex. Using anything looser
 *  would flag ordinary near-misses that the kernel wouldn't consider
 *  coincident anyway; anything tighter could miss the exact case this
 *  exists to catch. */
const CHAIN_VERTEX_MERGE_EPS = 1e-9

/** Degenerate-segment threshold for the commit gates (`_commitPlaneSegment`,
 *  `_appendFacePoint`) — mirrors `kernel::tol::POINT_MERGE`, the EXACT
 *  threshold of the kernel's own refusal (`SketchError::DegenerateSegment`:
 *  endpoints within `POINT_MERGE` of each other after vertex merging,
 *  kernel/src/sketch.rs). These gates exist to PREDICT that refusal with a
 *  friendly toast instead of committing speculatively, so they must sit
 *  exactly on the kernel's line. Sitting looser (the legacy hardcoded 1e-8)
 *  refused segments the kernel itself accepts — and reopened the locked-click
 *  degeneracy as a nanometre band: the inference engine's lock-projection
 *  disqualification reads `tol::POINT_MERGE` directly, so a candidate whose
 *  projection landed in [POINT_MERGE, 1e-8) was returned as a real winner and
 *  then refused here. One threshold, defined by the kernel, read by all three
 *  layers, keeps them agreeing by construction.
 *
 *  Compared INCLUSIVELY (`<=`), because the kernel's own test is inclusive:
 *  `Point3::approx_eq` is `length_squared() <= tolerance * tolerance`. A strict
 *  `<` here would let a segment of length exactly `POINT_MERGE` past this gate
 *  and straight into the kernel's refusal, reporting the wrong message for a
 *  one-value band — agreeing on the magnitude but not on the boundary is not
 *  agreeing. */
const DEGENERATE_SEGMENT_EPS = 1e-9

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

  /** The current editing context (component-edit-parity.md phase A1) — a
   *  single value replacing the old `_activeContext`/`_activeComponent`
   *  duck-typed fields. `_activeContext`/`_activeInstance` below are
   *  read-only views derived from it, kept so the rest of this file's
   *  object-context logic (which predates this refactor) is untouched. */
  private _editContext: EditContext = { kind: 'top' }

  /** The entered OBJECT id, or null — unchanged meaning/behavior from the
   *  old `_activeContext` field. */
  private get _activeContext(): bigint | null {
    return this._editContext.kind === 'object' ? this._editContext.id : null
  }

  /** The entered component INSTANCE id, or null (component-edit-parity.md
   *  phase A2) — the new case this refactor adds. Plane-mode drawing routes
   *  through `begin_sketch_on_plane_in_instance`/def-owned sketches, and
   *  face-mode cuts route through `split_face_in_instance`, whenever this is
   *  set. */
  private get _activeInstance(): bigint | null {
    return this._editContext.kind === 'instance' ? this._editContext.id : null
  }

  /** VCB buffer — raw string being typed by the user (length, in display units) */
  private typed: string = ''

  /** Last rubber-band cursor positions, tracked for typed-entry direction */
  private _lastPlaneCursor: V3 | null = null
  private _lastFaceCursor: V3 | null = null


  /** Current axis lock: 0=X, 1=Y, 2=Z, null=free. Mirrors MoveTool. */
  private lockAxis: 0 | 1 | 2 | null = null
  /** True when the *current* axis lock was set by holding Shift (vs. an arrow). */
  private shiftAxisLock: boolean = false

  /** The direction (world-space, normalized) of the LAST committed segment
   *  of the current plane-mode chain, or null before any segment has been
   *  committed (the chain's first segment, or no chain in progress). Feeds
   *  `rehomePlaneNormal` (tool-parity playtest2 §2b) when a locked segment
   *  leaves the frozen plane — the new plane spans this direction and the
   *  new segment's, keeping an L-shaped chain coplanar. Reset to null
   *  whenever a chain starts fresh (`_onPointerDownPlane`'s idle branch)
   *  or ends (`_endChain`/`cancel`). */
  private _prevSegmentDir: V3 | null = null

  /** Every vertex committed so far in the current plane-mode chain, each
   *  tagged with the kernel sketch handle it actually landed in — spans
   *  re-homes (tool-parity playtest2 §2b), so a chain that re-homes across
   *  several planes/sketches still has its FULL history here, not just the
   *  current leg's. Feeds the honest-refusal check in `_commitPlaneSegment`
   *  (playtest-2 review finding B): welding (`find_or_plan_vertex`,
   *  kernel/src/sketch.rs) and region detection are strictly PER-SKETCH (no
   *  cross-sketch vertex graph — ARCHITECTURE.md's "no implicit welding"
   *  applies here too), so a chain that re-homes and then visually returns
   *  to a point from an EARLIER sketch creates a merely-coincident new
   *  vertex there — not a weld, not a closed region — with nothing to tell
   *  the kernel otherwise. Reset to `[]` at every point `_prevSegmentDir`
   *  is (a fresh chain start or a chain end), for the same reason: both
   *  describe "no committed history for the chain in progress". */
  private _chainVertices: { point: V3; sketch: bigint }[] = []

  /** The last pointer ray's direction seen by either `onPointerMove` or
   *  `onPointerDown` — a best-effort "current view direction" for
   *  `rehomePlaneNormal`'s view-facing fallback (tool-parity playtest2
   *  §2b), since a typed (VCB) commit has no ray of its own at commit time.
   *  Never reset — a stale value from a moment ago is still a reasonable
   *  camera-direction proxy, and there is always a fresh one by the time a
   *  gesture reaches its second point (the pointer had to move there). */
  private _lastViewDir: V3 | null = null

  /** Idle plane lock (design §5.2): while FULLY idle (no anchored stage),
   *  an arrow key locks the future plane's NORMAL to a world axis (0=X/red,
   *  1=Y/green, 2=Z/blue — `arrowToAxis`); the same arrow again, or
   *  Escape/ArrowDown, clears it. An ACTIVE lock overrides face pick and
   *  sketch-hover adoption on the next click (SketchUp: an explicit lock
   *  beats inference) — see `_currentMode`/`_resolveClickTarget`. Survives a
   *  completed gesture (cleared only by `cancel()`, which
   *  `onDocumentReset()`/`setEditContext()` already route through). */
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

  /** The single editing-context channel (component-edit-parity.md phase A1;
   *  replaces `setActiveContext`). Re-asserting the SAME context must not
   *  abort an in-progress gesture — mirrors the old field's guard. */
  setEditContext(ctx: EditContext): void {
    if (editContextEq(ctx, this._editContext)) return
    this._editContext = ctx
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
  snapConstraint(ray: Ray): SnapConstraint | null {
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
        // A plane-mode chain can HONOUR an off-plane point snap: the commit
        // re-homes onto a new sketch plane through the anchor and the
        // snapped point (`_commitPlaneSegment`), so a visible vertex on an
        // EARLIER sketch of this same chain — the chain's own origin after
        // two axis-locked re-homes, the 3d-line staircase — is a
        // legitimate target the frozen plane must not veto. Face mode
        // deliberately does NOT set this (above): a face path commits into
        // the face's own plane and cannot re-home, so an off-plane snap
        // there would be a lie.
        offPlanePoints: true,
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
      frame: getDrawingAxes(this.wasmScene),
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
    return resolveIdleDrawTarget(this.wasmScene, this._sketchPickCache, ray, this._editContext)
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
    return resolveClickDrawTarget(
      this.wasmScene, this._sketchPickCache, this.idlePlaneLock, snap, ray, this._editContext,
    )
  }

  /** The cursor's position on `plane`. On the ground plane with NO active
   *  direction lock, this is EXACTLY `[snap.x, snap.y, 0]` (no basis math,
   *  snap required) — the legacy fast path, bit-identical to before this
   *  module existed. `this.lockAxis` is always null here at the very first
   *  click of a chain (`capturingInput()` gates every place that sets it),
   *  so an unanchored anchor point always takes this exact branch too.
   *
   *  On the ground plane WITH an active lock (`this.lockAxis !== null`,
   *  reachable only once a chain is anchored): the full snap, z included.
   *  A hard lock's resolved point can genuinely leave z = 0 (an up-arrow
   *  Z lock from a ground anchor is the canonical case — Kurt's "axis-lock
   *  at any point... it is the SketchUp way"); forcing it back to the
   *  anchor's own z made every such click collapse to zero length and get
   *  refused as degenerate (`_commitPlaneSegment`'s guard) — a silent
   *  no-op. Letting it through is safe: `_commitPlaneSegment` already
   *  detects a locked point that left the frozen plane and re-homes the
   *  chain onto a genuine new plane (`_rehomeChain`) instead of ever
   *  committing an off-plane point — the same machinery already proven for
   *  a locked segment leaving a non-ground frozen plane. When the lock
   *  direction happens to stay in-plane (e.g. an X/Y lock with the default
   *  unrotated drawing axes), the snap's z is exactly the anchor's z
   *  already, so this is bit-identical to the unlocked branch in that case.
   *
   *  On any OTHER (non-ground) plane: the snap (already plane-constrained
   *  via `snapConstraint`) if present, else ray∩plane — unchanged. */
  private _planeCursor(snap: Snap | null, ray: Ray, plane: DrawPlane): V3 | null {
    if (plane.ground) {
      if (snap === null) return null
      // Records whether the z about to be discarded was actually carrying
      // information — see `snapProjected`.
      this._snapProjected = this.lockAxis === null && snap.z !== 0
      if (this.lockAxis !== null) return [snap.x, snap.y, snap.z]
      return [snap.x, snap.y, 0]
    }
    this._snapProjected = false
    if (snap !== null) return [snap.x, snap.y, snap.z]
    return pointOnPlane(ray, plane)
  }

  /** See `Tool.snapProjected`. Set by `_planeCursor`, which is where the
   *  drawing plane's z actually replaces the snap's. */
  snapProjected(): boolean {
    return this._snapProjected
  }

  /** Whether the last `_planeCursor` discarded a non-zero snap z. */
  private _snapProjected = false

  onPointerMove(snap: Snap | null, ray: Ray): void {
    this._lastViewDir = ray.direction
    if (this._currentMode(ray) === 'face') {
      this._onPointerMoveFace(snap, ray)
    } else {
      this._onPointerMovePlane(snap, ray)
    }
  }

  /**
   * Decide which mode governs the NEXT pointer event (same contract as
   * Rectangle/Circle/Polygon/Arc):
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
    this._drawRubberBandSegment(anchor, cursor, this._previewStyle(snap))
    this._reportMeasurement(anchor, cursor)
    this._publishTransient()
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
    this._drawRubberBandSegment(last, cursor, this._previewStyle(snap))
    this._reportMeasurement(last, cursor)
    this._publishTransient()
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
   *   which touch the kernel sketch until `split_face` commits).
   * - Plane mode: nothing — committed segments are already persistent via
   *   `reconcile`/`register_sketch`.
   *
   * The LIVE rubber-band segment (anchor → cursor) is deliberately NOT
   * published, in either mode. Its far endpoint IS the cursor of the
   * previous pointer move, which sits inside the next query's pick cone
   * almost by definition — so publishing it made the engine snap to the
   * tool's own feedback: a phantom "Endpoint" one frame behind the cursor,
   * at a point where no geometry exists. On a single plane the phantom hid
   * within the snap radius of the true cursor; on a re-homed chain it
   * reported (and committed) a ray∩frozen-plane point metres from the
   * visible target — the 3d-line staircase defect's lying "Endpoint".
   * A tool must never be offered its own rubber band as evidence.
   *
   * Always clears the previous publish first (replace semantics).
   */
  private _publishTransient(): void {
    this.wasmScene.clear_transient_segments()

    if (this.faceStage.kind === 'anchored') {
      const { points } = this.faceStage
      for (let i = 0; i < points.length - 1; i++) {
        this._publishSegment(points[i], points[i + 1])
      }
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
    this._lastViewDir = ray.direction
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

  /**
   * True while a gesture is anchored OR an idle plane lock is armed — Escape
   * has tool-local work to do (clear the lock, or step the gesture back)
   * before a context-pop is appropriate (component-edit-parity.md phase A2;
   * see `toolHasArmedGesture` in tools/types.ts). `capturingInput()` alone
   * misses the idle-locked case: locked-but-idle is not "capturing input"
   * but IS armed for Escape's purposes.
   */
  hasArmedGesture(): boolean {
    return this.capturingInput() || this.idlePlaneLock !== null
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
    this._prevSegmentDir = null
    this._chainVertices = []
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
    this._prevSegmentDir = null
    this._chainVertices = []
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

  /**
   * Kernel history changed (undo/redo, any entry point — `types.ts`'s
   * `onHistoryChanged` doc). `_chainVertices` describes segments this SAME
   * chain has already committed, purely so the cross-sketch coincidence
   * check (`_commitPlaneSegment`'s "spans two planes" toast) can tell a
   * phantom coincidence from a real one; an undo can remove the very
   * segment a cached vertex came from, and a phantom entry then misfires
   * the toast against a perfectly legitimate close (tool-parity DELTA
   * review finding 2). Dropping the whole cache is the safe direction: the
   * worst case is a missed toast right after an undo (the kernel is still
   * the actual authority on whether a close succeeds), never a wrongly
   * refused one. The gesture itself is untouched — `planeStage`/
   * `faceStage` stay anchored, so the chain keeps drawing from wherever it
   * was; only this derived bookkeeping is invalidated.
   */
  onHistoryChanged(): void {
    this._chainVertices = []
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
      this._prevSegmentDir = null
      this._chainVertices = []
      this._lastPlaneCursor = null
      this.typed = ''
      this.onMeasurementCb('')
      this._publishTransient()
    } else {
      const { plane, target, anchor } = this.planeStage
      const cursor = this._planeCursor(snap, ray, plane)
      if (cursor === null) return
      this._commitPlaneSegment(plane, target, anchor, cursor)
    }
  }

  /**
   * Re-home the chain (tool-parity playtest2 §2b) when a locked segment's
   * endpoint has left the CURRENT frozen plane: finalize the sketch as it
   * stands (its already-committed segments stay exactly where they are —
   * nothing further is added to it) and resolve a NEW plane through
   * `anchor` that genuinely contains the segment being drawn, so the point
   * can commit instead of being refused.
   *
   * The new plane is chosen per the design: the plane spanned by the
   * PREVIOUS segment's direction and this one, when the two aren't
   * parallel (keeps an L-shaped chain coplanar, so it can still close a
   * region); otherwise the view-facing plane containing this segment's
   * direction (`rehomePlaneNormal`, lineInput.ts — pure, unit-tested on its
   * own). The returned `target` is an ordinary PLANE-mode target
   * (`SketchTarget.plane`), so the caller's `runSketchGesture` call mints
   * (or reuses, via the shared `SketchPlaneCache`) a sketch for it exactly
   * like any other plane-mode commit — from the kernel's perspective this
   * is nothing special: begin a sketch (on a cache miss) and add one
   * segment, both inside the SAME undo-gesture bracket `runSketchGesture`
   * already opens per commit. A re-homing commit therefore costs no
   * surprise extra undo step: Cmd+Z after it undoes exactly the one segment
   * just placed (and the sketch that segment alone created, if any), same
   * as undoing any other single LineTool segment.
   */
  private _rehomeChain(anchor: V3, cursor: V3): { plane: DrawPlane; target: SketchTarget } {
    // `cursor` was already confirmed non-degenerate with `anchor` by the
    // caller, so `directionBetween` cannot return null here.
    const segDir = directionBetween(anchor, cursor) ?? [0, 0, 1]
    const viewDir = this._lastViewDir ?? [0, 0, -1]
    const normal = rehomePlaneNormal(segDir, this._prevSegmentDir, viewDir)
    // `normal` is always a genuine unit vector (`rehomePlaneNormal` is
    // total), so `facePlaneBasis` never actually returns null here; the
    // fallback just keeps this function total without a non-null assertion.
    const { u, v } = facePlaneBasis(normal) ?? { u: [1, 0, 0] as V3, v: [0, 1, 0] as V3 }
    const plane: DrawPlane = { origin: anchor, normal, u, v, ground: isGroundPlane(anchor, normal) }
    return { plane, target: { kind: 'plane', plane, instance: this._activeInstance } }
  }

  /** Commit one segment anchor -> cursor, then chain forward from cursor. */
  private _commitPlaneSegment(plane: DrawPlane, target: SketchTarget, anchor: V3, cursor: V3): void {
    // Skip degenerate zero-length segments — one Euclidean check at
    // `DEGENERATE_SEGMENT_EPS` (the kernel's own `DegenerateSegment` line,
    // see the constant's doc) for ground and non-ground alike. This used to
    // be a per-axis legacy check at a hardcoded 1e-8 on ground and a
    // Euclidean 1e-8 elsewhere; both refused segments the kernel accepts,
    // and the inference engine's lock-projection disqualification (which
    // reads `tol::POINT_MERGE` directly) could therefore return a winner
    // this gate then refused — the nanometre-band remnant of the locked-
    // click degeneracy. The Euclidean form is the kernel's own predicate,
    // so the per-axis shape (whose stated purpose was bit-identical gating
    // at the OLD threshold) has nothing left to preserve.
    //
    // A refused click must never be SILENT (tool-parity playtest-2 defect: a
    // resolved snap that collapses back onto `anchor` — e.g. an inference
    // candidate for an axis the cursor never actually moved along — used to
    // discard the click with no toast, no error, and no visible feedback at
    // all, the worst failure mode for a drawing tool). `onToast` here is a
    // defensive backstop independent of WHY the click collapsed — an honest
    // "nothing to commit" beats silence whether the cause is an ordinary
    // same-point re-click or a genuine snap-resolution defect, and mirrors
    // `OffsetTool._commit`'s identical guard/toast for the same reason.
    if (segmentLength(anchor, cursor) <= DEGENERATE_SEGMENT_EPS) {
      this.onToast("That point is the same as the last one — move the cursor before clicking")
      return
    }

    // Re-home (design §2b) instead of letting the kernel refuse: `cursor`
    // can legitimately leave the frozen `plane` two ways — a locked segment
    // direction aimed off it (the inference lock branch is deliberately not
    // plane-constrained — a directional inference, not a candidate snap),
    // or an off-plane POINT snap (`snapConstraint`'s `offPlanePoints`: a
    // chain that already re-homed must still snap back to, e.g., its own
    // origin vertex on an EARLIER sketch's plane — the 3d-line defect).
    // Predict that BEFORE committing, with the exact tolerance the kernel's
    // own PointOffPlane check uses (`isPointOnDrawPlane` mirrors
    // `kernel::tol::PLANE_DIST`), rather than committing speculatively and
    // catching the refusal. Unlocked, unsnapped cursors are already on
    // `plane` by construction (ray∩plane fallback), so this fires exactly
    // when the honoured target genuinely lies off the frozen plane.
    // Reachable from the GROUND plane too (`_planeCursor` lets a locked
    // cursor's z through instead of forcing it back to 0) — an up-arrow Z
    // lock from an ordinary ground anchor, the canonical case, re-homes
    // here exactly like a locked segment leaving any other frozen plane.
    let effectivePlane = plane
    let effectiveTarget = target
    if (!isPointOnDrawPlane(cursor, plane)) {
      const rehomed = this._rehomeChain(anchor, cursor)
      effectivePlane = rehomed.plane
      effectiveTarget = rehomed.target
    }

    try {
      // Each committed segment is its own gesture — one Cmd+Z undoes exactly
      // that segment, matching LineTool's chain-forward-per-click semantics.
      runSketchGesture(this.wasmScene, this.sketchCache, effectiveTarget, (sketch, toLocal) => {
        // `toLocal` is the identity for a world target and pose⁻¹ for a
        // definition-owned one (component-edit-parity.md phase A2) — see
        // `runSketchGesture`'s doc for why an ordinary `sketch_add_segment`
        // needs this and `begin_sketch_on_plane_in_instance` doesn't.
        const a = toLocal(anchor)
        const b = toLocal(cursor)
        const report = this.wasmScene.sketch_add_segment(
          sketch,
          a[0], a[1], a[2],
          b[0], b[1], b[2],
        )
        let closed: boolean
        try {
          closed = report.regions_created().length > 0
        } finally {
          report.free()
        }

        // Honest refusal instead of silent failure (tool-parity playtest-2
        // review finding B): vertex welding (`find_or_plan_vertex`) and
        // region detection are strictly PER-SKETCH in the kernel — there is
        // no cross-sketch vertex graph, matching the project's "no implicit
        // welding" invariant (docs/agents/ARCHITECTURE.md). A chain that re-homed
        // onto a NEW sketch (`_rehomeChain`, §2b) and then visually returns
        // to a point that belongs to an EARLIER sketch of this same chain
        // therefore creates a merely-coincident vertex THERE, not a weld —
        // `closed` comes back false with no region and no error, exactly
        // like any other still-open chain. Tell the user honestly rather
        // than silently accepting geometry that looks closed but isn't; an
        // ordinary "not closed yet" (no coincident earlier-sketch point) is
        // unaffected and stays silent, as intended.
        if (!closed) {
          const spansEarlierSketch = this._chainVertices.some(
            (v) => v.sketch !== sketch && segmentLength(v.point, cursor) < CHAIN_VERTEX_MERGE_EPS,
          )
          if (spansEarlierSketch) {
            this.onToast(
              "This outline spans two planes, so it can't close into a single face — finish the shape on one plane instead.",
            )
          }
        }
        if (this._chainVertices.length === 0) this._chainVertices.push({ point: anchor, sketch })
        this._chainVertices.push({ point: cursor, sketch })

        this.onCommit(sketch)

        if (closed) {
          // The loop closed into a face — end the chain (idle, ready for a new line).
          this.planeStage = { kind: 'idle' }
          this.typed = ''
          this._lastPlaneCursor = null
          this._prevSegmentDir = null
          this._chainVertices = []
          this.lockAxis = null
          this.shiftAxisLock = false
          this._clearPreview()
          this.onMeasurementCb('')
          this.wasmScene.clear_transient_segments()
        } else {
          // Chain forward: the new point becomes the anchor for the next
          // segment, on the (possibly just re-homed) frozen plane/target.
          this.planeStage = { kind: 'anchored', plane: effectivePlane, target: effectiveTarget, anchor: cursor }
          this._prevSegmentDir = directionBetween(anchor, cursor)
          this._lastPlaneCursor = null
          this.typed = ''
          this._clearPreview()
          this.onMeasurementCb('')
          this._publishTransient()
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
      const normal = worldFaceNormal(this.wasmScene, objectHandle, faceHandle, this._activeInstance)
      if (normal === null) return // stale instance/degenerate pose — treat as no eligible face
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
      this._publishTransient()
    } else {
      const cursor = this._faceCursor(snap, ray)
      if (cursor === null) return
      this._appendFacePoint(cursor)
    }
  }

  /**
   * Append a point to the face-mode path, skipping a degenerate (zero-length)
   * segment.
   *
   * A refused click must never be SILENT (tool-parity playtest-2 defect —
   * see `_commitPlaneSegment`'s identical guard/toast) and delta-review
   * finding 2: this face-mode guard had the same collapse-to-the-last-point
   * failure mode as the plane-mode one but, unlike it, discarded the click
   * with no toast at all. `onToast` here is the same defensive backstop for
   * the same reason — an honest "nothing to commit" beats silence whether
   * the cause is an ordinary same-point re-click or a genuine
   * snap-resolution defect.
   */
  private _appendFacePoint(point: V3): void {
    if (this.faceStage.kind !== 'anchored') return
    const { points } = this.faceStage
    const last = points[points.length - 1]
    if (segmentLength(last, point) <= DEGENERATE_SEGMENT_EPS) {
      this.onToast("That point is the same as the last one — move the cursor before clicking")
      return
    }

    points.push(point)
    this._lastFaceCursor = null
    this.typed = ''
    this._clearPreview()
    this.onMeasurementCb('')
    this._publishTransient()
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
      // Inside a component instance's editing context (component-edit-
      // parity.md phase A2), `object` is a definition member — the world
      // `split_face` refuses it outright (`apply_object_op`'s `is_world`
      // guard); `split_face_in_instance` maps `path` through the instance's
      // pose⁻¹ and routes through `apply_def_op` instead. Every instance of
      // the definition sees the cut at once.
      if (this._activeInstance !== null) {
        const report = this.wasmScene.split_face_in_instance(this._activeInstance, object, face, path)
        report.free()
      } else {
        const report = this.wasmScene.split_face(object, face, path)
        report.free()
      }
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
  private _drawRubberBandSegment(a: V3, b: V3, style: FatSegmentsOpts = PREVIEW_LINE_STYLE): void {
    const pts = new Float32Array([...a, ...b])
    this.preview.add(makeFatSegments(pts, style))
  }

  /**
   * The rubber band's colour/weight for the CURRENT resolved `snap`
   * (tool-parity playtest2 §2c — "the line turns green [...] and locks in
   * on that axis"). `snap.direction` is set exactly when the kernel's
   * result is axis-relevant: the kernel's OWN soft-axis inference (no lock
   * held — `SnapKind::OnAxis`, generated through the anchor per
   * `inference::resolve`'s docs) OR an active `lockAxis` hold (which
   * attaches its direction to whatever the lock resolved to, `OnAxis` or a
   * point candidate riding the locked line alike). Either way, painting the
   * whole segment in that axis's colour is the visual Kurt asked for.
   *
   * Hard (`this.lockAxis !== null`) reads BOLDER than soft — SketchUp's own
   * convention for an explicit hold, and the one thing this preview must do
   * to keep the two readable apart (see `HARD_LOCK_PREVIEW_WIDTH_PX`'s doc).
   * Falls back to the plain `PREVIEW_LINE_STYLE` (blue) whenever there's no
   * direction to paint, or it doesn't read as one of the three frame axes.
   */
  private _previewStyle(snap: Snap | null): FatSegmentsOpts {
    if (snap === null || snap.direction === undefined) return PREVIEW_LINE_STYLE
    const match = axisColorForDirection(
      snap.direction,
      LINE_AXIS_PREVIEW_TOL_DOT,
      axisColorsForTheme(getResolvedTheme()),
      getDrawingAxes(this.wasmScene),
    )
    if (match === null) return PREVIEW_LINE_STYLE
    return {
      ...PREVIEW_LINE_STYLE,
      color: match.color,
      widthPx: this.lockAxis !== null ? HARD_LOCK_PREVIEW_WIDTH_PX : PREVIEW_LINE_STYLE.widthPx,
    }
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
