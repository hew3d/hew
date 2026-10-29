/**
 * screenPick — screen-space hit testing for lines the viewport draws itself:
 * construction guide lines and annotation segments (dimension lines,
 * leaders).
 *
 * A line is picked by its pixel distance from the click. A straight world
 * segment projects to a straight screen segment, so it is clipped to the near
 * plane in view space (the same rule `MarqueeProjector` applies to edges — a
 * point behind the camera has no meaningful screen position), then projected
 * and measured as one 2D segment. Clipping instead of dropping a segment that
 * reaches behind the camera matters when zoomed in on a guide that runs into
 * depth: the stretch on screen then lies next to the camera, right where the
 * line crosses the near plane.
 */

import * as THREE from 'three'
import { guideSpanOf, stretchToUse } from './guideWindow'

type V3 = readonly [number, number, number]

/** Stretch of an infinite guide line tested for a click, as ± metres along
 *  it from its origin. The drawn stretch follows the view
 *  (SceneRenderer.updateGuideExtents); a click can only land where the line
 *  is on screen, which this always spans. */
const GUIDE_LINE_PICK_HALF = 50

/** Distance in pixels from `(px, py)` to the 2D segment `a → b`. */
function pointSegPx(
  px: number, py: number,
  ax: number, ay: number, bx: number, by: number,
): number {
  const dx = bx - ax, dy = by - ay
  const len2 = dx * dx + dy * dy
  if (len2 < 1e-9) return Math.hypot(px - ax, py - ay)
  let t = ((px - ax) * dx + (py - ay) * dy) / len2
  t = Math.max(0, Math.min(1, t))
  return Math.hypot(px - (ax + t * dx), py - (ay + t * dy))
}

/**
 * Pixel distance from the canvas point `(clickX, clickY)` to the world
 * segment `a → b` as the camera shows it: clipped to the near plane, then
 * projected into a `width × height` canvas. `Infinity` when the whole
 * segment is behind the near plane.
 */
export function segmentPickPx(
  camera: THREE.PerspectiveCamera | THREE.OrthographicCamera,
  width: number, height: number,
  a: V3, b: V3,
  clickX: number, clickY: number,
): number {
  const va = new THREE.Vector3(a[0], a[1], a[2]).applyMatrix4(camera.matrixWorldInverse)
  const vb = new THREE.Vector3(b[0], b[1], b[2]).applyMatrix4(camera.matrixWorldInverse)
  const near = -camera.near
  const aFront = va.z <= near
  const bFront = vb.z <= near
  if (!aFront && !bFront) return Infinity
  if (!aFront || !bFront) {
    const cut = va.clone().lerp(vb, (near - va.z) / (vb.z - va.z))
    cut.z = near
    if (aFront) vb.copy(cut)
    else va.copy(cut)
  }
  va.applyMatrix4(camera.projectionMatrix)
  vb.applyMatrix4(camera.projectionMatrix)
  return pointSegPx(
    clickX, clickY,
    (va.x * 0.5 + 0.5) * width, (0.5 - va.y * 0.5) * height,
    (vb.x * 0.5 + 0.5) * width, (0.5 - vb.y * 0.5) * height,
  )
}

/**
 * Pixel distance from the canvas point `(clickX, clickY)` to a line guide,
 * from `guide_geometry`'s `[ox, oy, oz, dx, dy, dz, from, to]`. An infinite
 * guide is tested over ±GUIDE_LINE_PICK_HALF around its origin, a ray or
 * segment over its own span within that. `Infinity` when no part of it is in
 * front of the camera (or the geometry is degenerate).
 */
export function guideLinePickPx(
  camera: THREE.PerspectiveCamera | THREE.OrthographicCamera,
  width: number, height: number,
  geometry: ArrayLike<number>,
  clickX: number, clickY: number,
): number {
  const ox = geometry[0], oy = geometry[1], oz = geometry[2]
  const dx = geometry[3], dy = geometry[4], dz = geometry[5]
  const len = Math.hypot(dx, dy, dz)
  if (len < 1e-9) return Infinity
  const ux = dx / len, uy = dy / len, uz = dz / len
  const { lo, hi } = guideSpanOf(geometry)
  const stretch = stretchToUse(-GUIDE_LINE_PICK_HALF, GUIDE_LINE_PICK_HALF, lo, hi)
  if (stretch === null) return Infinity
  const [t0, t1] = stretch
  return segmentPickPx(
    camera, width, height,
    [ox + ux * t0, oy + uy * t0, oz + uz * t0],
    [ox + ux * t1, oy + uy * t1, oz + uz * t1],
    clickX, clickY,
  )
}
