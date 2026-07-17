/**
 * ArcTool — SketchUp-style 2-point arc, drawn as a faceted polyline chain
 *. Mirrors CircleTool: no kernel change, the arc decomposes into N
 * chained `sketch_add_segment` calls (ground mode) or one `split_face` call
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
 * entered object only, inside a context), ground mode otherwise. All three
 * points lie in the sketch plane — Z=0, or the picked face's plane via
 * `snapConstraint`.
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
 * so a region (ground) or a face imprint (face mode, via `split_face_inner`
 * like CircleTool) appears immediately and push/pull works. The live preview
 * draws the closing edges and the radius readout names the mode. The mode
 * persists across commits until the tool is switched (tools are recreated on
 * every switch, so a fresh Arc tool always starts `open`) or the document is
 * replaced (`onDocumentReset`).
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { V3 } from '../viewport/geoHelpers'
import { facePlaneBasis } from '../viewport/geoHelpers'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { makeFatSegments, disposeFatSegments, PREVIEW_LINE_STYLE } from '../viewport/fatLine'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { segmentLength, directionBetween } from './lineInput'
import { editLengthBuffer, isLengthInputKey, pointAlong } from './moveInput'
import { runSketchGesture, makeSketchHandleCache, type SketchHandleCache } from './sketchGesture'
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

/** Ground gesture: idle → endpoint A placed → chord (A,B) placed. */
type GroundStage =
  | { kind: 'idle' }
  | { kind: 'chord'; a: [number, number] }
  | { kind: 'bulge'; a: [number, number]; b: [number, number] }

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

/**
 * Intersect a ray with an arbitrary plane defined by a point and unit normal.
 * Returns the intersection point, or null if the ray is nearly parallel to
 * the plane (|dot(dir, normal)| < 1e-10).
 */
function intersectPlane(
  rayOrigin: [number, number, number],
  rayDir: [number, number, number],
  planePoint: V3,
  normal: V3,
): V3 | null {
  const denom = rayDir[0] * normal[0] + rayDir[1] * normal[1] + rayDir[2] * normal[2]
  if (Math.abs(denom) < 1e-10) return null
  const wx = planePoint[0] - rayOrigin[0]
  const wy = planePoint[1] - rayOrigin[1]
  const wz = planePoint[2] - rayOrigin[2]
  const t = (wx * normal[0] + wy * normal[1] + wz * normal[2]) / denom
  if (t < 0) return null
  return [
    rayOrigin[0] + t * rayDir[0],
    rayOrigin[1] + t * rayDir[1],
    rayOrigin[2] + t * rayDir[2],
  ]
}

export class ArcTool implements Tool {
  readonly name = 'Arc'

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    const stage = this.faceStage.kind !== 'idle' ? this.faceStage.kind : this.groundStage.kind
    if (stage === 'chord') {
      return "Click the arc's second endpoint — or type an exact chord length."
    }
    if (stage === 'bulge') {
      return 'Click to set the curve — Alt cycles open arc / pie / segment; or type an exact bulge.'
    }
    return "Click the arc's first endpoint — on the ground plane or any face."
  }

  private groundStage: GroundStage = { kind: 'idle' }
  private faceStage: FaceStage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnArcCommit
  private onFaceImprint: OnFaceImprint
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** Cached ground-sketch handle — the Viewport passes one cache shared by
   *  every draw tool, so mixed-tool profiles land in a single sketch. */
  private readonly sketchCache: SketchHandleCache

  /** The currently active editing context (entered object), or null at top level. */
  private _activeContext: bigint | null = null

  /** Completion mode for committed arcs — Alt cycles it mid-gesture and it
   *  persists across commits (see module doc). */
  private completion: ArcCompletion = 'open'

  /** VCB buffer — raw string being typed by the user (length, in display units). */
  private typed: string = ''

  /** Last live cursor position seen this stage (ground/face — only one is
   *  ever populated at a time, mirroring LineTool's pair of fields). Used
   *  for the typed-commit chord direction and the bulge-side resolution. */
  private _lastGroundCursor: [number, number] | null = null
  private _lastFaceCursor: V3 | null = null

  /** The last NONZERO sagitta sign seen during the current bulge stage —
   *  the fallback when a typed bulge commit's live cursor sits exactly on
   *  the chord (see module doc's VCB section). Reset every time a fresh
   *  bulge stage begins (or the gesture ends). */
  private _lastSagittaSign: -1 | 1 | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnArcCommit,
    onToast: OnToast,
    onFaceImprint: OnFaceImprint,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
    sketchCache: SketchHandleCache = makeSketchHandleCache(),
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
   * Decide which mode governs the NEXT pointer event (same contract as the
   * other draw tools): sticky mid-gesture; always face mode inside an
   * entered object context (scoped drawing — no top-level ground sketch
   * from inside); else decided by what's under the cursor.
   */
  private _currentMode(ray?: Ray): 'face' | 'ground' {
    if (this.faceStage.kind !== 'idle') return 'face'
    if (this.groundStage.kind !== 'idle') return 'ground'
    // Inside an entered object context, drawing stays scoped to that
    // object's faces — a click elsewhere is ignored by the face handler
    // rather than falling through to a top-level ground sketch.
    if (this._activeContext !== null) return 'face'
    if (ray === undefined) return 'ground'
    return this._eligiblePickFor(ray) !== null ? 'face' : 'ground'
  }

  /**
   * Provide a constraint plane for snap so off-plane/occluded geometry is
   * excluded while snapping during face-mode drawing — identical to
   * CircleTool's policy:
   *
   * - Face mode, gesture in progress: return the already-locked face plane.
   * - Idle: pick the hovered face (if an eligible one is under the cursor)
   *   and return its plane so the FIRST-click endpoint lands precisely on
   *   the face.
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

    if (this.groundStage.kind !== 'idle') {
      // Mid ground gesture — no constraint
      return null
    }

    // Idle: face mode iff an eligible face is under the cursor
    const eligible = this._eligiblePickFor(ray)
    if (eligible === null) return null

    const a = this.wasmScene.face_plane(eligible.object, eligible.face)
    return {
      constraintPlane: {
        point: [a[0], a[1], a[2]],
        normal: [a[3], a[4], a[5]],
      },
    }
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (this._currentMode(ray) === 'face') {
      this._onPointerMoveFace(snap, ray)
    } else {
      this._onPointerMoveGround(snap)
    }
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (this._currentMode(ray) === 'face') {
      this._onPointerDownFace(snap, ray)
    } else {
      this._onPointerDownGround(snap)
    }
  }

  /**
   * True while a gesture is in progress, so the Viewport routes keys here
   * (Escape stage-back, and — since the — typed VCB entry) instead
   * of treating letters as tool-switch shortcuts mid-gesture.
   */
  capturingInput(): boolean {
    return this.groundStage.kind !== 'idle' || this.faceStage.kind !== 'idle'
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this._stepBack()
      return
    }

    if (!this.capturingInput()) return

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
    if (this.groundStage.kind === 'bulge') {
      this.groundStage = { kind: 'chord', a: this.groundStage.a }
      this.typed = ''
      this._lastGroundCursor = null
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
    this.groundStage = { kind: 'idle' }
    this.faceStage = { kind: 'idle' }
    this.typed = ''
    this._lastGroundCursor = null
    this._lastFaceCursor = null
    this._lastSagittaSign = null
    this._clearPreview()
    this.onMeasurementCb('')
  }

  /**
   * Enter with a non-empty typed buffer: resolved per-stage (see module doc).
   */
  private _commitTyped(distance: number): void {
    if (this.groundStage.kind === 'chord') {
      this._commitTypedGroundChord(distance)
    } else if (this.groundStage.kind === 'bulge') {
      this._commitTypedGroundBulge(distance)
    } else if (this.faceStage.kind === 'chord') {
      this._commitTypedFaceChord(distance)
    } else if (this.faceStage.kind === 'bulge') {
      this._commitTypedFaceBulge(distance)
    }
  }

  /** Chord stage (ground): place B at `distance` from A along the live
   *  cursor direction. Refused (buffer kept) with no direction yet, or a
   *  sub-ARC_MIN_CHORD_M distance. */
  private _commitTypedGroundChord(distance: number): void {
    if (this.groundStage.kind !== 'chord') return
    if (distance < ARC_MIN_CHORD_M) return
    const cursor = this._lastGroundCursor
    if (cursor === null) return
    const { a } = this.groundStage
    const a3: V3 = [a[0], a[1], 0]
    const dir = directionBetween(a3, [cursor[0], cursor[1], 0])
    if (dir === null) return
    const endpoint = pointAlong(a3, dir, distance)
    this._placeGroundB([endpoint[0], endpoint[1]])
  }

  /** Bulge stage (ground): commit the arc with |sagitta| == `distance`, on
   *  the side resolved by `_resolveGroundSagittaSign`. Refused (same hint as
   *  a flat pointer-commit) with no side resolvable, or a flat result. */
  private _commitTypedGroundBulge(distance: number): void {
    if (this.groundStage.kind !== 'bulge') return
    const sign = this._resolveGroundSagittaSign()
    if (sign === null) {
      this.onMeasurementCb(FLAT_BULGE_HINT)
      return
    }
    const { a, b } = this.groundStage
    const chain = this._groundChain(a, b, sign * distance)
    if (chain === null) {
      this.onMeasurementCb(FLAT_BULGE_HINT)
      return
    }
    this._commitGroundChain(chain.chain, chain.curveSegments, chain.geom)
    this.groundStage = { kind: 'idle' }
    this.typed = ''
    this._lastGroundCursor = null
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
   * Resolve the bulge side for a typed ground commit: the sign of the live
   * cursor's sagitta if it's non-negligible, else the last nonzero side seen
   * this bulge stage, else null (refuse — see module doc).
   */
  private _resolveGroundSagittaSign(): -1 | 1 | null {
    if (this.groundStage.kind !== 'bulge') return null
    const { a, b } = this.groundStage
    const cursor = this._lastGroundCursor
    if (cursor !== null) {
      const s = chordSagitta(a, b, cursor)
      if (s !== null && Math.abs(s) >= ARC_MIN_SAGITTA_M) return Math.sign(s) as -1 | 1
    }
    return this._lastSagittaSign
  }

  /** Face-mode counterpart of `_resolveGroundSagittaSign`. */
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

  /**
   * A new/loaded document replaced the Scene, so the cached ground-sketch
   * handle is now stale (reusing it throws UnknownSketch). Drop it and reset.
   * Called by the Viewport from `notifyLoaded`.
   */
  onDocumentReset(): void {
    this.sketchCache.set(null)
    this.completion = 'open' // a fresh document starts with the default close
    this.cancel()
  }

  // ------------------------------------------------------------------ ground mode

  private _onPointerMoveGround(snap: Snap | null): void {
    if (snap === null || this.groundStage.kind === 'idle') {
      this._clearPreview()
      if (this.typed === '') this.onMeasurementCb('')
      return
    }

    if (this.groundStage.kind === 'chord') {
      // Stage 2: rubber-band the chord A→cursor, report chord length.
      const { a } = this.groundStage
      const cursor: [number, number] = [snap.x, snap.y]
      this._lastGroundCursor = cursor
      this._clearPreview()
      this._drawSegments([[a[0], a[1], 0], [cursor[0], cursor[1], 0]])
      this.onMeasurementCb(this._measurementText(formatLength(Math.hypot(cursor[0] - a[0], cursor[1] - a[1]))))
      return
    }

    // Stage 3: rubber-band the faceted arc (plus any completion-mode closing
    // edges) through the cursor's bulge side.
    const { a, b } = this.groundStage
    const cursor: [number, number] = [snap.x, snap.y]
    this._lastGroundCursor = cursor
    const s = chordSagitta(a, b, cursor)
    if (s !== null && Math.abs(s) >= ARC_MIN_SAGITTA_M) this._lastSagittaSign = Math.sign(s) as -1 | 1
    const verts = s === null ? null : (this._groundChain(a, b, s)?.chain ?? null)
    if (verts === null) {
      // Flat bulge — fall back to showing the bare chord.
      this._clearPreview()
      this._drawSegments([[a[0], a[1], 0], [b[0], b[1], 0]])
      this.onMeasurementCb(this._measurementText(''))
      return
    }
    this._clearPreview()
    this._drawSegments(verts)
    this._reportRadius(a, b, s as number)
  }

  private _onPointerDownGround(snap: Snap | null): void {
    if (snap === null) return

    if (this.groundStage.kind === 'idle') {
      // First click: endpoint A
      this.groundStage = { kind: 'chord', a: [snap.x, snap.y] }
      this.typed = ''
      this._lastGroundCursor = null
      return
    }

    if (this.groundStage.kind === 'chord') {
      // Second click: endpoint B (the chord). Ignore a degenerate chord.
      const b: [number, number] = [snap.x, snap.y]
      this._placeGroundB(b)
      return
    }

    // Third click: commit at the cursor's sagitta. Refuse a flat bulge.
    const { a, b } = this.groundStage
    const s = chordSagitta(a, b, [snap.x, snap.y])
    const chain = s === null ? null : this._groundChain(a, b, s)
    if (chain === null) {
      this.onMeasurementCb(FLAT_BULGE_HINT)
      return
    }

    this._commitGroundChain(chain.chain, chain.curveSegments, chain.geom)
    this.groundStage = { kind: 'idle' }
    this.typed = ''
    this._lastGroundCursor = null
    this._lastSagittaSign = null
    this._clearPreview()
    this.onMeasurementCb('')
  }

  /** Place chord endpoint B (ground) — shared by the pointer-click and typed
   *  (`_commitTypedGroundChord`) paths. Ignores a degenerate (sub-ARC_MIN_CHORD_M)
   *  chord. */
  private _placeGroundB(b: [number, number]): void {
    if (this.groundStage.kind !== 'chord') return
    const { a } = this.groundStage
    if (Math.hypot(b[0] - a[0], b[1] - a[1]) < ARC_MIN_CHORD_M) return
    this.groundStage = { kind: 'bulge', a, b }
    this.typed = ''
    this._lastGroundCursor = null
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

  /** 3D arc center for the face chord a→b with sagitta s, lifted through the
   *  face's in-plane basis, or null on degenerate input. */
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
  private _groundChain(
    a: Vec2,
    b: Vec2,
    s: number,
  ): { chain: V3[]; curveSegments: number; geom: { center: V3; radius: number } | null } | null {
    const verts = this._groundPolyline(a, b, s)
    if (verts === null) return null
    const arc = arcFromChord(a, b, s)
    return {
      chain: verts.concat(this._closingVerts(verts, this._groundCenter(a, b, s))),
      curveSegments: verts.length - 1,
      geom: arc === null ? null : { center: [arc.center[0], arc.center[1], 0], radius: arc.radius },
    }
  }

  /** Commit the polyline chain as N ground-sketch segments; the first
   *  `curveSegments` of them are bracketed as ONE curve chain (the arc), so
   *  clicking any facet later selects the whole arc. When the arc's analytic
   *  circle is known it rides on the chain (durable center/radius —
   *  the true-curves design). */
  private _commitGroundChain(
    verts: V3[],
    curveSegments: number,
    geom: { center: V3; radius: number } | null = null,
  ): void {
    try {
      runSketchGesture(this.wasmScene, this.sketchCache, (sketch) => {
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
    return intersectPlane(ray.origin, ray.direction, planePoint, normal)
  }

  /** In-plane (u,v) signed sagitta of `cursor` relative to the face chord
   * a→b (the shared projection step `_facePolyline` and the bulge-side
   * resolution both need). Returns null when the face has no valid in-plane
   * basis, or the chord is degenerate in-plane. */
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
   *  suffixes the completion mode). */
  private _reportRadius(a: Vec2, b: Vec2, s: number): void {
    const arc = arcFromChord(a, b, s)
    if (arc === null) {
      this.onMeasurementCb(this._measurementText(''))
      return
    }
    this.onMeasurementCb(this._measurementText(`R ${formatLength(arc.radius)}`))
  }

  /** Report the radius from an already-built polyline (face mode): the
   * distance from any interior vertex to the chord endpoints determines the
   * circle, but it's simplest to recompute from the sagitta implied by the
   * mid vertex. */
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
