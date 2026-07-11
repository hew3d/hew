/**
 * TapeMeasureTool — SketchUp-style Tape Measure, scoped to what the
 * current kernel/UI support: live distance readout, parallel construction
 * guide lines (from an existing edge), and standalone guide points.
 *
 * Gesture:
 *   1. First click:
 *      - On an edge (snap.elementKind === 'edge' for a live world Object,
 *        or 'sketch-edge' for a committed sketch line) → PARALLEL-GUIDE
 *        mode. Remembers the edge's two endpoints (via
 *        `wasmScene.edge_endpoints` / `sketch_edge_endpoints`) and the
 *        picked point on it.
 *      - Anywhere else → MEASURE mode. Remembers the picked point as P0.
 *   2. Pointer move previews:
 *      - Parallel mode: a dashed guide line through (edgePoint + offset)
 *        along the edge direction, where `offset` is the component of
 *        (cursor − edgePoint) perpendicular to the edge direction (so
 *        dragging the cursor off the edge "pulls" the guide sideways, as in
 *        SketchUp).
 *      - Measure mode: a dashed segment from P0 to the cursor, with a live
 *        `formatLength` readout via the measurement callback.
 *   3. Second click commits:
 *      - Parallel mode → `add_guide_line(origin, direction)`.
 *      - Measure mode, cursor on real geometry → just finalizes the readout
 *        (SketchUp does the same: measuring between two existing points
 *        doesn't drop a guide).
 *      - Measure mode, cursor in empty space → `add_guide_point(point)`.
 *   4. VCB numeric entry (typed digits while a stage is active) commits an
 *      exact typed distance along the current direction, mirroring MoveTool.
 *   5. Esc cancels the current stage; `cancel()` clears all preview state.
 *
 * Explicitly OUT of scope for this slice (see ROADMAP):
 *   - Protractor (angle guides) —, a separate tool.
 *   - Picking an individual existing guide to delete it — needs per-entity
 *     pick/select for guides, not built yet. Edit ▸ Delete Guide Lines
 *     (delete_all_guides) is the only deletion path for now.
 *   - Parallel guides from an edge on INSTANCED geometry: `edge_endpoints`
 *     only resolves live world Objects (instances are placed copies of a
 *     shared definition, not their own object), so an edge snap on instanced
 *     geometry simply falls back to measure mode (object-space coordinates
 *     would need the instance pose composed in, which is unsupported here).
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { editLengthBuffer, isLengthInputKey, pointAlong } from './moveInput'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'

export type OnGuideCreated = () => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/** Construction guide color — matches SceneRenderer's GUIDE_COLOR. */
const GUIDE_PREVIEW_COLOR = 0x9933cc
/** Half-length of the previewed parallel-guide line (meters). */
const GUIDE_HALF_LENGTH = 50

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'parallel'
      /** A point on the source edge, in world space (the pick that started this gesture). */
      edgePoint: [number, number, number]
      /** Unit direction of the source edge. */
      edgeDir: [number, number, number]
      /** Last computed guide origin (edgePoint + perpendicular offset). */
      origin: [number, number, number]
    }
  | {
      kind: 'measure'
      /** First picked point, in world space. */
      p0: [number, number, number]
      /** Last cursor point (snapped), in world space. */
      p1: [number, number, number]
      /** Whether the cursor is currently resting on real geometry (vs. empty space). */
      onGeometry: boolean
    }

/** v − (v·d)d for unit d: the component of v perpendicular to d. */
function perpComponent(
  v: [number, number, number],
  d: [number, number, number],
): [number, number, number] {
  const dot = v[0] * d[0] + v[1] * d[1] + v[2] * d[2]
  return [v[0] - dot * d[0], v[1] - dot * d[1], v[2] - dot * d[2]]
}

export class TapeMeasureTool implements Tool {
  readonly name = 'Tape Measure'

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onGuideCreated: OnGuideCreated
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** VCB buffer — raw string being typed by the user. */
  private typed: string = ''

  /** THREE.js LineSegments for the preview guide/segment. */
  private previewLine: THREE.LineSegments | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onGuideCreated: OnGuideCreated,
    onToast: OnToast,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onGuideCreated = onGuideCreated
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
  }

  // ── Optional Tool interface extensions ─────────────────────────────────

  capturingInput(): boolean {
    return this.stage.kind !== 'idle'
  }

  // ── Tool interface ──────────────────────────────────────────────────────

  onPointerMove(snap: Snap | null, _ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'parallel') {
      const cursor: [number, number, number] = [snap.x, snap.y, snap.z]
      const rel: [number, number, number] = [
        cursor[0] - this.stage.edgePoint[0],
        cursor[1] - this.stage.edgePoint[1],
        cursor[2] - this.stage.edgePoint[2],
      ]
      const offset = perpComponent(rel, this.stage.edgeDir)
      const origin: [number, number, number] = [
        this.stage.edgePoint[0] + offset[0],
        this.stage.edgePoint[1] + offset[1],
        this.stage.edgePoint[2] + offset[2],
      ]
      this.stage.origin = origin
      this._updatePreviewLine()
      // No numeric readout for parallel mode — SketchUp shows the offset
      // distance from the original edge; report that here.
      const offsetDist = Math.sqrt(offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2])
      this._reportOffsetOrTyped(offsetDist)
      return
    }

    if (this.stage.kind === 'measure') {
      this.stage.p1 = [snap.x, snap.y, snap.z]
      this.stage.onGeometry = snap.kind !== 'ground'
      this._updatePreviewLine()
      this._reportDistanceOrTyped(this.stage.p0, this.stage.p1)
    }
  }

  onPointerDown(snap: Snap | null, _ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'idle') {
      const edge = this._tryResolveEdge(snap)
      if (edge !== null) {
        const { edgePoint, edgeDir } = edge
        this.stage = { kind: 'parallel', edgePoint, edgeDir, origin: edgePoint }
        this._updatePreviewLine()
        return
      }

      const p0: [number, number, number] = [snap.x, snap.y, snap.z]
      this.stage = { kind: 'measure', p0, p1: p0, onGeometry: snap.kind !== 'ground' }
      this._updatePreviewLine()
      return
    }

    if (this.stage.kind === 'parallel') {
      this._commitParallelGuide(this.stage.origin, this.stage.edgeDir)
      return
    }

    if (this.stage.kind === 'measure') {
      this._commitMeasure(this.stage.p0, [snap.x, snap.y, snap.z], snap.kind !== 'ground')
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
      return
    }

    if (this.stage.kind === 'idle') return

    if (ev.key === 'Enter') {
      const meters = parseLengthToMeters(this.typed)
      if (meters !== null) {
        this._commitFromTyped(meters)
      }
      return
    }

    if (isLengthInputKey(ev.key)) {
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
      this.onMeasurementCb(this._typedReadout())
    }
  }

  /** The typed-buffer readout, suffixed for metric formats (imperial tokens
   * like `'`/`"` are already visible in the buffer itself). */
  private _typedReadout(): string {
    return typedReadout(this.typed)
  }

  cancel(): void {
    this._resetToIdle()
  }

  // ── Private helpers ─────────────────────────────────────────────────────

  /**
   * Resolve a snap to an edge's endpoints + the picked point on it, or null
   * if the snap isn't on a live world-Object edge or committed sketch edge.
   *
   * Only `elementKind === 'edge'` / `'sketch-edge'` snaps carry an edge
   * handle directly (on-edge, midpoint); an `endpoint` snap is a vertex, not
   * an edge, so it intentionally falls through to measure mode here (a
   * simplification — SketchUp lets you start a parallel guide from a
   * vertex-snapped point on an edge too, but distinguishing "vertex that
   * happens to sit on an edge" needs more inference-engine plumbing than
   * this slice adds).
   */
  private _tryResolveEdge(
    snap: Snap,
  ): { edgePoint: [number, number, number]; edgeDir: [number, number, number] } | null {
    let endpoints: Float64Array | number[] | undefined
    if (snap.elementKind === 'edge' && snap.object !== undefined && snap.element !== undefined) {
      endpoints = this.wasmScene.edge_endpoints(snap.object, snap.element)
    } else if (
      snap.elementKind === 'sketch-edge' &&
      snap.sketch !== undefined &&
      snap.element !== undefined
    ) {
      // A committed sketch line works as a guide reference too — the most
      // common case: a parallel guide off a just-drawn rectangle's edge.
      endpoints = this.wasmScene.sketch_edge_endpoints(snap.sketch, snap.element)
    }
    if (endpoints === undefined) return null // stale/consumed — fall back

    const [ax, ay, az, bx, by, bz] = endpoints
    const dx = bx - ax, dy = by - ay, dz = bz - az
    const len = Math.sqrt(dx * dx + dy * dy + dz * dz)
    if (len < 1e-9) return null // degenerate edge

    return {
      edgePoint: [snap.x, snap.y, snap.z],
      edgeDir: [dx / len, dy / len, dz / len],
    }
  }

  private _commitParallelGuide(origin: [number, number, number], dir: [number, number, number]): void {
    try {
      this.wasmScene.add_guide_line(origin[0], origin[1], origin[2], dir[0], dir[1], dir[2])
      this.onGuideCreated()
    } catch (err) {
      const raw = err instanceof Error ? err.message : String(err)
      this.onToast(`Couldn't create guide line: ${raw}`)
    }
    this._resetToIdle()
  }

  /**
   * Commit a measure-mode gesture. If the endpoint rests on real geometry,
   * SketchUp only finalizes the readout (no guide is created, since both ends
   * are already well-defined points). In empty space, drop a guide point so
   * the measured point is preserved as a construction reference.
   */
  private _commitMeasure(
    p0: [number, number, number],
    p1: [number, number, number],
    onGeometry: boolean,
  ): void {
    const dx = p1[0] - p0[0], dy = p1[1] - p0[1], dz = p1[2] - p0[2]
    const dist = Math.sqrt(dx * dx + dy * dy + dz * dz)
    this.onMeasurementCb(formatLength(dist))

    if (!onGeometry) {
      try {
        this.wasmScene.add_guide_point(p1[0], p1[1], p1[2])
        this.onGuideCreated()
      } catch (err) {
        const raw = err instanceof Error ? err.message : String(err)
        this.onToast(`Couldn't create guide point: ${raw}`)
      }
    }
    this._resetToIdle()
  }

  /** Commit an exact typed distance (Enter in the VCB), per stage. */
  private _commitFromTyped(dist: number): void {
    if (this.stage.kind === 'parallel') {
      const { edgePoint, edgeDir, origin } = this.stage
      // Signed offset direction: from edgePoint toward the current origin
      // (i.e. whichever side of the edge the cursor is currently on).
      const rel: [number, number, number] = [
        origin[0] - edgePoint[0],
        origin[1] - edgePoint[1],
        origin[2] - edgePoint[2],
      ]
      const relLen = Math.sqrt(rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2])
      if (relLen < 1e-9) {
        // No offset direction yet (cursor sitting on the edge) — nothing to commit.
        this._resetToIdle()
        return
      }
      const dir: [number, number, number] = [rel[0] / relLen, rel[1] / relLen, rel[2] / relLen]
      const newOrigin = pointAlong(edgePoint, dir, dist)
      this._commitParallelGuide(newOrigin, edgeDir)
      return
    }

    if (this.stage.kind === 'measure') {
      const { p0, p1 } = this.stage
      const rel: [number, number, number] = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]]
      const relLen = Math.sqrt(rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2])
      const dir: [number, number, number] = relLen < 1e-9 ? [1, 0, 0] : [rel[0] / relLen, rel[1] / relLen, rel[2] / relLen]
      const endpoint = pointAlong(p0, dir, dist)
      // Typed-exact endpoints are, by definition, not resting on picked
      // geometry — always drop a guide point so the typed distance is preserved.
      this._commitMeasure(p0, endpoint, false)
    }
  }

  private _resetToIdle(): void {
    this.stage = { kind: 'idle' }
    this.typed = ''
    this._clearPreviewLine()
    this.onMeasurementCb('')
  }

  private _reportOffsetOrTyped(offsetDist: number): void {
    if (this.typed !== '') {
      this.onMeasurementCb(this._typedReadout())
      return
    }
    this.onMeasurementCb(formatLength(offsetDist))
  }

  private _reportDistanceOrTyped(p0: [number, number, number], p1: [number, number, number]): void {
    if (this.typed !== '') {
      this.onMeasurementCb(this._typedReadout())
      return
    }
    const dx = p1[0] - p0[0], dy = p1[1] - p0[1], dz = p1[2] - p0[2]
    this.onMeasurementCb(formatLength(Math.sqrt(dx * dx + dy * dy + dz * dz)))
  }

  /**
   * Rebuild the dashed preview line in the preview group, dispatched by
   * stage: a long dashed line through (origin, edgeDir) in parallel mode, or
   * a dashed segment from p0 to p1 in measure mode. Removes the previous
   * preview (if any) first.
   */
  private _updatePreviewLine(): void {
    this._clearPreviewLine()

    // The preview is a solid line (the placed guide renders dashed). A
    // screen-constant manual dash here would just duplicate SceneRenderer's
    // dash logic for a transient overlay; a thin solid line reads clearly and
    // avoids the metre-sized-dash problem entirely.
    let pts: Float32Array
    if (this.stage.kind === 'parallel') {
      const { origin, edgeDir } = this.stage
      const nx = edgeDir[0] * GUIDE_HALF_LENGTH
      const ny = edgeDir[1] * GUIDE_HALF_LENGTH
      const nz = edgeDir[2] * GUIDE_HALF_LENGTH
      pts = new Float32Array([
        origin[0] - nx, origin[1] - ny, origin[2] - nz,
        origin[0] + nx, origin[1] + ny, origin[2] + nz,
      ])
    } else if (this.stage.kind === 'measure') {
      const { p0, p1 } = this.stage
      pts = new Float32Array([p0[0], p0[1], p0[2], p1[0], p1[1], p1[2]])
    } else {
      return
    }

    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(pts, 3))
    const mat = new THREE.LineBasicMaterial({ color: GUIDE_PREVIEW_COLOR, depthTest: false })
    const line = new THREE.LineSegments(geo, mat)
    this.preview.add(line)
    this.previewLine = line
  }

  private _clearPreviewLine(): void {
    if (this.previewLine === null) return
    this.previewLine.geometry.dispose()
    if (this.previewLine.material instanceof THREE.Material) {
      this.previewLine.material.dispose()
    }
    this.preview.remove(this.previewLine)
    this.previewLine = null
  }
}
