/**
 * RectangleTool — two-click rectangle sketching.
 *
 * Two modes:
 *
 * Ground mode (activeContext === null):
 *   1. First click: anchor corner (snapped on Z=0)
 *   2. Move: rubber-band rectangle preview on the ground plane
 *   3. Second click: commit — begin_ground_sketch() if needed, four
 *      sketch_add_segment calls forming the rectangle
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
import { rectangleCorners, faceRectangleCorners, facePlaneBasis } from '../viewport/geoHelpers'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { makeFatSegments, disposeFatSegments, PREVIEW_LINE_STYLE } from '../viewport/fatLine'
import { formatLength, parseDimensionsToMeters, typedReadout } from '../settings/units'
import { editDimsBuffer } from './moveInput'
import { runSketchGesture, makeSketchHandleCache, type SketchHandleCache } from './sketchGesture'
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

/** Ground stage: waiting for first click, or waiting for second click */
type GroundStage =
  | { kind: 'idle' }
  | { kind: 'anchored'; anchor: [number, number] }

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

export class RectangleTool implements Tool {
  readonly name = 'Rectangle'

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    return this.groundStage.kind === 'idle' && this.faceStage.kind === 'idle'
      ? 'Click the first corner — on the ground plane or any face.'
      : 'Click the opposite corner — or type exact dimensions.'
  }

  private groundStage: GroundStage = { kind: 'idle' }
  private faceStage: FaceStage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnRectangleCommit
  private onFaceImprint: OnFaceImprint
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** Cached ground-sketch handle — the Viewport passes one cache shared by
   *  every draw tool, so mixed-tool profiles land in a single sketch. */
  private readonly sketchCache: SketchHandleCache

  /** The currently active editing context (entered object), or null at top level. */
  private _activeContext: bigint | null = null

  /** VCB buffer — raw string being typed by the user (W,D in display units) */
  private typed: string = ''

  /** Last rubber-band cursor positions, tracked for typed-entry sign/direction */
  private _lastGroundCursor: [number, number] | null = null
  private _lastFaceCursor: V3 | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnRectangleCommit,
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
   * other draw tools):
   *   - Already anchored in one mode: stick with it (mid-gesture).
   *   - Inside an entered object context: always face mode (drawing stays
   *     scoped to that object — no top-level ground sketch from inside).
   *   - Otherwise idle at top level: face mode if an eligible Object face is
   *     under the cursor (via `pick_face`), else ground mode.
   */
  private _currentMode(ray?: Ray): 'face' | 'ground' {
    if (this.faceStage.kind === 'anchored') return 'face'
    if (this.groundStage.kind === 'anchored') return 'ground'
    // Inside an entered object context, drawing stays scoped to that
    // object's faces — a click elsewhere is ignored by the face handler
    // rather than falling through to a top-level ground sketch.
    if (this._activeContext !== null) return 'face'
    if (ray === undefined) return 'ground'
    return this._eligiblePickFor(ray) !== null ? 'face' : 'ground'
  }

  /**
   * Provide a constraint plane for snap so off-plane/occluded geometry is
   * excluded while snapping during face-mode drawing.
   *
   * - Face mode, anchored: return the already-known face plane so subsequent
   *   snaps stay on that plane.
   * - Idle: pick the hovered face (if an eligible one is under the cursor)
   *   and return its plane so the FIRST-click anchor lands precisely on the
   *   face, preventing the kernel from rejecting a non-planar rectangle.
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

    if (this.groundStage.kind === 'anchored') {
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
      // Face mode
      if (this.faceStage.kind !== 'anchored') {
        this._clearPreview()
        this.onMeasurementCb('')
        return
      }
      const { anchor, normal, planePoint } = this.faceStage
      // Project cursor ray onto face plane
      const cursorOnPlane = intersectPlane(ray.origin, ray.direction, planePoint, normal)
      if (cursorOnPlane === null) {
        this._clearPreview()
        this.onMeasurementCb('')
        return
      }
      this._lastFaceCursor = cursorOnPlane
      const corners = faceRectangleCorners(anchor, cursorOnPlane, normal)
      if (corners !== null) {
        this._drawRubberBandFace(corners)
        this._reportMeasurement(corners)
      } else {
        this._clearPreview()
        if (this.typed === '') this.onMeasurementCb('')
      }
    } else {
      // Ground mode
      if (this.groundStage.kind !== 'anchored' || snap === null) {
        this._clearPreview()
        this.onMeasurementCb('')
        return
      }
      const { anchor } = this.groundStage
      const cursor: [number, number] = [snap.x, snap.y]
      this._lastGroundCursor = cursor
      this._drawRubberBandGround(anchor, cursor)
      this._reportMeasurement(rectangleCorners(anchor, cursor))
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
   * Typed VCB entry is available once the first corner has been placed
   * (either ground or face mode) — see the Viewport key router, which
   * routes digit/letter/arrow keys here instead of tool-switch shortcuts
   * while this returns true.
   */
  capturingInput(): boolean {
    return this.groundStage.kind === 'anchored' || this.faceStage.kind === 'anchored'
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
      return
    }

    if (!this.capturingInput()) return

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
    this.groundStage = { kind: 'idle' }
    this.faceStage = { kind: 'idle' }
    this.typed = ''
    this._lastGroundCursor = null
    this._lastFaceCursor = null
    this._clearPreview()
    this.onMeasurementCb('')
  }

  /**
   * A new/loaded document replaced the Scene, so the cached ground-sketch
   * handle is now stale (reusing it throws UnknownSketch). Drop it and reset.
   * Called by the Viewport from `notifyLoaded`.
   */
  onDocumentReset(): void {
    this.sketchCache.set(null)
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
   * direction along each axis. Dispatches to ground or face mode depending
   * on which stage is anchored.
   */
  private _commitTyped(w: number, d: number): void {
    if (this.groundStage.kind === 'anchored') {
      const { anchor } = this.groundStage
      // Sign of growth along each axis follows the last rubber-band cursor
      // position (so typing matches the direction the user was dragging);
      // default +,+ if the cursor hasn't moved yet.
      const cursor = this._lastGroundCursor ?? anchor
      const signX = cursor[0] - anchor[0] < 0 ? -1 : 1
      const signY = cursor[1] - anchor[1] < 0 ? -1 : 1
      const farCorner: [number, number] = [anchor[0] + signX * w, anchor[1] + signY * d]

      this._commitGroundRectangle(anchor, farCorner)
      this.groundStage = { kind: 'idle' }
      this.typed = ''
      this._lastGroundCursor = null
      this._clearPreview()
      this.onMeasurementCb('')
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

  // ------------------------------------------------------------------ ground mode

  private _onPointerDownGround(snap: Snap | null): void {
    if (snap === null) return

    if (this.groundStage.kind === 'idle') {
      // First click: set anchor
      this.groundStage = { kind: 'anchored', anchor: [snap.x, snap.y] }
      this._lastGroundCursor = null
    } else {
      // Second click: commit the rectangle
      const { anchor } = this.groundStage
      const cursor: [number, number] = [snap.x, snap.y]

      // Skip degenerate rectangles (same point or zero area)
      if (
        Math.abs(anchor[0] - cursor[0]) < 1e-8 ||
        Math.abs(anchor[1] - cursor[1]) < 1e-8
      ) {
        return
      }

      this._commitGroundRectangle(anchor, cursor)
      this.groundStage = { kind: 'idle' }
      this.typed = ''
      this._lastGroundCursor = null
      this._clearPreview()
      this.onMeasurementCb('')
    }
  }

  private _commitGroundRectangle(a: [number, number], b: [number, number]): void {
    try {
      runSketchGesture(this.wasmScene, this.sketchCache, (sketch) => {
        const corners = rectangleCorners(a, b)
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
      const cursorOnPlane = intersectPlane(ray.origin, ray.direction, planePoint, normal)
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
   * Draw a rubber-band rectangle from two 2D ground-plane corners.
   * Used in ground mode.
   */
  private _drawRubberBandGround(a: [number, number], b: [number, number]): void {
    this._clearPreview()
    const corners = rectangleCorners(a, b)
    this._drawRubberBandCorners(corners)
  }

  /**
   * Draw a rubber-band rectangle from four explicit 3D corners.
   * Used in face mode — corners already lie on the face plane.
   */
  private _drawRubberBandFace(corners: [V3, V3, V3, V3]): void {
    this._clearPreview()
    this._drawRubberBandCorners(corners)
  }

  /**
   * Emit a LineSegments preview for a closed 4-corner loop. Corners are used
   * exactly as given — the preview's depth bias (PREVIEW_LINE_STYLE,
   * depthPolicy.ts) settles coincidence with the ground/committed lines, so
   * no z-lift.
   *
   * @param corners  Four world-space xyz corners in order.
   */
  private _drawRubberBandCorners(corners: readonly [V3, V3, V3, V3]): void {
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
