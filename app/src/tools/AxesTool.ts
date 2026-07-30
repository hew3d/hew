/**
 * AxesTool — SketchUp-style Axes: reposition the document's movable drawing
 * axes (tool-parity design §4) by three inference-snapped clicks: the new
 * origin, a point along the new red (X) axis, and a point along the new
 * green (Y) axis. The kernel derives blue (Z = X × Y) and is the sole
 * authority on validity (`Scene::set_axes` refuses a non-finite or
 * non-orthonormal candidate) — this tool's own math only has to produce an
 * orthonormal X/Y pair from three arbitrary picks, via Gram-Schmidt, so that
 * refusal is never actually reachable in practice (see `_commit`'s catch,
 * kept anyway as the same defensive idiom every other `scene.*`-calling tool
 * uses).
 *
 * Gesture (three clicks, no drag, no selection — mirrors ProtractorTool's
 * shape far more than Move/Rotate's copy/array machinery, which doesn't
 * apply here):
 *   1. Click 1 — origin. Advances to `origin-picked`.
 *   2. Click 2 — a point along the new X axis. Degenerate (coincides with
 *      the origin) clicks are ignored and the stage stays put — mirrors
 *      ProtractorTool's baseline-click guard ("if that projection is
 *      ~zero... the click is ignored").
 *   3. Click 3 — a point that, together with the first two, spans a plane:
 *      `x = normalize(xPoint − origin)`, `n = cross(x, yPoint − origin)`
 *      (the plane normal implied by the three picks), `y = normalize(cross(n,
 *      x))` — Gram-Schmidt against `x`, so `y` lands in the same half-plane
 *      as the user's raw third click rather than merely "some perpendicular
 *      direction". A click colinear with the first two (n ~ zero) is
 *      degenerate the same way and is ignored, with a status hint steering
 *      the user off the line. On a valid third click, `set_axes` commits and
 *      the tool returns to idle.
 *
 * Escape steps back exactly ONE stage (x-picked → origin-picked → idle),
 * matching FollowMeTool's Escape convention (see its `onKey`'s Escape
 * block) rather than ProtractorTool's full-cancel-on-Escape — re-aiming the
 * X or Y pick shouldn't throw away an already-good origin. `cancel()` (tool
 * switch) still fully resets to idle.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { V3 } from '../viewport/geoHelpers'
import { normalize3 } from './transformMath'
import { axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'

export type OnAxesCommitted = () => void
export type OnToast = (message: string, code?: string) => void

type Stage =
  | { kind: 'idle' }
  | { kind: 'origin-picked'; origin: V3 }
  | { kind: 'x-picked'; origin: V3; xPoint: V3 }

/** a × b. */
function cross(a: V3, b: V3): V3 {
  return [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ]
}

/** Half-length of the preview line drawn while picking X or Y. */
const PREVIEW_LEN = 50

export class AxesTool implements Tool {
  readonly name = 'Axes'

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    switch (this.stage.kind) {
      case 'origin-picked':
        return 'Click a point along the new red (X) axis.'
      case 'x-picked':
        return 'Click a point along the new green (Y) axis — not in line with the first two.'
      default:
        return 'Click a point for the new origin.'
    }
  }

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnAxesCommitted
  private onToast: OnToast

  /** THREE.js LineSegments for the in-progress axis preview (origin → last
   *  snapped cursor point), colored red while picking X and green while
   *  picking Y. Null while idle (nothing to preview yet). */
  private previewLine: THREE.LineSegments | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnAxesCommitted,
    onToast: OnToast,
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onCommit = onCommit
    this.onToast = onToast
  }

  // ── Tool interface ──────────────────────────────────────────────────────

  capturingInput(): boolean {
    return this.stage.kind !== 'idle'
  }

  onPointerMove(snap: Snap | null, _ray: Ray): void {
    if (snap === null || this.stage.kind === 'idle') return
    const cursor: V3 = [snap.x, snap.y, snap.z]
    const origin = this.stage.origin
    // Colored by which axis is currently being aimed: red while the X pick
    // is pending, green while the Y pick is pending — matches the axes'
    // own red/green convention, not a hover-inferred axis match (there's
    // nothing to infer here; the color just names "what click 2/3 sets").
    const color = axisColorsForTheme(getResolvedTheme())[this.stage.kind === 'origin-picked' ? 0 : 1]
    this._updatePreviewLine(origin, cursor, color)
  }

  onPointerDown(snap: Snap | null, _ray: Ray): void {
    if (snap === null) return
    const point: V3 = [snap.x, snap.y, snap.z]

    if (this.stage.kind === 'idle') {
      this.stage = { kind: 'origin-picked', origin: point }
      return
    }

    if (this.stage.kind === 'origin-picked') {
      const { origin } = this.stage
      // Degenerate: the X pick coincides with the origin — nothing to aim
      // at, ignore and stay put (mirrors ProtractorTool's baseline guard).
      const rel: V3 = [point[0] - origin[0], point[1] - origin[1], point[2] - origin[2]]
      if (normalize3(rel) === null) return
      this.stage = { kind: 'x-picked', origin, xPoint: point }
      return
    }

    // x-picked: this click sets the Y direction (and, together with the
    // first two, the plane the new frame lies in).
    const { origin, xPoint } = this.stage
    const x = normalize3([xPoint[0] - origin[0], xPoint[1] - origin[1], xPoint[2] - origin[2]])
    if (x === null) return // unreachable — excluded when entering x-picked

    const yRaw: V3 = [point[0] - origin[0], point[1] - origin[1], point[2] - origin[2]]
    const n = normalize3(cross(x, yRaw))
    if (n === null) {
      // The Y pick is colinear with origin/xPoint — no plane is implied.
      // Ignore the click and stay in x-picked (mirrors the origin-picked
      // degenerate guard above); the status hint already steers off the line.
      return
    }
    // Gram-Schmidt: y ⊥ x, in the same half-plane as the user's raw pick.
    const y = normalize3(cross(n, x))
    if (y === null) return // unreachable — n and x are both unit and ⊥

    this._commit(origin, x, y)
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key !== 'Escape') return
    // Step back exactly one stage — FollowMeTool's Escape convention, not
    // ProtractorTool's full-cancel: re-aiming X or Y shouldn't discard an
    // already-good origin.
    if (this.stage.kind === 'x-picked') {
      this.stage = { kind: 'origin-picked', origin: this.stage.origin }
      this._clearPreviewLine()
      return
    }
    if (this.stage.kind === 'origin-picked') {
      this.stage = { kind: 'idle' }
      this._clearPreviewLine()
      return
    }
    this.cancel()
  }

  cancel(): void {
    this.stage = { kind: 'idle' }
    this._clearPreviewLine()
  }

  // ── Private helpers ─────────────────────────────────────────────────────

  /**
   * Commit the new frame. The kernel derives blue (z = x×y) and validates
   * orthonormality itself — this tool's own Gram-Schmidt already guarantees
   * a valid orthonormal x/y pair, so the catch below is unreachable in
   * practice but kept as the same defensive idiom every other
   * `scene.*`-calling tool uses (mirrors TapeMeasureTool's
   * `_commitParallelGuide`/`confirmRescale`).
   */
  private _commit(origin: V3, x: V3, y: V3): void {
    try {
      this.wasmScene.set_axes(origin[0], origin[1], origin[2], x[0], x[1], x[2], y[0], y[1], y[2])
      this.onCommit()
    } catch (err) {
      const raw = err instanceof Error ? err.message : String(err)
      this.onToast(`Couldn't move the drawing axes: ${raw}`)
    }
    this.cancel()
  }

  /** Rebuild the preview line: origin → cursor, colored `color`. Removes the
   *  previous preview (if any) first — same shape as ProtractorTool's. */
  private _updatePreviewLine(origin: V3, cursor: V3, color: number): void {
    this._clearPreviewLine()

    const dx = cursor[0] - origin[0]
    const dy = cursor[1] - origin[1]
    const dz = cursor[2] - origin[2]
    const len = Math.sqrt(dx * dx + dy * dy + dz * dz)
    const [ex, ey, ez] = len < 1e-9
      ? cursor
      : [
          origin[0] + (dx / len) * PREVIEW_LEN,
          origin[1] + (dy / len) * PREVIEW_LEN,
          origin[2] + (dz / len) * PREVIEW_LEN,
        ]

    const pts = new Float32Array([origin[0], origin[1], origin[2], ex, ey, ez])
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(pts, 3))
    const mat = new THREE.LineBasicMaterial({ color, depthTest: false })
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
