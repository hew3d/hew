/**
 * CircleTool — two-click faceted-circle (regular N-gon) sketching.
 *
 * A "circle" here is a faceted regular N-gon (true arcs are out of scope —
 * see). It decomposes into N chained `sketch_add_segment` calls (ground
 * mode) or one `split_face_inner` call with N loop points (face mode) —
 * mirroring exactly how RectangleTool works with 4 corners. No kernel change
 * is needed; push/pull then works for free once a closed loop forms a
 * region.
 *
 * Two modes:
 *
 * Ground mode (activeContext === null):
 *   1. First click: anchor center (snapped on Z=0)
 *   2. Move: rubber-band N-gon preview whose first vertex passes through the
 *      cursor (radius = distance from center to cursor; start angle = angle
 *      from center to cursor)
 *   3. Second click: commit — begin_ground_sketch() if needed, N
 *      sketch_add_segment calls chaining vertex[i] -> vertex[i+1], last one
 *      vertex[N-1] -> vertex[0] (using the SAME stored vertex[0] coords for
 *      exact closure)
 *   4. Esc between clicks: cancel stage 1
 *   Calls onCommit() after each successful commit so the viewport can
 *   refresh scene geometry and trigger re-render.
 *
 * Face mode (activeContext !== null):
 *   1. First click on a face of the entered object: anchor center (on face
 *      plane)
 *   2. Move: rubber-band N-gon preview projected onto that face plane
 *   3. Second click: commit — split_face_inner() on the entered object face
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
import { circlePolygonGround, circlePolygonFace, facePlaneBasis, parseKernelErrorCode, kernelErrorMessage } from '../viewport/geoHelpers'
import { makeFatSegments, disposeFatSegments, PREVIEW_LINE_STYLE } from '../viewport/fatLine'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { editLengthBuffer, isLengthInputKey } from './moveInput'
import { runSketchGesture, makeSketchHandleCache, type SketchHandleCache } from './sketchGesture'

import { segmentsPerTurn } from './arcMath'

/** Floor of the adaptive facet count (docs/design/true-curves.md §6): small
 * circles are regular 24-gons; larger radii adapt up via `segmentsPerTurn`
 * so the chord sagitta stays within the draw-time budget. The analytic
 * center/radius rides the curve chain regardless. */
export const CIRCLE_SEGMENTS = 24

/** Adaptive facet count for a ground circle (center/rim in plane coords). */
function groundSegments(center: [number, number], rim: [number, number]): number {
  return segmentsPerTurn(Math.hypot(rim[0] - center[0], rim[1] - center[1]))
}

/** Adaptive facet count for an on-face circle (center/rim in world coords). */
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

/** Ground stage: waiting for first click, or waiting for second click */
type GroundStage =
  | { kind: 'idle' }
  | { kind: 'anchored'; center: [number, number] }

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

export class CircleTool implements Tool {
  readonly name = 'Circle'

  private groundStage: GroundStage = { kind: 'idle' }
  private faceStage: FaceStage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnCircleCommit
  private onFaceImprint: OnFaceImprint
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** Cached ground-sketch handle — the Viewport passes one cache shared by
   *  every draw tool, so mixed-tool profiles land in a single sketch. */
  private readonly sketchCache: SketchHandleCache

  /** The currently active editing context (entered object), or null at top level. */
  private _activeContext: bigint | null = null

  /** VCB buffer — raw string being typed by the user (radius, in display units) */
  private typed: string = ''

  /** Last rubber-band cursor positions, tracked for typed-entry direction */
  private _lastGroundCursor: [number, number] | null = null
  private _lastFaceCursor: V3 | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnCircleCommit,
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
   * Provide a constraint plane for snap so off-plane/occluded geometry is
   * excluded while snapping during face-mode drawing.
   *
   * - Ground mode: return null (unconstrained).
   * - Face mode, anchored: return the already-known face plane so subsequent
   *   snaps stay on that plane.
   * - Face mode, idle: pick the hovered face and return its plane so the
   *   FIRST-click center lands precisely on the face, preventing the kernel
   *   from rejecting a non-planar loop.
   */
  snapConstraint(ray: Ray): { constraintPlane?: { point: [number, number, number]; normal: [number, number, number] } } | null {
    if (this._activeContext === null) {
      // Ground mode — no constraint
      return null
    }

    if (this.faceStage.kind === 'anchored') {
      // Already anchored: lock to the established face plane
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
      // Face mode
      if (this.faceStage.kind !== 'anchored') {
        this._clearPreview()
        this.onMeasurementCb('')
        return
      }
      const { center, normal, planePoint } = this.faceStage
      // Project cursor ray onto face plane
      const cursorOnPlane = intersectPlane(ray.origin, ray.direction, planePoint, normal)
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
      // Ground mode
      if (this.groundStage.kind !== 'anchored' || snap === null) {
        this._clearPreview()
        this.onMeasurementCb('')
        return
      }
      const { center } = this.groundStage
      const cursor: [number, number] = [snap.x, snap.y]
      this._lastGroundCursor = cursor
      const verts = circlePolygonGround(center, cursor, groundSegments(center, cursor))
      if (verts.length > 0) {
        this._drawRubberBandGround(verts)
        this._reportMeasurement([center[0], center[1], 0], [cursor[0], cursor[1], 0])
      } else {
        this._clearPreview()
        if (this.typed === '') this.onMeasurementCb('')
      }
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
   * Typed VCB entry is available once the center has been placed (either
   * ground or face mode) — see the Viewport key router, which routes
   * digit/letter/arrow keys here instead of tool-switch shortcuts while this
   * returns true.
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
   * cursor hasn't moved yet). Dispatches to ground or face mode depending on
   * which stage is anchored.
   */
  private _commitTyped(radius: number): void {
    if (this.groundStage.kind === 'anchored') {
      const { center } = this.groundStage
      const cursor = this._lastGroundCursor ?? [center[0] + 1, center[1]]
      const dx = cursor[0] - center[0]
      const dy = cursor[1] - center[1]
      const len = Math.hypot(dx, dy)
      const dir: [number, number] = len < 1e-9 ? [1, 0] : [dx / len, dy / len]
      const rim: [number, number] = [center[0] + dir[0] * radius, center[1] + dir[1] * radius]

      this._commitGroundCircle(center, rim)
      this.groundStage = { kind: 'idle' }
      this.typed = ''
      this._lastGroundCursor = null
      this._clearPreview()
      this.onMeasurementCb('')
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

  // ------------------------------------------------------------------ ground mode

  private _onPointerDownGround(snap: Snap | null): void {
    if (snap === null) return

    if (this.groundStage.kind === 'idle') {
      // First click: set center
      this.groundStage = { kind: 'anchored', center: [snap.x, snap.y] }
      this._lastGroundCursor = null
    } else {
      // Second click: commit the circle
      const { center } = this.groundStage
      const cursor: [number, number] = [snap.x, snap.y]

      // Skip degenerate circles (zero radius)
      if (Math.hypot(cursor[0] - center[0], cursor[1] - center[1]) < 1e-7) {
        return
      }

      this._commitGroundCircle(center, cursor)
      this.groundStage = { kind: 'idle' }
      this.typed = ''
      this._lastGroundCursor = null
      this._clearPreview()
      this.onMeasurementCb('')
    }
  }

  private _commitGroundCircle(center: [number, number], rim: [number, number]): void {
    const verts = circlePolygonGround(center, rim, groundSegments(center, rim))
    if (verts.length === 0) return // degenerate — ignore

    try {
      runSketchGesture(this.wasmScene, this.sketchCache, (sketch) => {
        let lastRegionsCreated: bigint[] = []
        // The whole circle is ONE curve chain — clicking any facet later
        // selects (and deletes) the circle as a unit — and it carries the
        // exact analytic circle the facets approximate (durable
        // center/radius — docs/design/true-curves.md).
        const radius = Math.hypot(rim[0] - center[0], rim[1] - center[1])
        this.wasmScene.sketch_begin_curve_with(sketch, center[0], center[1], 0, radius)
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
      // First click: pick a face of the entered object
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
        const center: V3 = [snap.x, snap.y, snap.z]

        this.faceStage = {
          kind: 'anchored',
          object: objectHandle,
          face: faceHandle,
          normal,
          planePoint: center,
          center,
        }
        this._lastFaceCursor = null
      } finally {
        pick.free()
      }
    } else {
      // Second click: commit the face imprint
      const { object, face, normal, planePoint, center } = this.faceStage

      // Project the click ray onto the face plane for the cursor position
      const cursorOnPlane = intersectPlane(ray.origin, ray.direction, planePoint, normal)
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
   * (docs/design/true-curves.md, playtest fix C3). The radius is measured to
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
   * Used in ground mode.
   */
  private _drawRubberBandGround(verts: V3[]): void {
    this._clearPreview()
    this._drawRubberBandVerts(verts, /* liftZ */ true)
  }

  /**
   * Draw a rubber-band N-gon from explicit 3D vertices.
   * Used in face mode — vertices already lie on the face plane.
   */
  private _drawRubberBandFace(verts: V3[]): void {
    this._clearPreview()
    this._drawRubberBandVerts(verts, /* liftZ */ false)
  }

  /**
   * Emit a LineSegments preview for a closed N-vertex loop.
   *
   * @param verts  N world-space xyz vertices in order.
   * @param liftZ  When true, bump each z by +0.001 to avoid z-fighting with
   *               the ground plane (ground mode). False in face mode.
   */
  private _drawRubberBandVerts(verts: V3[], liftZ: boolean): void {
    const n = verts.length
    const pts = new Float32Array(n * 2 * 3)
    for (let i = 0; i < n; i++) {
      const a = verts[i]
      const b = verts[(i + 1) % n]
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
