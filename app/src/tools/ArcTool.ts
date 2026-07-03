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
 * Mode selection mirrors CircleTool exactly: ground mode when
 * `activeContext === null`, face mode (on the entered object's faces) when a
 * context is active. All three points lie in the sketch plane — Z=0, or the
 * picked face's plane via `snapConstraint`.
 *
 * Degenerate guards (constants in arcMath.ts — no inline epsilons):
 *   - zero/short chord (B on A): the B click is ignored.
 *   - flat bulge (|sagitta| < ARC_MIN_SAGITTA_M): the commit click is refused
 *     and the measurement line hints to pull out the bulge.
 *
 * VCB: no typed entry in  — the "12s" segment override is explicitly
 * deferred, and typed radius entry is ambiguous for a 2-point arc (radius
 * alone doesn't pick the bulge side), so the gesture is pointer-only.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { V3 } from '../viewport/geoHelpers'
import { facePlaneBasis, parseKernelErrorCode, kernelErrorMessage } from '../viewport/geoHelpers'
import { makeFatSegments, disposeFatSegments, PREVIEW_LINE_STYLE } from '../viewport/fatLine'
import { formatLength } from '../settings/units'
import { segmentLength } from './lineInput'
import {
  ARC_MIN_CHORD_M,
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

  private groundStage: GroundStage = { kind: 'idle' }
  private faceStage: FaceStage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnArcCommit
  private onFaceImprint: OnFaceImprint
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** Handle to the current active sketch — reused across commits if not null */
  private sketchHandle: bigint | null = null

  /** The currently active editing context (entered object), or null at top level. */
  private _activeContext: bigint | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnArcCommit,
    onToast: OnToast,
    onFaceImprint: OnFaceImprint,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onCommit = onCommit
    this.onFaceImprint = onFaceImprint
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
  }

  /** Set the active editing context (entered object), or null for top level. */
  setActiveContext(id: bigint | null): void {
    if (id === this._activeContext) return  // re-asserting the same context must not abort an in-progress gesture
    this._activeContext = id
    this.cancel()
  }

  /**
   * Provide a constraint plane for snap so off-plane/occluded geometry is
   * excluded while snapping during face-mode drawing — identical to
   * CircleTool's policy:
   *
   * - Ground mode: return null (unconstrained).
   * - Face mode, gesture in progress: return the already-locked face plane.
   * - Face mode, idle: pick the hovered face and return its plane so the
   *   FIRST-click endpoint lands precisely on the face.
   */
  snapConstraint(ray: Ray): { constraintPlane?: { point: [number, number, number]; normal: [number, number, number] } } | null {
    if (this._activeContext === null) {
      // Ground mode — no constraint
      return null
    }

    if (this.faceStage.kind !== 'idle') {
      // Gesture in progress: lock to the established face plane
      return {
        constraintPlane: {
          point: this.faceStage.planePoint,
          normal: this.faceStage.normal,
        },
      }
    }

    // Face mode, idle: pick the face under the cursor and use its plane
    const pick = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (pick === undefined) return null

    try {
      const objectHandle = pick.object()
      if (objectHandle !== this._activeContext) return null

      const faceHandle = pick.face()
      const a = this.wasmScene.face_plane(objectHandle, faceHandle)
      return {
        constraintPlane: {
          point: [a[0], a[1], a[2]],
          normal: [a[3], a[4], a[5]],
        },
      }
    } finally {
      pick.free()
    }
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (this._activeContext !== null) {
      this._onPointerMoveFace(snap, ray)
    } else {
      this._onPointerMoveGround(snap)
    }
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (this._activeContext !== null) {
      this._onPointerDownFace(snap, ray)
    } else {
      this._onPointerDownGround(snap)
    }
  }

  /**
   * True while a gesture is in progress, so the Viewport routes keys here
   * (Escape stage-back) instead of treating letters as tool-switch
   * shortcuts mid-gesture. There is no typed VCB entry (see module doc).
   */
  capturingInput(): boolean {
    return this.groundStage.kind !== 'idle' || this.faceStage.kind !== 'idle'
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this._stepBack()
    }
    // No VCB — all other keys are ignored (segment override deferred).
  }

  /** Esc steps back one stage: bulge → chord (A kept), chord → idle. */
  private _stepBack(): void {
    if (this.groundStage.kind === 'bulge') {
      this.groundStage = { kind: 'chord', a: this.groundStage.a }
      this._clearPreview()
      this.onMeasurementCb('')
      return
    }
    if (this.faceStage.kind === 'bulge') {
      const { object, face, normal, planePoint, a } = this.faceStage
      this.faceStage = { kind: 'chord', object, face, normal, planePoint, a }
      this._clearPreview()
      this.onMeasurementCb('')
      return
    }
    this.cancel()
  }

  cancel(): void {
    this.groundStage = { kind: 'idle' }
    this.faceStage = { kind: 'idle' }
    this._clearPreview()
    this.onMeasurementCb('')
  }

  /**
   * A new/loaded document replaced the Scene, so the cached ground-sketch
   * handle is now stale (reusing it throws UnknownSketch). Drop it and reset.
   * Called by the Viewport from `notifyLoaded`.
   */
  onDocumentReset(): void {
    this.sketchHandle = null
    this.cancel()
  }

  // ------------------------------------------------------------------ ground mode

  private _onPointerMoveGround(snap: Snap | null): void {
    if (snap === null || this.groundStage.kind === 'idle') {
      this._clearPreview()
      this.onMeasurementCb('')
      return
    }

    if (this.groundStage.kind === 'chord') {
      // Stage 2: rubber-band the chord A→cursor, report chord length.
      const { a } = this.groundStage
      const cursor: [number, number] = [snap.x, snap.y]
      this._clearPreview()
      this._drawSegments([[a[0], a[1], 0], [cursor[0], cursor[1], 0]], /* liftZ */ true)
      this.onMeasurementCb(formatLength(Math.hypot(cursor[0] - a[0], cursor[1] - a[1])))
      return
    }

    // Stage 3: rubber-band the faceted arc through the cursor's bulge side.
    const { a, b } = this.groundStage
    const s = chordSagitta(a, b, [snap.x, snap.y])
    const verts = s === null ? null : this._groundPolyline(a, b, s)
    if (verts === null) {
      // Flat bulge — fall back to showing the bare chord.
      this._clearPreview()
      this._drawSegments([[a[0], a[1], 0], [b[0], b[1], 0]], /* liftZ */ true)
      this.onMeasurementCb('')
      return
    }
    this._clearPreview()
    this._drawSegments(verts, /* liftZ */ true)
    this._reportRadius(a, b, s as number)
  }

  private _onPointerDownGround(snap: Snap | null): void {
    if (snap === null) return

    if (this.groundStage.kind === 'idle') {
      // First click: endpoint A
      this.groundStage = { kind: 'chord', a: [snap.x, snap.y] }
      return
    }

    if (this.groundStage.kind === 'chord') {
      // Second click: endpoint B (the chord). Ignore a degenerate chord.
      const { a } = this.groundStage
      const b: [number, number] = [snap.x, snap.y]
      if (Math.hypot(b[0] - a[0], b[1] - a[1]) < ARC_MIN_CHORD_M) return
      this.groundStage = { kind: 'bulge', a, b }
      this._clearPreview()
      this.onMeasurementCb('')
      return
    }

    // Third click: commit at the cursor's sagitta. Refuse a flat bulge.
    const { a, b } = this.groundStage
    const s = chordSagitta(a, b, [snap.x, snap.y])
    const verts = s === null ? null : this._groundPolyline(a, b, s)
    if (verts === null) {
      this.onMeasurementCb(FLAT_BULGE_HINT)
      return
    }

    this._commitGroundChain(verts)
    this.groundStage = { kind: 'idle' }
    this._clearPreview()
    this.onMeasurementCb('')
  }

  /** The faceted ground-plane polyline for chord a→b with sagitta s (z=0). */
  private _groundPolyline(a: Vec2, b: Vec2, s: number): V3[] | null {
    return arcPolylineOnPlane([a[0], a[1], 0], [b[0], b[1], 0], s, [1, 0, 0], [0, 1, 0])
  }

  /** Commit the open polyline chain as N ground-sketch segments. */
  private _commitGroundChain(verts: V3[]): void {
    try {
      // Begin sketch if we don't already have one
      if (this.sketchHandle === null) {
        this.sketchHandle = this.wasmScene.begin_ground_sketch()
      }
      const sketch = this.sketchHandle

      let lastRegionsCreated: bigint[] = []
      for (let i = 0; i < verts.length - 1; i++) {
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

      this.onCommit({ sketchHandle: sketch, regionsCreated: lastRegionsCreated })
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

  /** In-plane (u,v) sagitta of `cursor` for the face chord a→b, plus the
   * face polyline it implies. Returns null when flat/degenerate. */
  private _facePolyline(a: V3, b: V3, normal: V3, cursor: V3): V3[] | null {
    const basis = facePlaneBasis(normal)
    if (basis === null) return null
    const { u, v } = basis
    const project = (p: V3): Vec2 => {
      const dx = p[0] - a[0]
      const dy = p[1] - a[1]
      const dz = p[2] - a[2]
      return [dx * u[0] + dy * u[1] + dz * u[2], dx * v[0] + dy * v[1] + dz * v[2]]
    }
    const b2 = project(b)
    const s = chordSagitta([0, 0], b2, project(cursor))
    if (s === null) return null
    return arcPolylineOnPlane(a, b, s, u, v)
  }

  private _onPointerMoveFace(snap: Snap | null, ray: Ray): void {
    if (this.faceStage.kind === 'idle') {
      this._clearPreview()
      this.onMeasurementCb('')
      return
    }
    const cursor = this._faceCursor(snap, ray)
    if (cursor === null) {
      this._clearPreview()
      this.onMeasurementCb('')
      return
    }

    if (this.faceStage.kind === 'chord') {
      const { a } = this.faceStage
      this._clearPreview()
      this._drawSegments([a, cursor], /* liftZ */ false)
      this.onMeasurementCb(formatLength(segmentLength(a, cursor)))
      return
    }

    if (this.faceStage.kind !== 'bulge') return // (narrowing is lost across the _faceCursor call)
    const { a, b, normal } = this.faceStage
    const verts = this._facePolyline(a, b, normal, cursor)
    this._clearPreview()
    if (verts === null) {
      this._drawSegments([a, b], /* liftZ */ false)
      this.onMeasurementCb('')
      return
    }
    this._drawSegments(verts, /* liftZ */ false)
    this._reportRadiusFromChain(verts)
  }

  private _onPointerDownFace(snap: Snap | null, ray: Ray): void {
    if (this.faceStage.kind === 'idle') {
      // First click: pick a face of the entered object (endpoint A on it).
      if (snap === null) return

      const pick = this.wasmScene.pick_face(
        ray.origin[0], ray.origin[1], ray.origin[2],
        ray.direction[0], ray.direction[1], ray.direction[2],
      )
      if (pick === undefined) return

      try {
        const objectHandle = pick.object()
        if (objectHandle !== this._activeContext) return

        const faceHandle = pick.face()
        const normalArr = this.wasmScene.face_normal(objectHandle, faceHandle)
        const normal: V3 = [normalArr[0], normalArr[1], normalArr[2]]
        const a: V3 = [snap.x, snap.y, snap.z]

        this.faceStage = {
          kind: 'chord',
          object: objectHandle,
          face: faceHandle,
          normal,
          planePoint: a,
          a,
        }
      } finally {
        pick.free()
      }
      return
    }

    const cursor = this._faceCursor(snap, ray)
    if (cursor === null) return

    if (this.faceStage.kind === 'chord') {
      // Second click: endpoint B. Ignore a degenerate chord.
      const { object, face, normal, planePoint, a } = this.faceStage
      if (segmentLength(a, cursor) < ARC_MIN_CHORD_M) return
      this.faceStage = { kind: 'bulge', object, face, normal, planePoint, a, b: cursor }
      this._clearPreview()
      this.onMeasurementCb('')
      return
    }

    // Third click: commit the face cut. Refuse a flat bulge.
    if (this.faceStage.kind !== 'bulge') return // (narrowing is lost across the _faceCursor call)
    const { object, face, a, b, normal } = this.faceStage
    const verts = this._facePolyline(a, b, normal, cursor)
    if (verts === null) {
      this.onMeasurementCb(FLAT_BULGE_HINT)
      return
    }

    this.faceStage = { kind: 'idle' }
    this._clearPreview()
    this.onMeasurementCb('')
    this._commitFaceChain(object, face, verts)
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

  /** Report the live arc radius for a valid ground bulge. */
  private _reportRadius(a: Vec2, b: Vec2, s: number): void {
    const arc = arcFromChord(a, b, s)
    if (arc === null) {
      this.onMeasurementCb('')
      return
    }
    this.onMeasurementCb(`R ${formatLength(arc.radius)}`)
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
      this.onMeasurementCb('')
      return
    }
    const radius = (ab * am * bm) / (4 * Math.sqrt(areaSq))
    this.onMeasurementCb(`R ${formatLength(radius)}`)
  }

  // ------------------------------------------------------------------ preview

  /**
   * Emit a fat-line preview for an open polyline.
   *
   * @param verts  Ordered world-space vertices (>= 2).
   * @param liftZ  When true, bump each z by +0.001 to avoid z-fighting with
   *               the ground plane (ground mode). False in face mode.
   */
  private _drawSegments(verts: V3[], liftZ: boolean): void {
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
    if (liftZ) {
      for (let i = 2; i < pts.length; i += 3) pts[i] += 0.001
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
