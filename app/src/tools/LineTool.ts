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
 * Ground mode (no eligible face under the cursor):
 *   1. First click: anchor the first point (snapped, on Z=0).
 *   2. Each subsequent click commits ONE segment from the previous point to
 *      the new point: lazily begin_ground_sketch() if no sketch handle yet,
 *      then sketch_add_segment(sketch, prev.., cur..). The new point becomes
 *      the anchor for the next segment (chain forward).
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
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { makeFatSegments, disposeFatSegments, PREVIEW_LINE_STYLE } from '../viewport/fatLine'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { arrowToAxis, editLengthBuffer, isLengthInputKey, pointAlong } from './moveInput'
import { segmentLength, directionBetween } from './lineInput'
import { runSketchGesture, makeSketchHandleCache, type SketchHandleCache } from './sketchGesture'
import { FacePickCache, defaultFaceEligible, type FaceEligible } from './faceDraw'

export type OnLineCommit = (sketchHandle: bigint) => void
export type OnFaceImprint = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/** Ground chain: idle, or anchored with the last placed point. */
type GroundStage =
  | { kind: 'idle' }
  | { kind: 'anchored'; anchor: [number, number] }

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


/**
 * Intersect a ray with an arbitrary plane defined by a point and unit normal.
 * Returns the intersection point, or null if the ray is nearly parallel to
 * the plane (|dot(dir, normal)| < 1e-10) or the intersection is behind the
 * ray origin.
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

export class LineTool implements Tool {
  readonly name = 'Line'

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    return this.groundStage.kind === 'idle' && this.faceStage.kind === 'idle'
      ? 'Click to start a line — on the ground plane or any face.'
      : 'Click the next point — type a length for an exact segment; double-click or Esc to finish.'
  }

  private groundStage: GroundStage = { kind: 'idle' }
  private faceStage: FaceStage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnLineCommit
  private onFaceImprint: OnFaceImprint
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** Cached ground-sketch handle — the Viewport passes one cache shared by
   *  every draw tool, so mixed-tool profiles land in a single sketch. */
  private readonly sketchCache: SketchHandleCache

  /** The currently active editing context (entered object), or null at top level. */
  private _activeContext: bigint | null = null

  /** VCB buffer — raw string being typed by the user (length, in display units) */
  private typed: string = ''

  /** Last rubber-band cursor positions, tracked for typed-entry direction */
  private _lastGroundCursor: [number, number] | null = null
  private _lastFaceCursor: V3 | null = null


  /** Current axis lock: 0=X, 1=Y, 2=Z, null=free. Mirrors MoveTool. */
  private lockAxis: 0 | 1 | 2 | null = null
  /** True when the *current* axis lock was set by holding Shift (vs. an arrow). */
  private shiftAxisLock: boolean = false

  /** Per-pointer-event `pick_face` memo — see `FacePickCache` in faceDraw.ts. */
  private readonly _pickCache = new FacePickCache()

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

  /**
   * Provide snap constraints: a constraint plane while drawing on a face
   * (so off-plane/occluded geometry is excluded), and/or an axis lock
   * (arrow keys / Shift, mirroring MoveTool) once a chain is anchored.
   *
   * - Anchored (ground or face): always include `anchor` (the last placed
   *   point) — the inference engine derives anchor-dependent candidates
   *   from it (a Tangent snap is "the rim point where the segment from the
   *   anchor touches the circle", the true-curves design). With an
   *   axis locked, additionally include `lockAxis` so the snap collapses
   *   onto the locked line.
   * - Face-anchored: ALSO return the known face plane's `constraintPlane`
   *   (unconditionally — independent of any axis lock) so subsequent snaps
   *   stay on that plane.
   * - Idle: pick the hovered face (any eligible Object, scoped by
   *   `_activeContext` exactly like PushPullTool) and return its plane so
   *   the FIRST-click point lands precisely on the face.
   * - No eligible face under the cursor and nothing anchored: return null
   *   (ground, unconstrained).
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

    if (this.groundStage.kind === 'anchored') {
      // No face plane while ground-anchored; just the axis lock (if any).
      return Object.keys(lockPart).length > 0 ? lockPart : null
    }

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

  /** The last placed point of whichever chain is currently anchored, or null. */
  private _currentAnchor(): [number, number, number] | null {
    if (this.faceStage.kind === 'anchored') {
      const { points } = this.faceStage
      return points[points.length - 1]
    }
    if (this.groundStage.kind === 'anchored') {
      const { anchor } = this.groundStage
      return [anchor[0], anchor[1], 0]
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

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (this._currentMode(ray) === 'face') {
      this._onPointerMoveFace(snap, ray)
    } else {
      this._onPointerMoveGround(snap)
    }
  }

  /**
   * Decide which mode governs the NEXT pointer event (same contract as
   * Rectangle/Circle/Arc):
   *   - Already anchored in one mode: stick with it (mid-chain).
   *   - Inside an entered object context: always face mode — drawing stays
   *     scoped to that object's faces, so a click elsewhere is ignored by
   *     the face handler rather than falling through to a TOP-LEVEL ground
   *     sketch from inside the context.
   *   - Otherwise idle at top level: face mode if an eligible Object face is
   *     under the cursor (via `pick_face`), else ground mode.
   */
  private _currentMode(ray?: Ray): 'face' | 'ground' {
    if (this.faceStage.kind === 'anchored') return 'face'
    if (this.groundStage.kind === 'anchored') return 'ground'
    if (this._activeContext !== null) return 'face'
    if (ray === undefined) return 'ground'

    return this._eligiblePickFor(ray) !== null ? 'face' : 'ground'
  }

  private _onPointerMoveGround(snap: Snap | null): void {
    if (this.groundStage.kind !== 'anchored' || snap === null) {
      this._clearPreview()
      if (this.typed === '') this.onMeasurementCb('')
      return
    }
    const { anchor } = this.groundStage
    const cursor: [number, number] = [snap.x, snap.y]
    this._lastGroundCursor = cursor
    this._drawRubberBandGround(anchor, cursor)
    this._reportMeasurement([anchor[0], anchor[1], 0], [cursor[0], cursor[1], 0])
    this._publishTransient([cursor[0], cursor[1], 0])
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
    this._drawRubberBandFace(last, cursor)
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
    return intersectPlane(ray.origin, ray.direction, planePoint, normal)
  }

  /**
   * Publish the in-progress geometry as transient snap candidates so the
   * line being drawn can snap to its own just-placed points (Phase B).
   *
   * - Face mode: every consecutive pair of accumulated `points` (none of
   *   which touch the kernel sketch until `split_face` commits) plus a
   *   trailing rubber-band segment to `cursor` (if given).
   * - Ground mode: committed segments are already persistent via
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
    } else if (this.groundStage.kind === 'anchored' && cursor !== null) {
      const { anchor } = this.groundStage
      this._publishSegment([anchor[0], anchor[1], 0], cursor)
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
      this._onPointerDownGround(snap)
    }
  }

  /**
   * Typed VCB entry is available once the first point has been placed
   * (either ground or face mode) — mirrors RectangleTool.
   */
  capturingInput(): boolean {
    return this.groundStage.kind === 'anchored' || this.faceStage.kind === 'anchored'
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this._onEscape()
      return
    }

    if (!this.capturingInput()) return

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
      : (this._lastGroundCursor !== null ? [this._lastGroundCursor[0], this._lastGroundCursor[1], 0] as V3 : null)
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
    if (this.groundStage.kind === 'anchored' || this.faceStage.kind === 'anchored') {
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
    this.groundStage = { kind: 'idle' }
    this.faceStage = { kind: 'idle' }
    this.typed = ''
    this._lastGroundCursor = null
    this._lastFaceCursor = null
    this.lockAxis = null
    this.shiftAxisLock = false
    this._clearPreview()
    this.onMeasurementCb('')
    this.wasmScene.clear_transient_segments()
  }

  cancel(): void {
    this.groundStage = { kind: 'idle' }
    this.faceStage = { kind: 'idle' }
    this.typed = ''
    this._lastGroundCursor = null
    this._lastFaceCursor = null
    this.lockAxis = null
    this.shiftAxisLock = false
    this._clearPreview()
    this.onMeasurementCb('')
    this.wasmScene.clear_transient_segments()
  }

  /**
   * A new/loaded document replaced the Scene, so any cached ground-sketch
   * handle is now stale (reusing it throws UnknownSketch). Drop it and reset
   * to idle. The Viewport calls this from `notifyLoaded`. Re-pressing the
   * Line shortcut while Line is already active does NOT recreate the tool, so
   * this hook — not tool re-instantiation — is what clears the stale handle.
   */
  onDocumentReset(): void {
    this.sketchCache.set(null)
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
    if (this.groundStage.kind === 'anchored') {
      const { anchor } = this.groundStage
      const last: V3 = [anchor[0], anchor[1], 0]
      const cursor2 = this._lastGroundCursor
      const cursor: V3 = cursor2 !== null ? [cursor2[0], cursor2[1], 0] : last
      const dir = directionBetween(last, cursor)
      if (dir === null) return
      const endpoint = pointAlong(last, dir, distance)
      this._commitGroundSegment(anchor, [endpoint[0], endpoint[1]])
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

  // ------------------------------------------------------------------ ground mode

  private _onPointerDownGround(snap: Snap | null): void {
    if (snap === null) return
    const cursor: [number, number] = [snap.x, snap.y]

    if (this.groundStage.kind === 'idle') {
      // First click: set anchor — no segment to commit yet.
      this.groundStage = { kind: 'anchored', anchor: cursor }
      this._lastGroundCursor = null
      this.typed = ''
      this.onMeasurementCb('')
      this._publishTransient(null)
    } else {
      const { anchor } = this.groundStage
      this._commitGroundSegment(anchor, cursor)
    }
  }

  /** Commit one segment anchor -> cursor, then chain forward from cursor. */
  private _commitGroundSegment(anchor: [number, number], cursor: [number, number]): void {
    // Skip degenerate zero-length segments.
    if (
      Math.abs(anchor[0] - cursor[0]) < 1e-8 &&
      Math.abs(anchor[1] - cursor[1]) < 1e-8
    ) {
      return
    }

    try {
      // Each committed segment is its own gesture — one Cmd+Z undoes exactly
      // that segment, matching LineTool's chain-forward-per-click semantics.
      runSketchGesture(this.wasmScene, this.sketchCache, (sketch) => {
        const report = this.wasmScene.sketch_add_segment(
          sketch,
          anchor[0], anchor[1], 0,
          cursor[0], cursor[1], 0,
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
          this.groundStage = { kind: 'idle' }
          this.typed = ''
          this._lastGroundCursor = null
          this.lockAxis = null
          this.shiftAxisLock = false
          this._clearPreview()
          this.onMeasurementCb('')
          this.wasmScene.clear_transient_segments()
        } else {
          // Chain forward: the new point becomes the anchor for the next segment.
          this.groundStage = { kind: 'anchored', anchor: cursor }
          this._lastGroundCursor = null
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

  /** Draw a rubber-band line from a 2D ground-plane anchor to the cursor. */
  private _drawRubberBandGround(anchor: [number, number], cursor: [number, number]): void {
    this._clearPreview()
    this._drawRubberBandSegment([anchor[0], anchor[1], 0], [cursor[0], cursor[1], 0], /* liftZ */ true)
  }

  /** Draw a rubber-band line from the last placed point to the cursor (face mode). */
  private _drawRubberBandFace(last: V3, cursor: V3): void {
    this._clearPreview()
    this._drawRubberBandSegment(last, cursor, /* liftZ */ false)
  }

  /**
   * Emit a LineSegments preview for a single segment.
   *
   * @param liftZ  When true, bump z by +0.001 to avoid z-fighting with the
   *               ground plane (ground mode). False in face mode.
   */
  private _drawRubberBandSegment(a: V3, b: V3, liftZ: boolean): void {
    const pts = new Float32Array([...a, ...b])
    if (liftZ) {
      pts[2] += 0.001
      pts[5] += 0.001
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
