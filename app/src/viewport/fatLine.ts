/**
 * fatLine — shared helper for screen-space-thick lines (Refinement pass).
 *
 * WebGL renders every `THREE.Line`/`LineSegments` at 1px regardless of
 * `linewidth`, so tool previews (rubber-band rectangles/circles/polylines) and
 * committed sketch edges were nearly invisible ("no well-defined lines showing
 * the outline"). This wraps three's `LineSegments2`/`LineMaterial` (the fat-line
 * examples module) so those overlays get a real, readable pixel width — the same
 * technique the origin axes use.
 *
 * `LineMaterial` needs its `resolution` uniform kept at the canvas pixel size or
 * the width is wrong; `updateFatLineResolutions` (called on resize/DPR change —
 * see Viewport's `ResizeObserver`) handles that, and new lines are born at the
 * last known resolution so they're correct on their very first frame.
 *
 * `updateFatLineResolutions` used to `scene.traverse()` every `Object3D` each
 * rendered frame to find the handful of `LineSegments2` materials that needed
 * it — on a large document (thousands of object/instance nodes) that walk was
 * measurable per-orbit-frame overhead just to set a uniform on a few dozen
 * materials at most. Instead every fat-line material is registered here at
 * construction (`makeFatSegments`) and dropped at disposal
 * (`disposeFatSegments`), so the update is O(live fat lines) and only runs
 * when the canvas size actually changes.
 */
import * as THREE from 'three'
import { LineSegments2 } from 'three/examples/jsm/lines/LineSegments2.js'
import { LineSegmentsGeometry } from 'three/examples/jsm/lines/LineSegmentsGeometry.js'
import { LineMaterial } from 'three/examples/jsm/lines/LineMaterial.js'

/** Last canvas size seen, so freshly-built lines start at the right width. */
const lastRes = new THREE.Vector2(1, 1)

/**
 * Every live fat-line material, keyed by the material itself. Populated by
 * `makeFatSegments`, cleared by `disposeFatSegments` — callers that build a
 * `LineSegments2` any other way won't get resolution updates (there are none
 * in this codebase; `makeFatSegments` is the sole constructor).
 */
const registry = new Set<LineMaterial>()

/** Shared style for draw-tool rubber-band previews: a bright, readable blue
 * that reads on both the dark and light ground, drawn on top (no depth test). */
export const PREVIEW_LINE_STYLE = {
  color: 0x4d90ff,
  widthPx: 2.5,
  depthTest: true,
  renderOrder: 997,
} as const

export interface FatSegmentsOpts {
  color: number
  /** Screen width in px. */
  widthPx: number
  dashed?: boolean
  opacity?: number
  depthTest?: boolean
  renderOrder?: number
  dashSize?: number
  gapSize?: number
  /** Force `transparent` (e.g. for a material whose opacity is animated later,
   * like the sketch-isolation fade). Defaults to true when opacity<1 or dashed. */
  transparent?: boolean
}

/**
 * Build a fat `LineSegments2` from flat segment-pair positions
 * (`[ax,ay,az, bx,by,bz, …]` — the same layout the tools already produce for
 * `THREE.LineSegments`).
 */
export function makeFatSegments(positions: ArrayLike<number>, opts: FatSegmentsOpts): LineSegments2 {
  const geo = new LineSegmentsGeometry()
  geo.setPositions(positions instanceof Float32Array ? positions : new Float32Array(positions))
  const mat = new LineMaterial({
    color: opts.color,
    linewidth: opts.widthPx,
    dashed: opts.dashed ?? false,
    dashSize: opts.dashSize ?? 0.2,
    gapSize: opts.gapSize ?? 0.15,
    transparent: opts.transparent ?? ((opts.opacity ?? 1) < 1 || (opts.dashed ?? false)),
    opacity: opts.opacity ?? 1,
    depthTest: opts.depthTest ?? true,
  })
  mat.resolution.copy(lastRes)
  registry.add(mat)
  const line = new LineSegments2(geo, mat)
  if (opts.dashed ?? false) line.computeLineDistances()
  if (opts.renderOrder !== undefined) line.renderOrder = opts.renderOrder
  return line
}

/** Dispose a fat line's geometry + material, and drop it from the resolution
 * registry (see the module doc comment). */
export function disposeFatSegments(obj: THREE.Object3D): void {
  if (obj instanceof LineSegments2) {
    obj.geometry.dispose()
    const mat = obj.material as LineMaterial
    registry.delete(mat)
    mat.dispose()
  }
}

/**
 * Point every registered fat-line material at the current canvas pixel size —
 * required for correct fat-line width. Call only when the canvas resolution
 * changes (mount, resize, DPR change) — NOT every render frame; a freshly
 * built line already starts at `lastRes` (see `makeFatSegments`), so nothing
 * needs a per-frame nudge.
 */
export function updateFatLineResolutions(width: number, height: number): void {
  lastRes.set(width, height)
  for (const mat of registry) {
    mat.resolution.set(width, height)
  }
}
