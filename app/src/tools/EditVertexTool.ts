/**
 * EditVertexTool — Phase D slice 3: drag a single vertex of a free-standing
 * (not-yet-extruded) sketch to a new position. Topology-preserving — incident
 * edges stretch to follow; the kernel refuses (with a typed error) any drag
 * that would cross/merge geometry, surfaced here as a toast.
 *
 * Gesture (two-click, mirrors MoveTool's idle/base machine):
 *   1. idle  onPointerDown : ray-pick a sketch vertex (`pick_sketch_vertex`).
 *      A hit seeds the gesture (`{ sketch, vertex, base }`) and builds a ghost
 *      preview; a miss is a no-op (idle has no destination to act on).
 *   2. base  onPointerMove : the ghost follows the resolved snap; reports the
 *      live base→cursor distance as the measurement readout.
 *   3. base  onPointerDown : if the destination ~= base (no move), just reset
 *      to idle (a click-without-drag is a cancel, not a zero-length move).
 *      Otherwise commit via `move_sketch_vertex`; a thrown kernel error
 *      (`WouldRetopologize`, `DegenerateSegment`, `PointOffPlane`,
 *      `UnknownSketch`, ...) is toasted and the vertex stays at `base` — the
 *      gesture always resets to idle afterward either way.
 *   4. Escape cancels the gesture (ghost cleared, back to idle).
 *
 * Ghost preview: only ONE vertex moves, so the whole-sketch translate clone
 * (`buildSketchPreviewClone` translated as a rigid body) would be wrong here.
 * Instead this rebuilds just the INCIDENT edges: `sketch_lines(sketch)` is
 * scanned for segments with an endpoint ~= `base` (within `PICK_EPSILON`),
 * and for each one a line is drawn from its OTHER endpoint to the current
 * cursor position. If a sketch vertex has no incident edges in the line
 * buffer (a lone point — shouldn't normally happen for a sketch vertex, but
 * cheap to guard), the preview is simply empty; the live drag still works,
 * it's only the rubber-band feedback that would be missing.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { parseKernelErrorCode, kernelErrorMessage } from '../viewport/geoHelpers'
import { formatLength } from '../settings/units'

export type OnVertexMoveCommit = () => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/** Ghost line color — matches the sketch preview ghost used by MoveTool/transformPreview. */
const SKETCH_LINE_COLOR = 0x2266cc
/** Distance (meters) within which a `sketch_lines` endpoint is considered "the picked vertex". */
const PICK_EPSILON = 1e-6

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'base'
      sketch: bigint
      vertex: bigint
      base: [number, number, number]
      /** Other endpoints of edges incident to `base`, captured once at pick time. */
      incidentOthers: [number, number, number][]
    }

function approxEqual(a: [number, number, number], b: [number, number, number]): boolean {
  const dx = a[0] - b[0]
  const dy = a[1] - b[1]
  const dz = a[2] - b[2]
  return Math.sqrt(dx * dx + dy * dy + dz * dz) < PICK_EPSILON
}

export class EditVertexTool implements Tool {
  readonly name = 'Edit Vertex'

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnVertexMoveCommit
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** THREE.js LineSegments ghost for the incident-edge preview. */
  private ghost: THREE.LineSegments | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnVertexMoveCommit,
    onToast: OnToast,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onCommit = onCommit
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
  }

  // ── Tool interface ──────────────────────────────────────────────────────

  onPointerMove(snap: Snap | null, _ray: Ray): void {
    if (this.stage.kind !== 'base' || snap === null) return
    const cursor: [number, number, number] = [snap.x, snap.y, snap.z]
    this._updateGhost(cursor)
    this._reportMeasurement(this.stage.base, cursor)
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (this.stage.kind === 'idle') {
      const pick = this.wasmScene.pick_sketch_vertex(
        ray.origin[0], ray.origin[1], ray.origin[2],
        ray.direction[0], ray.direction[1], ray.direction[2],
      )
      if (pick === undefined) return // miss — stay idle

      let sketch: bigint, vertex: bigint, base: [number, number, number]
      try {
        sketch = pick.sketch()
        vertex = pick.vertex()
        base = [pick.x(), pick.y(), pick.z()]
      } finally {
        pick.free()
      }

      const incidentOthers = this._findIncidentOthers(sketch, base)
      this.stage = { kind: 'base', sketch, vertex, base, incidentOthers }
      this._updateGhost(base)
      return
    }

    if (this.stage.kind === 'base') {
      if (snap === null) return
      const dest: [number, number, number] = [snap.x, snap.y, snap.z]
      const { sketch, vertex, base } = this.stage

      if (approxEqual(base, dest)) {
        // No movement — treat as a cancel, not a zero-length commit.
        this._resetToIdle()
        return
      }

      try {
        this.wasmScene.move_sketch_vertex(sketch, vertex, dest[0], dest[1], dest[2])
      } catch (err) {
        // The kernel refused the move (e.g. WouldRetopologize) and left the
        // sketch untouched (strong guarantee) — toast and reset, do NOT fire
        // onCommit (nothing changed; a refresh would falsely dirty the doc).
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        this._resetToIdle()
        return
      }
      this._resetToIdle()
      this.onCommit()
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
    }
  }

  cancel(): void {
    this._resetToIdle()
  }

  // ── Private helpers ─────────────────────────────────────────────────────

  /**
   * Scan `sketch_lines(sketch)` for segments with an endpoint ~= `base`,
   * collecting each one's OTHER endpoint. Used to seed the incident-edge
   * ghost without a second kernel round-trip for topology.
   */
  private _findIncidentOthers(
    sketch: bigint,
    base: [number, number, number],
  ): [number, number, number][] {
    const lines = this.wasmScene.sketch_lines(sketch)
    const others: [number, number, number][] = []
    for (let i = 0; i + 5 < lines.length; i += 6) {
      const a: [number, number, number] = [lines[i], lines[i + 1], lines[i + 2]]
      const b: [number, number, number] = [lines[i + 3], lines[i + 4], lines[i + 5]]
      if (approxEqual(a, base)) others.push(b)
      else if (approxEqual(b, base)) others.push(a)
    }
    return others
  }

  private _resetToIdle(): void {
    this.stage = { kind: 'idle' }
    this._clearGhost()
    this.onMeasurementCb('')
  }

  private _reportMeasurement(base: [number, number, number], cursor: [number, number, number]): void {
    const dx = cursor[0] - base[0]
    const dy = cursor[1] - base[1]
    const dz = cursor[2] - base[2]
    const dist = Math.sqrt(dx * dx + dy * dy + dz * dz)
    this.onMeasurementCb(formatLength(dist))
  }

  /**
   * Rebuild the ghost: one line segment per incident edge, from its captured
   * OTHER endpoint to `cursor`. Falls back to a tiny crosshair at `cursor` if
   * the picked vertex had no incident edges in the line buffer.
   */
  private _updateGhost(cursor: [number, number, number]): void {
    this._clearGhost()
    if (this.stage.kind !== 'base') return

    const { incidentOthers } = this.stage
    const pts: number[] = []
    if (incidentOthers.length > 0) {
      for (const other of incidentOthers) {
        pts.push(other[0], other[1], other[2], cursor[0], cursor[1], cursor[2])
      }
    } else {
      // Fallback marker: a small crosshair at the cursor so the drag still
      // gives visual feedback even with no incident-edge data.
      const h = 0.05
      pts.push(
        cursor[0] - h, cursor[1], cursor[2], cursor[0] + h, cursor[1], cursor[2],
        cursor[0], cursor[1] - h, cursor[2], cursor[0], cursor[1] + h, cursor[2],
        cursor[0], cursor[1], cursor[2] - h, cursor[0], cursor[1], cursor[2] + h,
      )
    }

    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(pts), 3))
    const mat = new THREE.LineBasicMaterial({ color: SKETCH_LINE_COLOR, depthTest: false })
    const line = new THREE.LineSegments(geo, mat)
    this.preview.add(line)
    this.ghost = line
  }

  private _clearGhost(): void {
    if (this.ghost === null) return
    this.ghost.geometry.dispose()
    if (this.ghost.material instanceof THREE.Material) {
      this.ghost.material.dispose()
    }
    this.preview.remove(this.ghost)
    this.ghost = null
  }
}
