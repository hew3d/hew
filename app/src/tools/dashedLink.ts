/**
 * dashedLink — the thin dashed line SketchUp draws between two points a
 * transform tool is working with: Scale's grabbed grip and the point it
 * scales about, Rotate's center and its first (0°) point, Move's base point
 * and destination. In the construction guides' gray (`GUIDE_COLOR`, read on
 * both canvases and over faces) — or, lying along a drawing axis, that
 * axis's color, as the inference shows an on-axis line — always on top, with a
 * constant on-screen dash denser than a guide's (the guides' screen-constant
 * dashing, `axisDashGapWorldFromWorldPerPixel`).
 */

import * as THREE from 'three'
import { axisDashGapWorldFromWorldPerPixel } from '../viewport/math'
import { axisColorForDirection, axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'
import { GUIDE_COLOR } from '../viewport/guideColors'
import type { DrawingAxes } from './drawingAxes'

/** On-screen dash and gap, in CSS pixels — denser than a guide's 9/5. */
const LINK_DASH_PX = 5
const LINK_GAP_PX = 3
const LINK_MIN_WORLD = 1e-5
/** The off-axis color: the guides' gray, the same in both themes (black,
 * SketchUp's, all but vanishes on the dark canvas). */
export const LINK_COLOR = GUIDE_COLOR
/** How parallel to an axis the line must be to take its color: on the axis,
 * as an axis inference or lock puts it (0.01°, float noise only) — not
 * merely close to it. */
const ON_AXIS_TOL_DOT = Math.cos((0.01 * Math.PI) / 180)

type Vec3 = readonly [number, number, number]

export class DashedLink {
  readonly line: THREE.LineSegments
  /** The drawing axes the line is colored against, read on every `set`. */
  private readonly axes: () => DrawingAxes

  constructor(axes: () => DrawingAxes) {
    this.axes = axes
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(6), 3))
    // Placeholder dash until the first `fitDash` (the next render frame).
    const mat = new THREE.LineDashedMaterial({ color: LINK_COLOR, dashSize: 0.01, gapSize: 0.006, depthTest: false })
    this.line = new THREE.LineSegments(geo, mat)
    this.line.frustumCulled = false
  }

  /** Run the line from `a` to `b`, in the color of the drawing axis it lies
   * along, else `LINK_COLOR`. */
  set(a: Vec3, b: Vec3): void {
    const match = axisColorForDirection(
      [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
      ON_AXIS_TOL_DOT,
      axisColorsForTheme(getResolvedTheme()),
      this.axes(),
    )
    ;(this.line.material as THREE.LineDashedMaterial).color.setHex(match !== null ? match.color : LINK_COLOR)
    const pos = this.line.geometry.getAttribute('position') as THREE.BufferAttribute
    pos.setXYZ(0, a[0], a[1], a[2])
    pos.setXYZ(1, b[0], b[1], b[2])
    pos.needsUpdate = true
    this.line.computeLineDistances()
    this.line.geometry.computeBoundingSphere()
  }

  /** Hold the dash at its on-screen size, measured at the line's midpoint —
   * call once per frame from the tool's screen-constant hook. */
  fitDash(camera: THREE.Camera, worldPerPixel: (dist: number) => number): void {
    const pos = this.line.geometry.getAttribute('position') as THREE.BufferAttribute
    const mid = new THREE.Vector3(
      (pos.getX(0) + pos.getX(1)) / 2,
      (pos.getY(0) + pos.getY(1)) / 2,
      (pos.getZ(0) + pos.getZ(1)) / 2,
    )
    const { dashSize, gapSize } = axisDashGapWorldFromWorldPerPixel(
      LINK_DASH_PX,
      LINK_GAP_PX,
      worldPerPixel(camera.position.distanceTo(mid)),
      LINK_MIN_WORLD,
    )
    const mat = this.line.material as THREE.LineDashedMaterial
    if (mat.dashSize === dashSize && mat.gapSize === gapSize) return
    mat.dashSize = dashSize
    mat.gapSize = gapSize
    mat.needsUpdate = true
  }

  dispose(): void {
    this.line.geometry.dispose()
    ;(this.line.material as THREE.Material).dispose()
  }
}
