/**
 * RectangleTool — two-click ground-plane rectangle sketching.
 *
 * Gesture:
 *   1. First click: anchor corner (snapped)
 *   2. Move: rubber-band rectangle preview on the ground plane
 *   3. Second click: commit — begin_ground_sketch() if needed, four
 *      sketch_add_segment calls forming the rectangle
 *   4. Esc between clicks: cancel stage 1
 *
 * Calls onCommit() after each successful commit so the viewport can
 * refresh scene geometry and trigger re-render.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { rectangleCorners, parseKernelErrorCode, kernelErrorMessage } from '../viewport/geoHelpers'

export type RectangleCommitResult = {
  sketchHandle: bigint
  /** Handles of regions created by the last segment (may be empty if not yet closed) */
  regionsCreated: bigint[]
}

export type OnRectangleCommit = (result: RectangleCommitResult) => void
export type OnToast = (message: string, code?: string) => void

/** Stage 0: waiting for first click; Stage 1: waiting for second click */
type Stage = { kind: 'idle' } | { kind: 'anchored'; anchor: [number, number] }

export class RectangleTool implements Tool {
  readonly name = 'Rectangle'

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnRectangleCommit
  private onToast: OnToast

  /** Handle to the current active sketch — reused across commits if not null */
  private sketchHandle: bigint | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnRectangleCommit,
    onToast: OnToast,
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onCommit = onCommit
    this.onToast = onToast
  }

  onPointerMove(snap: Snap | null, _ray: Ray): void {
    if (this.stage.kind !== 'anchored' || snap === null) {
      this._clearPreview()
      return
    }
    const { anchor } = this.stage
    const cursor: [number, number] = [snap.x, snap.y]
    this._drawRubberBand(anchor, cursor)
  }

  onPointerDown(snap: Snap | null, _ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'idle') {
      // First click: set anchor
      this.stage = { kind: 'anchored', anchor: [snap.x, snap.y] }
    } else {
      // Second click: commit the rectangle
      const { anchor } = this.stage
      const cursor: [number, number] = [snap.x, snap.y]

      // Skip degenerate rectangles (same point or zero area)
      if (
        Math.abs(anchor[0] - cursor[0]) < 1e-8 ||
        Math.abs(anchor[1] - cursor[1]) < 1e-8
      ) {
        return
      }

      this._commitRectangle(anchor, cursor)
      this.stage = { kind: 'idle' }
      this._clearPreview()
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
    }
  }

  cancel(): void {
    this.stage = { kind: 'idle' }
    this._clearPreview()
  }

  private _commitRectangle(a: [number, number], b: [number, number]): void {
    try {
      // Begin sketch if we don't already have one
      if (this.sketchHandle === null) {
        this.sketchHandle = this.wasmScene.begin_ground_sketch()
      }
      const sketch = this.sketchHandle

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
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      const message = kernelErrorMessage(code ?? 'Unknown', rawMsg)
      this.onToast(message, code ?? undefined)
    }
  }

  private _drawRubberBand(a: [number, number], b: [number, number]): void {
    this._clearPreview()

    const corners = rectangleCorners(a, b)
    // Build a closed loop of lines for the rectangle preview
    const pts = new Float32Array([
      ...corners[0], ...corners[1],
      ...corners[1], ...corners[2],
      ...corners[2], ...corners[3],
      ...corners[3], ...corners[0],
    ])
    // Lift slightly above ground to avoid z-fighting
    for (let i = 2; i < pts.length; i += 3) pts[i] = 0.001

    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(pts, 3))
    const mat = new THREE.LineBasicMaterial({
      color: 0x2266cc,
      depthTest: false,
    })
    const lines = new THREE.LineSegments(geo, mat)
    lines.renderOrder = 997
    this.preview.add(lines)
  }

  private _clearPreview(): void {
    this.preview.traverse((child) => {
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
