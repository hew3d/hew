/**
 * DrawPlaneCueLayer — the drawing-plane cue at the cursor (view-die design
 * §4), drawn from the active draw tool's `drawPlaneState()`:
 *
 * | Cursor is over | Cue |
 * |---|---|
 * | The ground, no hold | none (the world grid is the cue) |
 * | A face or a sketch | none |
 * | A corner or edge, leaning onto a face of the object | the face's outer boundary, 1.5 px, in the plane's color |
 * | Empty space on an inferred (non-ground) plane, or an extended lean off the object | the small patch |
 * | Any held state (locked, pinned, anchored) | the small patch, held weight |
 *
 * The small patch is a 0.5 m square in the plane with one cross line each
 * way (2×2 cells), centered on the hover ray's hit, its on-screen size
 * clamped to 36–72 css px every frame (`applyScale`, from the mesh's own
 * `onBeforeRender`, so a zoom with the cursor still resizes it). Lines are
 * 1 px at 0.4 alpha, 1.5 px at 0.6 alpha when held. A curved face, which
 * has no planar loop to outline, gets the patch instead.
 *
 * Colored by the plane's pip (`planePipColor`): the drawing axes' red,
 * green or blue, or the custom violet. Purely visual: never a snap
 * participant, never pickable. The patch opts out of depth testing (it sits
 * at the cursor, wherever the plane is); the outline lies on the face's own
 * edges and wins their depth tie with the preview rung of the depth ladder.
 *
 * The Viewport calls `update()` with `planeCueFor(tool.drawPlaneState())`
 * after every pointer move, key and pin change, and `clear()` on tool
 * switch, document reset and unmount.
 */
import * as THREE from 'three'
import type { LineSegments2 } from 'three/examples/jsm/lines/LineSegments2.js'
import { isGroundPlane, type DrawPlane, type DrawPlaneState, type PlanePip } from '../tools/drawPlane'
import type { V3 } from './geoHelpers'
import { planePipColor } from './axisColors'
import { getResolvedTheme } from '../settings/theme'
import { cameraWorldPerPixelAt } from './annotationLayout'
import { DEPTH_BIAS } from './depthPolicy'
import { disposeFatSegments, makeFatSegments } from './fatLine'

/** The small patch's nominal side (meters), before the screen clamp. */
export const PATCH_SIZE_M = 0.5
/** The small patch's on-screen side is clamped to this range (css px). */
export const PATCH_MIN_PX = 36
export const PATCH_MAX_PX = 72

/** Line weights: an inferred patch, a held patch, and the face outline. */
export const PATCH_LINE = { widthPx: 1, opacity: 0.4 } as const
export const PATCH_LINE_HELD = { widthPx: 1.5, opacity: 0.6 } as const
export const OUTLINE_LINE = { widthPx: 1.5, opacity: 1 } as const

/** What the layer draws: a face outline or the small patch. */
export type PlaneCueSpec =
  | { kind: 'outline'; pip: PlanePip; normal: V3; loop: V3[] }
  | { kind: 'patch'; pip: PlanePip; plane: DrawPlane; through: V3; held: boolean }

/**
 * The cue for a draw tool's plane state (the table in the module doc), or
 * null for none: no state (not a draw tool), no hover (the pointer is off
 * the canvas), the ground with no hold, a plain face, or a sketch.
 */
export function planeCueFor(state: DrawPlaneState | null): PlaneCueSpec | null {
  if (state === null || state.hover === null) return null
  const { plane, pip, hover } = state
  if (state.hold !== 'inferred') return { kind: 'patch', pip, plane, through: hover.point, held: true }
  switch (hover.kind) {
    case 'ground':
    case 'face':
    case 'sketch':
      return null
    case 'lean':
      return hover.faceLoop !== undefined && hover.faceLoop.length >= 3
        ? { kind: 'outline', pip, normal: plane.normal, loop: hover.faceLoop }
        : { kind: 'patch', pip, plane, through: hover.point, held: false }
    case 'leanExtended':
      return { kind: 'patch', pip, plane, through: hover.point, held: false }
    case 'space':
      // Empty space: the inferred plane, unless it is the ground the world
      // grid already shows (a moved frame's blue plane can coincide with it).
      return plane.ground || isGroundPlane(plane.origin, plane.normal)
        ? null
        : { kind: 'patch', pip, plane, through: hover.point, held: false }
  }
}

/** The patch's side in world units at `worldPerPixel`: 0.5 m, clamped so
 *  it spans 36–72 css px on screen. A non-positive or non-finite scale
 *  keeps the nominal size. */
export function patchWorldSize(worldPerPixel: number): number {
  if (!(worldPerPixel > 0) || !Number.isFinite(worldPerPixel)) return PATCH_SIZE_M
  return Math.min(Math.max(PATCH_SIZE_M, PATCH_MIN_PX * worldPerPixel), PATCH_MAX_PX * worldPerPixel)
}

/** The patch's unit line set: three lines each way across a 2×2 square of
 *  half-side 1 in the plane's (u, v) basis, centered on the origin. The
 *  object's position and uniform scale place and size it. */
function unitPatchPositions(plane: DrawPlane): Float32Array {
  const [ux, uy, uz] = plane.u
  const [vx, vy, vz] = plane.v
  const pts: number[] = []
  for (const s of [-1, 0, 1]) {
    // Parallel to v at u = s, then parallel to u at v = s.
    pts.push(ux * s - vx, uy * s - vy, uz * s - vz, ux * s + vx, uy * s + vy, uz * s + vz)
    pts.push(vx * s - ux, vy * s - uy, vz * s - uz, vx * s + ux, vy * s + uy, vz * s + uz)
  }
  return new Float32Array(pts)
}

/** Closed-loop segment pairs through `loop`. */
function loopPositions(loop: readonly V3[]): Float32Array {
  const pts: number[] = []
  for (let i = 0; i < loop.length; i++) {
    const a = loop[i]
    const b = loop[(i + 1) % loop.length]
    pts.push(a[0], a[1], a[2], b[0], b[1], b[2])
  }
  return new Float32Array(pts)
}

const scratchSize = new THREE.Vector2()

export class DrawPlaneCueLayer {
  readonly group: THREE.Group
  private line: LineSegments2 | null = null
  private shown: { kind: 'outline' | 'patch'; normal: V3; through: V3; held: boolean } | null = null

  /** The cue drawn right now, or null: its kind, the plane normal, a point
   *  it is drawn through (the patch's center, the outline's centroid), and
   *  whether it is held weight. Observe-only (the E2E harness reads it). */
  get current(): { kind: 'outline' | 'patch'; normal: V3; through: V3; held: boolean } | null {
    return this.shown === null ? null : { ...this.shown, normal: [...this.shown.normal], through: [...this.shown.through] }
  }

  constructor() {
    this.group = new THREE.Group()
    this.group.name = 'DrawPlaneCueLayer'
  }

  /** Rebuild the cue for `spec` (`planeCueFor` of the active tool's plane
   *  state), or clear it when `spec` is null. */
  update(spec: PlaneCueSpec | null): void {
    this._dispose()
    if (spec === null) return
    const color = planePipColor(spec.pip, getResolvedTheme())
    if (spec.kind === 'outline') {
      const line = makeFatSegments(loopPositions(spec.loop), {
        color,
        widthPx: OUTLINE_LINE.widthPx,
        opacity: OUTLINE_LINE.opacity,
        depthTest: true,
        depthBias: DEPTH_BIAS.PREVIEW,
        renderOrder: 996,
      })
      line.name = 'DrawPlaneCueOutline'
      this.line = line
      this.group.add(line)
      const n = spec.loop.length
      const c: V3 = [0, 0, 0]
      for (const p of spec.loop) { c[0] += p[0] / n; c[1] += p[1] / n; c[2] += p[2] / n }
      this.shown = { kind: 'outline', normal: [...spec.normal], through: c, held: false }
      return
    }
    const weight = spec.held ? PATCH_LINE_HELD : PATCH_LINE
    const line = makeFatSegments(unitPatchPositions(spec.plane), {
      color,
      widthPx: weight.widthPx,
      opacity: weight.opacity,
      depthTest: false,
      renderOrder: 996,
    })
    line.name = 'DrawPlaneCuePatch'
    line.position.set(spec.through[0], spec.through[1], spec.through[2])
    line.scale.setScalar(PATCH_SIZE_M / 2)
    // Size the patch for the camera of every frame it is drawn in, after the
    // fat line's own resolution hook.
    const through = spec.through
    const base = line.onBeforeRender.bind(line)
    const hook: THREE.Object3D['onBeforeRender'] = (renderer, _scene, camera) => {
      base(renderer)
      renderer.getSize(scratchSize)
      if (this.applyScale(camera, scratchSize.y, through)) line.updateMatrixWorld()
    }
    ;(line as THREE.Object3D).onBeforeRender = hook
    this.line = line
    this.group.add(line)
    this.shown = { kind: 'patch', normal: [...spec.plane.normal], through: [...spec.through], held: spec.held }
  }

  /**
   * Size the patch for `camera` on a viewport `heightPx` css px tall
   * (`patchWorldSize` at the patch's center). Returns whether the scale
   * changed; a no-op without a patch.
   */
  applyScale(camera: THREE.Camera, heightPx: number, through?: V3): boolean {
    const line = this.line
    if (line === null || this.shown === null || this.shown.kind !== 'patch') return false
    const at = through ?? this.shown.through
    const half = patchWorldSize(cameraWorldPerPixelAt(camera, at, heightPx)) / 2
    if (line.scale.x === half) return false
    line.scale.setScalar(half)
    return true
  }

  /** The patch's current side in world units, or null without a patch
   *  (test observation). */
  get patchSize(): number | null {
    return this.line === null || this.shown?.kind !== 'patch' ? null : this.line.scale.x * 2
  }

  /** Dispose and hide the cue — tool switch, cancel, document reset. */
  clear(): void {
    this._dispose()
  }

  private _dispose(): void {
    this.shown = null
    if (this.line === null) return
    disposeFatSegments(this.line)
    this.group.remove(this.line)
    this.line = null
  }
}
