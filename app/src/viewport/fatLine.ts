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
 * the width is wrong; `updateFatLineResolutions` (called each render frame over
 * the preview + sketch groups) handles that, and new lines are born at the last
 * known resolution so they're correct on their very first frame.
 */
import * as THREE from 'three'
import { LineSegments2 } from 'three/examples/jsm/lines/LineSegments2.js'
import { LineSegmentsGeometry } from 'three/examples/jsm/lines/LineSegmentsGeometry.js'
import { LineMaterial } from 'three/examples/jsm/lines/LineMaterial.js'

/** Last canvas size seen, so freshly-built lines start at the right width. */
const lastRes = new THREE.Vector2(1, 1)

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
  const line = new LineSegments2(geo, mat)
  if (opts.dashed ?? false) line.computeLineDistances()
  if (opts.renderOrder !== undefined) line.renderOrder = opts.renderOrder
  return line
}

/** Dispose a fat line's geometry + material. */
export function disposeFatSegments(obj: THREE.Object3D): void {
  if (obj instanceof LineSegments2) {
    obj.geometry.dispose()
    ;(obj.material as LineMaterial).dispose()
  }
}

/**
 * Point every `LineSegments2` material under `root` at the current canvas pixel
 * size — required for correct fat-line width. Cheap; call from the render loop.
 */
export function updateFatLineResolutions(root: THREE.Object3D, width: number, height: number): void {
  lastRes.set(width, height)
  root.traverse((o) => {
    if (o instanceof LineSegments2) (o.material as LineMaterial).resolution.set(width, height)
  })
}
