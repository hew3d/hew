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

/** One candidate spot on a picked segment: its pixel distance from the click
 *  and the world point there. */
export interface PickSample {
  px: number
  point: V3
}

/** Screen offsets (pixels along the segment, from the foot of the
 *  perpendicular) at which a pick also samples the segment. */
const SAMPLE_OFFSETS_PX = [3, -3, 6, -6]

/**
 * Points on the world segment `a -> b` within `radiusPx` of the click,
 * nearest first: the foot of the perpendicular (its `px` is exactly
 * `segmentPickPx`), then +-3 px and +-6 px along the screen segment, clamped
 * to the near-clipped segment with duplicates dropped. The world point of a
 * screen position uses the endpoints' clip-space `w` (perspective-correct); a
 * parallel camera has `w = 1`, so it is a plain lerp. Empty when the segment
 * is entirely behind the near plane or no spot is within the radius.
 */
export function segmentPickSamples(
  camera: THREE.PerspectiveCamera | THREE.OrthographicCamera,
  width: number, height: number,
  a: V3, b: V3,
  clickX: number, clickY: number,
  radiusPx: number,
): PickSample[] {
  const va = new THREE.Vector3(a[0], a[1], a[2]).applyMatrix4(camera.matrixWorldInverse)
  const vb = new THREE.Vector3(b[0], b[1], b[2]).applyMatrix4(camera.matrixWorldInverse)
  const near = -camera.near
  const aFront = va.z <= near
  const bFront = vb.z <= near
  if (!aFront && !bFront) return []
  if (!aFront || !bFront) {
    const cut = va.clone().lerp(vb, (near - va.z) / (vb.z - va.z))
    cut.z = near
    if (aFront) vb.copy(cut)
    else va.copy(cut)
  }
  const ca = new THREE.Vector4(va.x, va.y, va.z, 1).applyMatrix4(camera.projectionMatrix)
  const cb = new THREE.Vector4(vb.x, vb.y, vb.z, 1).applyMatrix4(camera.projectionMatrix)
  const wa = ca.w, wb = cb.w
  const ax = (ca.x / wa * 0.5 + 0.5) * width, ay = (0.5 - ca.y / wa * 0.5) * height
  const bx = (cb.x / wb * 0.5 + 0.5) * width, by = (0.5 - cb.y / wb * 0.5) * height
  const dx = bx - ax, dy = by - ay
  const len = Math.hypot(dx, dy)

  const at = (t: number): PickSample => {
    const s = t * wa / (t * wa + (1 - t) * wb)
    const v = va.clone().lerp(vb, s).applyMatrix4(camera.matrixWorld)
    const sx = ax + t * dx, sy = ay + t * dy
    return { px: Math.hypot(sx - clickX, sy - clickY), point: [v.x, v.y, v.z] }
  }

  if (len < 1e-6) {
    const only = at(0)
    return only.px <= radiusPx ? [only] : []
  }
  const foot = Math.max(0, Math.min(1, ((clickX - ax) * dx + (clickY - ay) * dy) / (len * len)))
  const ts = [foot]
  for (const off of SAMPLE_OFFSETS_PX) {
    const t = Math.max(0, Math.min(1, foot + off / len))
    if (!ts.some((u) => Math.abs(u - t) * len < 0.5)) ts.push(t)
  }
  return ts.map(at).filter((s) => s.px <= radiusPx).sort((p, q) => p.px - q.px)
}

/** `segmentPickSamples` over a line guide's tested stretch (see
 *  `guideLinePickPx`). */
export function guideLinePickSamples(
  camera: THREE.PerspectiveCamera | THREE.OrthographicCamera,
  width: number, height: number,
  geometry: ArrayLike<number>,
  clickX: number, clickY: number,
  radiusPx: number,
): PickSample[] {
  const ox = geometry[0], oy = geometry[1], oz = geometry[2]
  const dx = geometry[3], dy = geometry[4], dz = geometry[5]
  const len = Math.hypot(dx, dy, dz)
  if (len < 1e-9) return []
  const ux = dx / len, uy = dy / len, uz = dz / len
  const { lo, hi } = guideSpanOf(geometry)
  const stretch = stretchToUse(-GUIDE_LINE_PICK_HALF, GUIDE_LINE_PICK_HALF, lo, hi)
  if (stretch === null) return []
  const [t0, t1] = stretch
  return segmentPickSamples(
    camera, width, height,
    [ox + ux * t0, oy + uy * t0, oz + uz * t0],
    [ox + ux * t1, oy + uy * t1, oz + uz * t1],
    clickX, clickY, radiusPx,
  )
}
