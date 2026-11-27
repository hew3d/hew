/**
 * deformMath — the pure geometry of the Deform tool: which face of the box
 * the ray hits, the deform frame a picked face defines, and the amount a
 * pointer drag asks for in each mode. No three.js, no wasm.
 *
 * World meters and radians throughout (01-conventions §10). The frame is the
 * kernel's `DeformFrame`: `origin` is the center of the FIXED end face,
 * `axis` runs from it to the moving end, `u` is the cross axis Bend bends
 * toward, and `length` is the box extent along `axis`.
 */

import type { Ray } from '../viewport/math'
import {
  crossV3,
  dotV3,
  perpComponentV3,
  rayPlaneIntersect,
  subV3,
  type V3,
} from '../viewport/geoHelpers'

export type { V3 }
/** The three modes a box face drives (the fourth, Bend to Path, uses a path). */
export type BoxMode = 'twist' | 'bend' | 'taper'

export const FOUR_PI = Math.PI * 4
export const TWO_PI = Math.PI * 2
/** Bounds on a pointer-driven taper ratio (the kernel needs a ratio > 0). */
export const MIN_TAPER = 0.02
export const MAX_TAPER = 20

/** One face of the world-axis box: the axis it is perpendicular to, which
 *  side, its center and its outward unit normal. */
export interface BoxFace {
  axisIndex: 0 | 1 | 2
  side: 0 | 1
  center: V3
  normal: V3
}

export function boxFaces(min: V3, max: V3): BoxFace[] {
  const c: V3 = [(min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2]
  const out: BoxFace[] = []
  for (const axisIndex of [0, 1, 2] as const) {
    for (const side of [0, 1] as const) {
      const center: V3 = [...c]
      center[axisIndex] = side === 0 ? min[axisIndex] : max[axisIndex]
      const normal: V3 = [0, 0, 0]
      normal[axisIndex] = side === 0 ? -1 : 1
      out.push({ axisIndex, side, center, normal })
    }
  }
  return out
}

/** The box face the ray enters through (slab method), or null on a miss or
 *  when the ray starts inside the box. */
export function rayBoxFace(ray: Ray, min: V3, max: V3): BoxFace | null {
  let tEnter = -Infinity
  let tExit = Infinity
  let enter: { axisIndex: 0 | 1 | 2; side: 0 | 1 } | null = null
  for (const i of [0, 1, 2] as const) {
    const o = ray.origin[i]
    const d = ray.direction[i]
    if (Math.abs(d) < 1e-12) {
      if (o < min[i] || o > max[i]) return null
      continue
    }
    // Through the min face when moving in +i, through the max face otherwise.
    const side: 0 | 1 = d > 0 ? 0 : 1
    const t0 = ((side === 0 ? min[i] : max[i]) - o) / d
    const t1 = ((side === 0 ? max[i] : min[i]) - o) / d
    if (t0 > tEnter) {
      tEnter = t0
      enter = { axisIndex: i, side }
    }
    tExit = Math.min(tExit, t1)
    if (tEnter > tExit) return null
  }
  if (enter === null || tEnter < 0) return null
  return boxFaces(min, max).find((f) => f.axisIndex === enter.axisIndex && f.side === enter.side) ?? null
}

export interface DeformFrame {
  origin: V3
  axis: V3
  u: V3
  length: number
}

/** The cross axes (unit world vectors) perpendicular to a face's axis. */
export function crossAxes(axisIndex: 0 | 1 | 2): [V3, V3] {
  const a = ((axisIndex + 1) % 3) as 0 | 1 | 2
  const b = ((axisIndex + 2) % 3) as 0 | 1 | 2
  const ua: V3 = [0, 0, 0]
  const ub: V3 = [0, 0, 0]
  ua[a] = 1
  ub[b] = 1
  return [ua, ub]
}

/**
 * The frame for a picked MOVING face: the fixed end is the opposite face,
 * the axis points from it to the picked one. `u` defaults to the first cross
 * axis; Bend replaces it once the drag direction is known.
 */
export function frameForFace(min: V3, max: V3, face: BoxFace, u?: V3): DeformFrame {
  const faces = boxFaces(min, max)
  const fixed = faces.find((f) => f.axisIndex === face.axisIndex && f.side !== face.side)!
  const length = max[face.axisIndex] - min[face.axisIndex]
  return {
    origin: fixed.center,
    axis: face.normal,
    u: u ?? crossAxes(face.axisIndex)[0],
    length,
  }
}

/** The moving face's center: where the drag's measuring plane sits. */
export function movingCenter(frame: DeformFrame): V3 {
  return [
    frame.origin[0] + frame.axis[0] * frame.length,
    frame.origin[1] + frame.axis[1] * frame.length,
    frame.origin[2] + frame.axis[2] * frame.length,
  ]
}

/** The ray's hit on the moving face's plane (also behind the eye), or null
 *  when the ray runs parallel to it. */
export function hitMovingPlane(frame: DeformFrame, ray: Ray): V3 | null {
  const c = movingCenter(frame)
  const fwd = rayPlaneIntersect(ray.origin as V3, ray.direction as V3, c, frame.axis)
  if (fwd !== null) return fwd
  const denom = dotV3(ray.direction as V3, frame.axis)
  if (Math.abs(denom) < 1e-10) return null
  const t = dotV3(subV3(c, ray.origin as V3), frame.axis) / denom
  return [
    ray.origin[0] + t * ray.direction[0],
    ray.origin[1] + t * ray.direction[1],
    ray.origin[2] + t * ray.direction[2],
  ]
}

/** Angle of `p` around the axis through `c`, in the (u, axis×u) basis. */
export function angleAround(frame: DeformFrame, c: V3, p: V3): number | null {
  const d = perpComponentV3(subV3(p, c), frame.axis)
  if (Math.hypot(d[0], d[1], d[2]) < 1e-9) return null
  const v = crossV3(frame.axis, frame.u)
  return Math.atan2(dotV3(d, v), dotV3(d, frame.u))
}

/**
 * Accumulates the angle the pointer sweeps around the axis, unwrapping each
 * step by the shortest arc so a drag can turn past a half turn. The twist is
 * the swept total, clamped to ±4π.
 */
export class TwistTracker {
  private last: number | null = null
  total = 0

  update(angle: number | null): number {
    if (angle === null) return this.total
    if (this.last !== null) {
      let delta = angle - this.last
      while (delta > Math.PI) delta -= TWO_PI
      while (delta < -Math.PI) delta += TWO_PI
      this.total = Math.max(-FOUR_PI, Math.min(FOUR_PI, this.total + delta))
    }
    this.last = angle
    return this.total
  }
}

/** Distance from `p` to the axis line through `c` along the frame's axis. */
export function radiusFromAxis(frame: DeformFrame, c: V3, p: V3): number {
  const d = perpComponentV3(subV3(p, c), frame.axis)
  return Math.hypot(d[0], d[1], d[2])
}

/** The taper ratio of a pointer at radius `r` against the grab radius. */
export function taperRatio(r: number, grabRadius: number): number {
  if (!(grabRadius > 1e-9)) return 1
  return Math.max(MIN_TAPER, Math.min(MAX_TAPER, r / grabRadius))
}

/** The bend angle for a pointer displacement `d`: `2·(d·u)/L`, clamped to ±2π. */
export function bendAngle(d: V3, u: V3, length: number): number {
  if (!(length > 1e-9)) return 0
  return Math.max(-TWO_PI, Math.min(TWO_PI, (2 * dotV3(d, u)) / length))
}

/** Pick `u` for a bend: the box cross axis closest to the drag direction `d`. */
export function bendCrossAxis(d: V3, axisIndex: 0 | 1 | 2): V3 {
  const [a, b] = crossAxes(axisIndex)
  return Math.abs(dotV3(d, a)) >= Math.abs(dotV3(d, b)) ? a : b
}

/** The identity amount of a mode (no deformation). */
export function identityAmount(mode: BoxMode): number {
  return mode === 'taper' ? 1 : 0
}

/**
 * Clamp `amount` to the kernel's `limit` (the largest amount, same side of
 * the identity, that works). Returns the clamped amount and whether it moved.
 * The result sits a hair inside the limit, where the kernel accepts it.
 */
export function clampToLimit(mode: BoxMode, amount: number, limit: number): { amount: number; clamped: boolean } {
  const id = identityAmount(mode)
  if (Math.abs(amount - id) <= Math.abs(limit - id)) return { amount, clamped: false }
  return { amount: id + (limit - id) * (1 - 1e-6), clamped: true }
}

/** The JSON `deform*` takes for Twist, Bend and Taper. */
export function boxSpecJson(mode: BoxMode, frame: DeformFrame, amount: number, stations?: number): string {
  const spec: Record<string, unknown> = { mode, frame }
  if (mode === 'taper') spec.ratio = amount
  else spec.angle = amount
  if (stations !== undefined && stations > 0) spec.stations = stations
  return JSON.stringify(spec)
}

/** The JSON for Bend to Path. */
export function pathSpecJson(base: { from: V3; to: V3 }, path: V3[], closed: boolean, stations?: number): string {
  const spec: Record<string, unknown> = {
    mode: 'bend_to_path',
    base_from: base.from,
    base_to: base.to,
    path,
    closed,
  }
  if (stations !== undefined && stations > 0) spec.stations_per_segment = stations
  return JSON.stringify(spec)
}
