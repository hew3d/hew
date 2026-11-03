/**
 * ArcTool — SketchUp-style 2-point arc, drawn as a faceted polyline chain.
 * Mirrors CircleTool: the arc decomposes into N chained segments inside a
 * curve bracket carrying the arc's analytic circle, committed as one draft
 * on the plane or face (`runDraft`) — an open chain by default, closed only
 * by a completion mode (see `_commitFace`). The circle rides only the arc's
 * own leading edges; any pie/segment closing edges are plain lines.
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
 * lie in the picked face's plane via `snapConstraint`; the commit is one
 * draft on that face (carrying the arc's analytic circle — see
 * `_commitFace`), which the engine lowers into the face or keeps as drawing.
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
 *     hasn't moved off A, or points straight off a non-ground plane, so it
 *     lands on A; a toast says which) or the typed distance is below
 *     ARC_MIN_CHORD_M. On the ground, a cursor straight above or below A
 *     (the Blue axis) is NOT flattened: the chord goes straight up and the
 *     arc moves onto a vertical plane through it, exactly as a Line
 *     re-homes (`_planeCursor`, `_placePlaneB`); a click there does the
 *     same.
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
 * so a region (plane mode) or a face imprint (face mode, a
 * draft on the face like CircleTool) appears immediately and push/pull
 * works. The live preview draws the closing edges and the radius readout
 * names the mode. The mode persists across commits until the tool is
 * switched (tools are recreated on every switch, so a fresh Arc tool always
 * starts `open`) or the document is replaced (`onDocumentReset`).
 */

import { t } from '@lingui/core/macro'
import * as THREE from 'three'
import type { Tool, Snap, EditContext } from './types'
import { editContextEq } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { V3 } from '../viewport/geoHelpers'
import { facePlaneBasis } from '../viewport/geoHelpers'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { makeFatSegments, disposeFatSegments } from '../viewport/fatLine'
import { displayedLength, isFreePick, sagittaForDisplayedRadius } from './displayedValue'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { segmentLength, directionBetween, rehomePlaneNormal } from './lineInput'
import { editLengthBuffer, isLengthInputKey, pointAlong, nextIdlePlaneLock, axisLockColorName, radiusReadout } from './moveInput'
import { RetypeWindow, idleRetypeCapturesKey, retypeStaleMessage } from './retypeWindow'
import type { SketchTarget } from './sketchGesture'
import { DetachedMode, type OnDetachedChange } from './detachedMode'
import { runDraft, addDraftSegment, refreshAfterDraft, draftTargetFromSketchTarget } from './draftCommit'
import { drawPlaneCue, drawPlaneThrough, isGroundPlane, SketchPickCache, resolveIdleDrawTarget, resolveClickDrawTarget, nextGestureLockPlane, groundNaturalTarget, type DrawPlane } from './drawPlane'
import { getDrawingAxes } from './drawingAxes'
import { ineligibleFacePlane, FacePickCache, defaultFaceEligible, worldFaceNormal, worldFacePlane, snapOnObjectBoundary, type FaceEligible, type FaceThroughPick, cursorOnFrozenPlane } from './faceDraw'
import { PlanePin } from './planePin'
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
function flatBulgeHint(): string {
  return t`Pull out the bulge`
}

/** Toast for a typed chord length with no direction to apply it in: the
 *  cursor is still on A (`_commitTypedPlaneChord`/`_commitTypedFaceChord`). */
const arcNoDirectionMessage = (): string => t`Move the cursor to point the arc, then type the length.`

/** The same refusal when the cursor IS pointing, but straight off the arc's
 *  plane, so dropped onto the plane it lands on A: a snap straight off a
 *  non-ground plane, or a point straight over a ground A while an arrow
 *  lock or a Shift pin holds the ground (the chosen plane holds, and those
 *  stages offer no soft Blue axis from A — `snapConstraint` — so only a
 *  real point straight over A gets here). An unlocked, unpinned ground A
 *  never does: a cursor straight over it re-homes the arc (`_planeCursor`).
 *  A click there gets the same toast (`_onPointerDownPlane`). An arrow key
 *  re-locks the plane through A (chord stage only). */
const arcOffPlaneMessage = (): string =>
  t`An arc stays on its drawing plane. Move the cursor along the plane, or press an arrow key to change it, then type the length.`

/** How far (metres) a snap may sit from straight over a ground A and still
 *  count as on the Blue axis through it (`_isStraightOverA`) — the same
 *  degenerate-segment line LineTool's `_isStraightOverAnchor` uses. */
const STRAIGHT_OVER_EPS_M = 1e-9

/** How the committed arc is closed (Alt cycles, in this order). */
export type ArcCompletion = 'open' | 'pie' | 'segment'

const COMPLETION_CYCLE: readonly ArcCompletion[] = ['open', 'pie', 'segment']

/** Readout label per mode ('open' shows the bare radius). */
function completionLabel(completion: ArcCompletion): string {
  switch (completion) {
    case 'open':
      return ''
    case 'pie':
      return t({ context: 'arc completion', message: 'Pie' })
    case 'segment':
      return t({ context: 'arc completion', message: 'Segment' })
  }
}

/** Plane gesture: idle → endpoint A placed (chord) → chord (A,B) placed
 *  (bulge), on a frozen `DrawPlane`/`SketchTarget`. `natural` (design §2a) is
 *  the plane/target the chord would have anchored onto at endpoint A's own
 *  click had no idle lock been active — carried through `bulge` (unused
 *  there; mid-gesture re-lock is scoped to the `chord` stage — see the
 *  onKey doc) only so `_stepBack` can restore it verbatim when it steps
 *  bulge back to chord. */
type PlaneStage =
  | { kind: 'idle' }
  | {
      kind: 'chord'
      plane: DrawPlane
      target: SketchTarget
      a: V3
      natural: { plane: DrawPlane; target: SketchTarget }
      /** Set when an arrow lock moved a FACE-anchored chord onto this plane
       *  (GitHub #51): releasing the lock goes back to that face instead of
       *  to `natural`. */
      fromFace?: FaceChordStage
    }
  | {
      kind: 'bulge'
      plane: DrawPlane
      target: SketchTarget
      a: V3
      b: V3
      natural: { plane: DrawPlane; target: SketchTarget }
      /** Carried from the chord stage so Esc back to it keeps the face. */
      fromFace?: FaceChordStage
      /** Set when B went straight up or down from a ground A (the Blue
       *  axis, `_isStraightOverA`) and the gesture moved onto a vertical
       *  plane through the chord (`_placePlaneB`): the chord stage's plane
       *  and target before that, which Esc back to the chord restores. */
      rehomedFrom?: { plane: DrawPlane; target: SketchTarget }
    }

/** Face gesture: same stages, on a locked face plane. */
type FaceChordStage = {
  kind: 'chord'
  object: bigint
  face: bigint
  normal: V3
  /** A world-space point that lies on the face plane (the first click position) */
  planePoint: V3
  a: V3
}
type FaceStage =
  | { kind: 'idle' }
  | FaceChordStage
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


/**
 * The just-committed arc, kept in a `RetypeWindow` (retypeWindow.ts) so a
 * bulge typed AFTER the third click redraws it in place — the chord `a`→`b`
 * stays, `s` is the signed sagitta it was drawn with (its sign fixes which
 * side the new bulge goes), `completion` the open / pie / segment closure
 * in force at the commit.
 */
type RetypeSpec = {
  a: V3
  b: V3
  s: number
  completion: ArcCompletion
  /** Detached mode the arc was drawn with — a retyped bulge re-commits with
   *  the same mode (separate-shapes design §6.1). */
  detached: boolean
} & (
  | { mode: 'plane'; plane: DrawPlane; target: SketchTarget }
  | { mode: 'face'; object: bigint; face: bigint; normal: V3 }
)

export class ArcTool implements Tool {
  readonly name = 'Arc'
  /** Click-drag-release works as well as click-click (Tool.dragCommits). */
  readonly dragCommits = true

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    const stage = this.faceStage.kind !== 'idle' ? this.faceStage.kind : this.planeStage.kind
    const idle = stage === 'idle' && !this.retype.isOpen
    return this.sep.hint(this._stageHint(), idle)
  }

  private _stageHint(): string {
    if (!this.capturingInput() && this.idlePlaneLock === null && this.retype.isOpen) {
      return t`Type an exact bulge to redraw the arc you just drew — or click the first point of the next one.`
    }
    const stage = this.faceStage.kind !== 'idle' ? this.faceStage.kind : this.planeStage.kind
    if (stage === 'chord') {
      return t`Click the arc's second endpoint — or type an exact chord length.`
    }
    if (stage === 'bulge') {
      return t`Click to set the curve — Alt cycles open arc / pie / segment; or type an exact bulge.`
    }
    if (this.idlePlaneLock !== null) {
      const axis = axisLockColorName(this.idlePlaneLock)
      return t`Locked to the ${axis} plane — click to start; same arrow or Esc unlocks.`
    }
    if (this.pin.current !== null) {
      return t`Pinned to the hovered plane — click to start; Shift again or Esc unpins.`
    }
    return t`Click the arc's first endpoint — on the ground plane or any face or sketch.`
  }

  private planeStage: PlaneStage = { kind: 'idle' }
  private faceStage: FaceStage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnArcCommit
  private onFaceImprint: OnFaceImprint
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement
  /** Detached mode (Ctrl/Cmd tap): the next arc joins nothing. */
  private readonly sep: DetachedMode

  /** The currently active editing context (entered object), or null at top level. */
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

  /** Completion mode for committed arcs — Alt cycles it mid-gesture and it
   *  persists across commits (see module doc). */
  private completion: ArcCompletion = 'open'

  /** VCB buffer — raw string being typed by the user (length, in display
   *  units). While IDLE with the retype window open it is the post-click
   *  bulge being typed for the arc just drawn. */
  private typed: string = ''

  /** The just-committed arc, redrawable by a typed bulge until the next
   *  pointer action, Escape, tool switch, or any other document mutation
   *  (see `RetypeSpec` and retypeWindow.ts). */
  private readonly retype: RetypeWindow<RetypeSpec>

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

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnArcCommit,
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

  /** Set the active editing context (entered object), or null for top level. */
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
   * miss, which used to send the whole gesture to the GROUND plane (an Arc
   * started on a vertical edge's midpoint landed at z = 0) — so the ranked
   * boundary pick (`FacePickCache.faceThrough`) answers instead: the most
   * camera-facing eligible face whose plane holds the snapped point. Any
   * other snap keeps the plain pick, with the plane read from it.
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
  private _resolveIdleTarget(ray: Ray, snap: Snap | null = null): { plane: DrawPlane; target: SketchTarget } {
    return resolveIdleDrawTarget(this.wasmScene, this._sketchPickCache, ray, this._editContext, () => ineligibleFacePlane(this.wasmScene, this._pickCache, ray, (o, i) => this._isEligible(o, i), snap), snapOnObjectBoundary(snap) ? `${snap.x},${snap.y},${snap.z}` : '')
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
    const pinned = this.pin.clickTarget(this._editContext)
    if (pinned !== null) return pinned
    return resolveClickDrawTarget(
      this.wasmScene, this._sketchPickCache, this.idlePlaneLock, snap, ray, this._editContext,
      () => ineligibleFacePlane(this.wasmScene, this._pickCache, ray, (o, i) => this._isEligible(o, i), snap),
      snapOnObjectBoundary(snap) ? `${snap.x},${snap.y},${snap.z}` : '',
    )
  }

  /** The cursor's position on `plane`. On the ground plane this is EXACTLY
   *  `[snap.x, snap.y, 0]` (no basis math, snap required) — the legacy fast
   *  path, bit-identical to before this module existed. On any other plane:
   *  the snap (already plane-constrained via `snapConstraint`) if present,
   *  else ray∩plane.
   *
   *  One exception on the ground, mirroring LineTool's `_planeCursor`: in
   *  the chord stage, a snap straight above or below A (the Blue axis, or
   *  any point vertically over it — `_isStraightOverA`) keeps its height.
   *  Flattened it would land on A itself, so a typed chord had no direction
   *  and a click was dropped as a zero chord; kept whole, the chord goes
   *  straight up and `_placePlaneB` moves the gesture onto a vertical plane
   *  through it, as Line re-homes its segment. */
  private _planeCursor(snap: Snap | null, ray: Ray, plane: DrawPlane): V3 | null {
    this._lastViewDir = ray.direction
    this._cursorStraightOver = false
    if (plane.ground) {
      if (snap === null) return null
      if (this._isStraightOverA(snap)) {
        this._snapProjected = false
        this._cursorStraightOver = true
        return [snap.x, snap.y, snap.z]
      }
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

  /** Whether `snap` sits straight above or below a ground-plane chord
   *  stage's A, with no arrow lock or Shift pin choosing the plane (an
   *  explicit plane holds — `_planeCursor`). False in every other stage. */
  private _isStraightOverA(snap: Snap): boolean {
    if (this.planeStage.kind !== 'chord' || !this.planeStage.plane.ground) return false
    if (this.idlePlaneLock !== null || this.pin.current !== null) return false
    const { a } = this.planeStage
    if (Math.hypot(snap.x - a[0], snap.y - a[1]) > STRAIGHT_OVER_EPS_M) return false
    return Math.abs(snap.z - a[2]) > STRAIGHT_OVER_EPS_M
  }

  /** The last pointer ray's direction — the view `_rehomedChordPlane`
   *  turns the vertical plane toward. */
  private _lastViewDir: V3 | null = null

  /** Whether the last `_planeCursor` kept a snap straight over a ground A
   *  whole (`_isStraightOverA`) — the one case `_placePlaneB` re-homes. */
  private _cursorStraightOver = false

  /** `_cursorStraightOver` as it was for `_lastPlaneCursor`, so a typed
   *  chord along that cursor re-homes exactly when a click there would. */
  private _lastPlaneCursorStraightOver = false

  /** See `Tool.snapProjected`. Set by `_planeCursor` and `_faceCursor`,
   *  where the drawing plane replaces an off-plane snap (the ground drops its
   *  height; a face or sketch plane drops it straight onto itself). */
  snapProjected(): boolean {
    // Only a gesture under way projects anything: once the stage is back to
    // idle (a commit, Esc, a context change) the last verdict must not ride
    // along on ordinary hover cues.
    return this._snapProjected && (this.faceStage.kind !== 'idle' || this.planeStage.kind !== 'idle')
  }

  /** Whether the last `_planeCursor` / `_faceCursor` moved the snap onto its plane. */
  private _snapProjected = false

  /**
   * Decide which mode governs the NEXT pointer event (same contract as the
   * other draw tools): sticky mid-gesture; always face mode inside an
   * entered object context (scoped drawing — no top-level plane sketch from
   * inside); else decided by what's under the cursor.
   */
  private _currentMode(ray?: Ray, snap: Snap | null = null): 'face' | 'plane' {
    if (this.faceStage.kind !== 'idle') return 'face'
    if (this.planeStage.kind !== 'idle') return 'plane'
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
   * - Face mode, gesture in progress: return the already-locked face plane.
   * - Plane mode, gesture in progress on a NON-ground plane (sketch mode):
   *   same — return the frozen plane. Ground-anchored: no constraint
   *   plane; the chord stage passes A as the soft-axis anchor.
   * - Idle: pick the hovered face (if an eligible one is under the cursor)
   *   and return its plane so the FIRST-click endpoint lands precisely on
   *   the face; absent that, a top-level hover over a non-ground sketch
   *   returns ITS plane.
   * - Otherwise (ground mode): return null (unconstrained).
   */
  snapConstraint(ray: Ray): { anchor?: [number, number, number]; constraintPlane?: { point: [number, number, number]; normal: [number, number, number] }; seeThroughPoints?: boolean } | null {
    if (this.faceStage.kind !== 'idle') {
      // Gesture in progress: lock to the established face plane
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

    if (this.planeStage.kind !== 'idle') {
      if (this.planeStage.plane.ground) {
        // The chord stage on the ground offers the soft axes through A, as
        // Line's anchored chain does: Red and Green along the ground, and
        // Blue straight up, which `_planeCursor` keeps whole so the chord
        // re-homes onto a vertical plane (`_placePlaneB`). Not under an
        // arrow lock or Shift pin: that plane holds (`_isStraightOverA`),
        // so a Blue snap would only flatten onto A.
        const offersAxes = this.planeStage.kind === 'chord' && this.idlePlaneLock === null && this.pin.current === null
        return offersAxes ? { anchor: this.planeStage.a } : null
      }
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

    const plane = this._idleHoverPlane(ray)
    if (!plane.ground) {
      return { constraintPlane: { point: plane.origin, normal: plane.normal }, seeThroughPoints: true }
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
    // Hovering an object's edge or corner: the plain ray pick misses there
    // (strict at the boundary), so the ranked boundary pick answers which
    // face the point belongs to — a Shift press over a wall's edge
    // midpoint pins that WALL, not the ground the miss fell through to.
    if (snapOnObjectBoundary(snap)) {
      const through = this._facePickAt(snap, ray)
      if (through !== null) {
        const drawPlane = drawPlaneThrough(through.point, through.normal)
        if (drawPlane !== null) return drawPlane
      }
    }
    const eligible = this._eligiblePickFor(ray)
    if (eligible !== null) {
      const plane = worldFacePlane(this.wasmScene, eligible.object, eligible.face, this._activeInstance)
      if (plane !== null) {
        const drawPlane = drawPlaneThrough(plane.point, plane.normal)
        if (drawPlane !== null) return drawPlane
      }
    }
    return this._resolveIdleTarget(ray, snap).plane
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
    })
  }

  /**
   * Shift pressed/released while IDLE toggles the plane pin (planePin.ts;
   * GitHub issue 14 — "press Shift to lock the plane does nothing"): the
   * plane the last hover recorded becomes the drawing plane until Shift is
   * pressed again or Escape releases it. A pin displaces an arrow-key lock
   * (the two are different shapes of the same choice). Mid-gesture Shift
   * does nothing here — the arc's plane is frozen at its first click.
   */
  setShiftHeld(held: boolean): void {
    if (this.capturingInput()) {
      this.pin.markShift(held) // keep the autorepeat guard honest, never toggle
      return
    }
    if (this.pin.setShiftHeld(held)) {
      this.idlePlaneLock = null
      this._lastIdleHoverPoint = null
    }
  }

  /** Idle hover bookkeeping: the hover point for the plane cue (arrow lock
   *  and Shift pin previews both draw through it) and, for the pin, the
   *  plane the next click would land on (`_idleHoverPlane`). */
  private _trackIdleHover(snap: Snap | null, ray: Ray): void {
    this._snapProjected = false
    if (snap === null) return
    const through: V3 = [snap.x, snap.y, snap.z]
    this._lastIdleHoverPoint = through
    if (this.idlePlaneLock === null) this.pin.trackHover(this._idleHoverPlane(ray, snap), through)
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (this._currentMode(ray, snap) === 'face') {
      this._onPointerMoveFace(snap, ray)
    } else {
      this._onPointerMovePlane(snap, ray)
    }
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    // Any pointer action ends the retype window — the next arc has begun.
    this.disarmRetype()
    if (this._currentMode(ray, snap) === 'face') {
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

  /**
   * True while a gesture is in progress OR an idle plane lock is armed —
   * Escape has tool-local work to do (clear the lock, or step the gesture
   * back) before a context-pop is appropriate (component-edit-parity.md
   * phase A2; see `toolHasArmedGesture` in tools/types.ts).
   * `capturingInput()` alone misses the idle-locked case: locked-but-idle is
   * not "capturing input" but IS armed for Escape's purposes.
   */
  hasArmedGesture(): boolean {
    return this.capturingInput() || this.idlePlaneLock !== null || this.pin.current !== null || this.typed !== ''
  }

  /**
   * Per-key refinement of the capture (see Tool.capturesKey): an armed
   * gesture keeps the whole keyboard, exactly as before; the IDLE retype
   * window takes only what a typed bulge needs (`idleRetypeCapturesKey`).
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
        return
      }
      // Aborting an in-progress gesture keeps the plane lock: the lock is
      // an idle aiming choice, cleared only by an idle Escape or toggle
      // (parity across all four draw tools — LineTool's _endChain path).
      const lock = this.idlePlaneLock
      const pinned = this.pin.current
      this._stepBack()
      this.idlePlaneLock = lock
      this.pin.restore(pinned)
      return
    }

    if (!this.capturingInput()) {
      // Idle retype window (see `RetypeSpec`): the keys `capturesKey` admits
      // edit the buffer; Enter redraws the just-committed arc at that bulge.
      if (this.retype.isOpen && this.capturesKey(ev.key)) {
        if (ev.key === 'Enter') {
          const meters = parseLengthToMeters(this.typed)
          this.typed = ''
          this.onMeasurementCb('')
          if (meters !== null) this._retypeBulge(Math.abs(meters))
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
      }
      return
    }

    // Mid-gesture plane re-lock (design §2a): once endpoint A is placed, an
    // arrow key still re-locks the plane — through A, not the cursor —
    // since nothing has reached the kernel yet at the CHORD stage (the arc
    // commits only on the third click). Scoped to the chord stage: once B is
    // also placed (bulge stage), the plane is pinned by BOTH A and B — an
    // arbitrary axis lock through A alone could no longer honestly contain
    // B, so re-locking there is out of scope (see the module doc).
    //
    // A FACE-anchored chord takes the lock too (GitHub #51 — see
    // RectangleTool's identical block): the gesture leaves the face for the
    // locked plane through A, and releasing the lock puts it back on that
    // face. Inside an entered object the face holds.
    if (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft' || ev.key === 'ArrowUp' || ev.key === 'ArrowDown') {
      // Keydown autorepeat would flip the lock on every repeat (a held
      // arrow alternating plane and face); only the first press counts.
      if (ev.repeat) return
      if (this.faceStage.kind === 'chord') {
        if (this._activeContext !== null) return
        const fromFace = this.faceStage
        const { a } = fromFace
        const natural = groundNaturalTarget(this._editContext, a)
        const next = nextGestureLockPlane(null, ev.key, a, natural, getDrawingAxes(this.wasmScene), this._editContext)
        if (next.lock === null) return // Down with no lock: nothing to release
        this.idlePlaneLock = next.lock
        this.faceStage = { kind: 'idle' }
        this.planeStage = { kind: 'chord', plane: next.plane, target: next.target, a, natural, fromFace }
        this._lastFaceCursor = null
        this._lastPlaneCursor = null
        this._clearPreview()
        this.onMeasurementCb('')
        return
      }
      if (this.planeStage.kind === 'chord') {
        const { a, natural, fromFace } = this.planeStage
        const next = nextGestureLockPlane(
          this.idlePlaneLock, ev.key, a, natural, getDrawingAxes(this.wasmScene), this._editContext,
        )
        this.idlePlaneLock = next.lock
        if (next.lock === null && fromFace !== undefined) {
          this.planeStage = { kind: 'idle' }
          this.faceStage = fromFace
        } else {
          this.planeStage = { kind: 'chord', plane: next.plane, target: next.target, a, natural, fromFace }
        }
        this._lastPlaneCursor = null
        this._lastFaceCursor = null
        this._clearPreview()
        this.onMeasurementCb('')
        return
      }
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
        this.onMeasurementCb(next === 'open' ? t({ context: 'tool', message: 'Arc' }) : completionLabel(next))
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
    const label = completionLabel(this.completion)
    if (label !== '' && text !== '' && this.capturingInput()) return `${text} · ${label}`
    return text
  }

  /** Esc steps back one stage: bulge → chord (A kept), chord → idle. */
  private _stepBack(): void {
    if (this.planeStage.kind === 'bulge') {
      const { a, natural, fromFace, rehomedFrom } = this.planeStage
      const { plane, target } = rehomedFrom ?? this.planeStage
      this.planeStage = { kind: 'chord', plane, target, a, natural, fromFace }
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
    this.retype.close()
    this.planeStage = { kind: 'idle' }
    this.faceStage = { kind: 'idle' }
    this.typed = ''
    this._lastPlaneCursor = null
    this._lastFaceCursor = null
    this._lastSagittaSign = null
    this.idlePlaneLock = null
    this.pin.clear()
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
    this.completion = 'open' // a fresh document starts with the default close
    this.cancel()
    this.sep.reset()
  }

  /** Ctrl/Cmd clean tap (Viewport-driven; a bare Ctrl never reaches onKey):
   *  flip Detached for the next arc (separate-shapes design §6.1). */
  toggleDetached(): void {
    this.sep.toggle()
  }

  /** Whether Detach is on. */
  get detachedOn(): boolean {
    return this.sep.on
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
   *  direction yet, or a sub-ARC_MIN_CHORD_M distance; the no-direction
   *  refusal says why. A Blue-axis hover over a ground A points straight up
   *  (`_planeCursor`), and `_placePlaneB` re-homes the arc onto a vertical
   *  plane through the chord. */
  private _commitTypedPlaneChord(distance: number): void {
    if (this.planeStage.kind !== 'chord') return
    if (distance < ARC_MIN_CHORD_M) return
    const { a } = this.planeStage
    const cursor = this._lastPlaneCursor
    const dir = cursor === null ? null : directionBetween(a, cursor)
    if (cursor === null || dir === null) {
      this.onToast(cursor !== null && this._snapProjected ? arcOffPlaneMessage() : arcNoDirectionMessage())
      return
    }
    const endpoint = pointAlong(a, dir, distance)
    this._placePlaneB(endpoint, this._lastPlaneCursorStraightOver)
  }

  /** Bulge stage (plane mode): commit the arc with |sagitta| == `distance`,
   *  on the side resolved by `_resolvePlaneSagittaSign`. Refused (same hint
   *  as a flat pointer-commit) with no side resolvable, or a flat result. */
  private _commitTypedPlaneBulge(distance: number): void {
    if (this.planeStage.kind !== 'bulge') return
    const { plane, target, a, b } = this.planeStage
    const sign = this._resolvePlaneSagittaSign(plane, a, b)
    if (sign === null) {
      this.onMeasurementCb(flatBulgeHint())
      return
    }
    const chain = this._planeChain(plane, a, b, sign * distance)
    if (chain === null) {
      this.onMeasurementCb(flatBulgeHint())
      return
    }
    const committed = this._commitPlaneChain(target, chain, plane)
    this.planeStage = { kind: 'idle' }
    this.typed = ''
    this._lastPlaneCursor = null
    this._lastSagittaSign = null
    this._clearPreview()
    this.onMeasurementCb('')
    if (committed) {
      this.retype.arm({ mode: 'plane', plane, target, a, b, s: sign * distance, completion: this.completion, detached: this.sep.on })
    }
  }

  /** Chord stage (face): place B at `distance` from A along the live cursor
   *  direction, projected onto the locked face plane. */
  private _commitTypedFaceChord(distance: number): void {
    if (this.faceStage.kind !== 'chord') return
    if (distance < ARC_MIN_CHORD_M) return
    const cursor = this._lastFaceCursor
    const { a } = this.faceStage
    const dir = cursor === null ? null : directionBetween(a, cursor)
    if (cursor === null || dir === null) {
      this.onToast(arcNoDirectionMessage())
      return
    }
    const endpoint = pointAlong(a, dir, distance)
    this._placeFaceB(endpoint)
  }

  /** Bulge stage (face): commit the arc with |sagitta| == `distance` on the
   *  side resolved by `_resolveFaceSagittaSign`. */
  private _commitTypedFaceBulge(distance: number): void {
    if (this.faceStage.kind !== 'bulge') return
    const sign = this._resolveFaceSagittaSign()
    if (sign === null) {
      this.onMeasurementCb(flatBulgeHint())
      return
    }
    const { object, face, a, b, normal } = this.faceStage
    const basis = facePlaneBasis(normal)
    const verts = basis === null ? null : arcPolylineOnPlane(a, b, sign * distance, basis.u, basis.v)
    if (verts === null) {
      this.onMeasurementCb(flatBulgeHint())
      return
    }
    this.faceStage = { kind: 'idle' }
    this.typed = ''
    this._lastFaceCursor = null
    this._lastSagittaSign = null
    this._clearPreview()
    this.onMeasurementCb('')
    if (this._commitFace(object, face, normal, verts, sign * distance)) {
      this.retype.arm({ mode: 'face', object, face, normal, a, b, s: sign * distance, completion: this.completion, detached: this.sep.on })
    }
  }

  /**
   * Redraw the just-committed arc with |sagitta| = `d` on the side it was
   * drawn (the sign of the committed sagitta), same chord, same closure —
   * through the shared window, so the result is exactly what a third click
   * at that bulge would have produced.
   */
  private _retypeBulge(d: number): void {
    const spec = this.retype.spec
    if (spec === null) return
    if (d < ARC_MIN_SAGITTA_M) {
      this.onMeasurementCb(flatBulgeHint())
      return
    }
    const s = (spec.s < 0 ? -1 : 1) * d
    // Resolve the new geometry BEFORE touching history — a flat/invalid
    // bulge is refused up front rather than after an undo.
    if (this._hotGeometry(spec, s) === null) {
      this.onMeasurementCb(flatBulgeHint())
      return
    }
    const outcome = this.retype.apply(
      (hot) => this._commitHot(hot, s),
      (hot) => this._commitHot(hot, hot.s),
      (hot) => ({ ...hot, s }),
    )
    if (outcome === 'stale') this.onToast(retypeStaleMessage('arc'))
  }

  /** The chain (plane) or polyline (face) for the hot arc at sagitta `s`,
   *  under the closure it was drawn with; null when flat/invalid. */
  private _hotGeometry(hot: RetypeSpec, s: number): { chain: ArcChain } | { verts: V3[] } | null {
    const prev = this.completion
    this.completion = hot.completion
    try {
      if (hot.mode === 'plane') {
        const chain = this._planeChain(hot.plane, hot.a, hot.b, s)
        return chain === null ? null : { chain }
      }
      const basis = facePlaneBasis(hot.normal)
      const verts = basis === null ? null : arcPolylineOnPlane(hot.a, hot.b, s, basis.u, basis.v)
      return verts === null ? null : { verts }
    } finally {
      this.completion = prev
    }
  }

  /** Lay the hot arc down at sagitta `s` through its own commit path. */
  private _commitHot(hot: RetypeSpec, s: number): boolean {
    const geom = this._hotGeometry(hot, s)
    if (geom === null) return false
    const prev = this.completion
    this.completion = hot.completion
    try {
      return 'chain' in geom
        ? this._commitPlaneChain((hot as Extract<RetypeSpec, { mode: 'plane' }>).target, geom.chain, (hot as Extract<RetypeSpec, { mode: 'plane' }>).plane, hot.detached)
        : this._commitFace(
            (hot as Extract<RetypeSpec, { mode: 'face' }>).object,
            (hot as Extract<RetypeSpec, { mode: 'face' }>).face,
            hot.mode === 'face' ? hot.normal : hot.plane.normal,
            geom.verts,
            s,
            hot.detached,
          )
    } finally {
      this.completion = prev
    }
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
      this._trackIdleHover(snap, ray)
      this._clearPreview()
      // An open retype window owns the readout (its buffer, or the flat-bulge
      // hint a refused retype just showed): the key router's re-hover after
      // each captured key must not wipe it.
      if (this.typed === '' && !this.retype.isOpen) this.onMeasurementCb('')
      return
    }
    const { plane } = this.planeStage
    const cursor = this._planeCursor(snap, ray, plane)
    if (cursor === null) {
      this._clearPreview()
      // An open retype window owns the readout (its buffer, or the flat-bulge
      // hint a refused retype just showed): the key router's re-hover after
      // each captured key must not wipe it.
      if (this.typed === '' && !this.retype.isOpen) this.onMeasurementCb('')
      return
    }
    this._lastPlaneCursor = cursor
    this._lastPlaneCursorStraightOver = this._cursorStraightOver

    if (this.planeStage.kind === 'chord') {
      // Stage 2: rubber-band the chord A→cursor, report chord length.
      const { a } = this.planeStage
      this._clearPreview()
      this._drawSegments([a, cursor], false)
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
        this._drawSegments([a, b], false)
        this.onMeasurementCb(this._measurementText(''))
        return
      }
      this._clearPreview()
      this._drawSegments(verts, false)
      this._reportRadius(a2, b2, s as number)
      return
    }

    const s = this._faceChordSagitta(a, b, plane.normal, cursor)
    if (s !== null && Math.abs(s) >= ARC_MIN_SAGITTA_M) this._lastSagittaSign = Math.sign(s) as -1 | 1
    const verts = this._facePolyline(a, b, plane.normal, cursor)
    if (verts === null || s === null) {
      this._clearPreview()
      this._drawSegments([a, b], false)
      this.onMeasurementCb(this._measurementText(''))
      return
    }
    this._clearPreview()
    this._drawSegments(verts.concat(this._closingVerts(verts, this._faceCenter(a, b, plane.normal, s))), false)
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
      // The unlocked resolution at this SAME click (design §2a) — reuse it
      // for free when no lock was active; fall back to ground WITHOUT
      // probing when one was (see RectangleTool's identical comment).
      // A Shift pin's first click resolves to the pinned plane itself, which
      // is exactly what a later arrow-lock release should revert to.
      const natural = this.idlePlaneLock !== null
        ? groundNaturalTarget(this._editContext, a)
        : resolved
      this.planeStage = { kind: 'chord', plane, target, a, natural }
      this.typed = ''
      this._lastPlaneCursor = null
      return
    }

    const { plane } = this.planeStage
    const cursor = this._planeCursor(snap, ray, plane)
    if (cursor === null) return

    if (this.planeStage.kind === 'chord') {
      // Second click: endpoint B (the chord). Ignore a degenerate chord.
      // A free pick places B at the DISPLAYED chord length, as if typed.
      const shown = displayedLength(segmentLength(this.planeStage.a, cursor))
      if (isFreePick(snap, this.planeStage.a) && shown >= ARC_MIN_CHORD_M) {
        this._lastPlaneCursor = cursor
        this._lastPlaneCursorStraightOver = this._cursorStraightOver
        this._commitTypedPlaneChord(shown)
        return
      }
      // A snap dropped onto the plane right at A (straight over it, on a
      // plane that holds) is refused aloud, not dropped as a zero chord.
      if (this._snapProjected && segmentLength(this.planeStage.a, cursor) < ARC_MIN_CHORD_M) {
        this.onToast(arcOffPlaneMessage())
        return
      }
      this._placePlaneB(cursor, this._cursorStraightOver)
      return
    }

    // Third click: commit at the cursor's sagitta. Refuse a flat bulge.
    const { target, a, b } = this.planeStage
    const s = plane.ground
      ? chordSagitta([a[0], a[1]], [b[0], b[1]], [cursor[0], cursor[1]])
      : this._faceChordSagitta(a, b, plane.normal, cursor)
    const chain = s === null ? null : this._planeChain(plane, a, b, s)
    if (chain === null || s === null) {
      this.onMeasurementCb(flatBulgeHint())
      return
    }
    // A free pick commits the sagitta of the DISPLAYED radius, as if typed.
    if (isFreePick(snap)) {
      const sd = sagittaForDisplayedRadius(segmentLength(a, b), Math.abs(s), Math.abs(s) > segmentLength(a, b) / 2)
      if (sd !== null && sd >= ARC_MIN_SAGITTA_M) {
        this._lastPlaneCursor = cursor
        this._commitTypedPlaneBulge(sd)
        return
      }
    }

    const committed = this._commitPlaneChain(target, chain, plane)
    this.planeStage = { kind: 'idle' }
    this.typed = ''
    this._lastPlaneCursor = null
    this._lastSagittaSign = null
    this._clearPreview()
    this.onMeasurementCb('')
    if (committed) this.retype.arm({ mode: 'plane', plane, target, a, b, s, completion: this.completion, detached: this.sep.on })
  }

  /** Place chord endpoint B (plane mode) — shared by the pointer-click and
   *  typed (`_commitTypedPlaneChord`) paths. Ignores a degenerate
   *  (sub-ARC_MIN_CHORD_M) chord. `straightOver`: B came from a cursor
   *  `_planeCursor` kept straight over a ground A (`_isStraightOverA`) —
   *  the only case that re-homes. Any other B stays on the frozen plane,
   *  even one a hair off it (`cursorOnFrozenPlane` passes a snap within its
   *  own tolerance through unprojected). */
  private _placePlaneB(b: V3, straightOver: boolean): void {
    if (this.planeStage.kind !== 'chord') return
    const { plane, target, a, natural, fromFace } = this.planeStage
    if (segmentLength(a, b) < ARC_MIN_CHORD_M) return
    // B straight over a ground A (`_planeCursor`) has left the ground: the
    // bulge continues on a vertical plane through the chord.
    if (straightOver) {
      const rehomed = this._rehomedChordPlane(a, b)
      this.planeStage = {
        kind: 'bulge', plane: rehomed.plane, target: rehomed.target, a, b, natural, fromFace,
        rehomedFrom: { plane, target },
      }
    } else {
      this.planeStage = { kind: 'bulge', plane, target, a, b, natural, fromFace }
    }
    this.typed = ''
    this._lastPlaneCursor = null
    this._lastSagittaSign = null
    this._clearPreview()
    this.onMeasurementCb('')
  }

  /** The plane a chord that left its frozen plane moves onto (only a chord
   *  straight off the ground reaches here — see `_planeCursor`): through A,
   *  holding the chord, turned toward the view — the plane LineTool's
   *  `_rehomeChain` picks for a first segment (`rehomePlaneNormal` with no
   *  previous direction). */
  private _rehomedChordPlane(a: V3, b: V3): { plane: DrawPlane; target: SketchTarget } {
    const dir = directionBetween(a, b) ?? [0, 0, 1]
    const normal = rehomePlaneNormal(dir, null, this._lastViewDir ?? [0, 0, -1])
    const { u, v } = facePlaneBasis(normal) ?? { u: [1, 0, 0] as V3, v: [0, 1, 0] as V3 }
    const plane: DrawPlane = { origin: a, normal, u, v, ground: isGroundPlane(a, normal) }
    return { plane, target: { kind: 'plane', plane, instance: this._activeInstance } }
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

  /** Commit the polyline chain as ONE draft on `target`'s plane: the first
   *  `curveSegments` segments are bracketed as ONE curve chain (the arc), so
   *  clicking any facet later selects the whole arc; pie/segment closing
   *  edges stay plain lines. When the arc's analytic circle is known it
   *  rides on the chain (durable center/radius — the true-curves design).
   *  Used by plane mode — ground AND any other plane (sketch mode); real
   *  face mode commits through `_commitFace`. */
  private _commitPlaneChain(target: SketchTarget, chain: ArcChain, plane: DrawPlane, detached = this.sep.on): boolean {
    return this._runArcDraft(draftTargetFromSketchTarget(target, detached, plane), chain)
  }

  private _runArcDraft(
    target: Parameters<typeof runDraft>[1],
    { chain: verts, curveSegments, geom }: ArcChain,
  ): boolean {
    try {
      const { result: regionsCreated, report } = runDraft(this.wasmScene, target, t`Arc`, (d) => {
        let last: bigint[] = []
        // A chain with no analytic circle (a degenerate, straight arc) still
        // commits as ONE curve chain, bracketed without a circle.
        if (geom !== null) {
          this.wasmScene.draft_begin_curve_with(d, new Float64Array(geom.center), geom.radius)
        } else {
          this.wasmScene.draft_begin_curve(d)
        }
        let open = true
        try {
          for (let i = 0; i < verts.length - 1; i++) {
            if (open && i === curveSegments) {
              this.wasmScene.draft_end_curve(d)
              open = false
            }
            last = addDraftSegment(this.wasmScene, d, verts[i], verts[i + 1])
          }
        } finally {
          if (open) this.wasmScene.draft_end_curve(d)
        }
        return last
      })
      refreshAfterDraft(report, {
        onObject: (o) => this.onFaceImprint(o),
        onSketch: (sketchHandle) => this.onCommit({ sketchHandle, regionsCreated }),
      })
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

  /**
   * The cursor's position on the locked face plane: the SNAPPED point when
   * one is available AND it actually lies on this plane — `snapConstraint`
   * already holds every face-mode snap to this plane, so an `Endpoint`/
   * `Midpoint`/`On Edge` chip is honoured exactly, the next point landing ON
   * the snapped point rather than a sub-pixel off it — else the raw
   * ray∩plane intersection (nothing snapped: past the face's extent, or no
   * kernel candidate). A snap that somehow sits off the plane (never, by the
   * constraint's construction — a defensive check only) falls back to the
   * ray too, so the arc can never be handed an off-plane point. Before the
   * on-plane check, this unconditionally trusted ANY snap (even one from a
   * different, coincidentally-hit surface) instead of verifying it against
   * the plane the gesture is actually locked to.
   */
  private _faceCursor(snap: Snap | null, ray: Ray): V3 | null {
    if (this.faceStage.kind === 'idle') return null
    const { planePoint, normal } = this.faceStage
    const at = cursorOnFrozenPlane(snap, ray, planePoint, normal)
    this._snapProjected = at?.projected ?? false
    return at?.point ?? null
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
      this._trackIdleHover(snap, ray)
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
      this._drawSegments([a, cursor], true)
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
      this._drawSegments([a, b], true)
      this.onMeasurementCb(this._measurementText(''))
      return
    }
    // Preview the arc plus any completion-mode closing edges; the radius
    // readout still derives from the bare arc vertices.
    this._drawSegments(verts.concat(this._closingVerts(verts, this._faceCenter(a, b, normal, s))), true)
    this._reportRadiusFromChain(verts)
  }

  private _onPointerDownFace(snap: Snap | null, ray: Ray): void {
    if (this.faceStage.kind === 'idle') {
      // First click: anchor endpoint A on the eligible face under the cursor.
      if (snap === null) return

      const eligible = this._facePickAt(snap, ray)
      if (eligible === null) return

      const { normal } = eligible
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
      // Second click: endpoint B. Ignore a degenerate chord. A free pick
      // places B at the DISPLAYED chord length, as if typed.
      const shown = displayedLength(segmentLength(this.faceStage.a, cursor))
      if (isFreePick(snap, this.faceStage.a) && shown >= ARC_MIN_CHORD_M) {
        this._lastFaceCursor = cursor
        this._commitTypedFaceChord(shown)
        return
      }
      this._placeFaceB(cursor)
      return
    }

    // Third click: commit the face cut. Refuse a flat bulge.
    if (this.faceStage.kind !== 'bulge') return // (narrowing is lost across the _faceCursor call)
    const { object, face, a, b, normal } = this.faceStage
    const s = this._faceChordSagitta(a, b, normal, cursor)
    const verts = s === null ? null : this._facePolyline(a, b, normal, cursor)
    if (verts === null || s === null) {
      this.onMeasurementCb(flatBulgeHint())
      return
    }
    // A free pick commits the sagitta of the DISPLAYED radius, as if typed.
    if (isFreePick(snap)) {
      const c = segmentLength(a, b)
      const sd = sagittaForDisplayedRadius(c, Math.abs(s), Math.abs(s) > c / 2)
      if (sd !== null && sd >= ARC_MIN_SAGITTA_M) {
        this._lastFaceCursor = cursor
        this._commitTypedFaceBulge(sd)
        return
      }
    }

    this.faceStage = { kind: 'idle' }
    this.typed = ''
    this._lastFaceCursor = null
    this._lastSagittaSign = null
    this._clearPreview()
    this.onMeasurementCb('')
    if (this._commitFace(object, face, normal, verts, s)) {
      this.retype.arm({ mode: 'face', object, face, normal, a, b, s, completion: this.completion, detached: this.sep.on })
    }
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

  /** Commit the arc on the face as ONE draft, for the current completion
   * mode: an open arc is just its polyline; pie/segment add their closing
   * vertices (a pie's center, or the chord back to the start). The arc's own
   * analytic circle (center via `_faceCenter`, radius the distance from that
   * center to `verts[0]` — mirrors `_nonGroundChain`) rides the leading
   * `verts.length - 1` edges; closing edges stay plain lines. The engine
   * lowers whatever crosses the face's edges or other drawing — an open arc
   * need not reach an edge, and one that does simply splits the face. A pie
   * whose center is unresolvable (degenerate basis — cannot happen when
   * `verts` built) falls back to the open arc. When the center can't be
   * resolved there is no analytic circle to carry. */
  private _commitFace(object: bigint, face: bigint, normal: V3, verts: V3[], s: number, detached = this.sep.on): boolean {
    const center = this._faceCenter(verts[0], verts[verts.length - 1], normal, s)
    const arcSegments = verts.length - 1
    const chain = verts.concat(this._closingVerts(verts, center))
    return this._runArcDraft(
      { kind: 'face', object, face, instance: this._activeInstance, ...(detached ? { detached: true as const } : {}) },
      {
        chain,
        curveSegments: arcSegments,
        geom: center === null ? null : { center, radius: segmentLength(center, verts[0]) },
      },
    )
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
    this.onMeasurementCb(this._measurementText(radiusReadout(formatLength(arc.radius))))
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
    this.onMeasurementCb(this._measurementText(radiusReadout(formatLength(radius))))
  }

  // ------------------------------------------------------------------ preview

  /**
   * Emit a fat-line preview for an open polyline. Vertices are used exactly
   * as given — the preview's depth bias (PREVIEW_LINE_STYLE, depthPolicy.ts)
   * settles coincidence with the ground/committed lines, so no z-lift.
   *
   * @param verts  Ordered world-space vertices (>= 2).
   */
  private _drawSegments(verts: V3[], joinsFace: boolean): void {
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
