/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * revolveMath — the pure geometry of the Revolve tool: is an axis usable,
 * what angle is the pointer asking for, how many segments does a turn get,
 * and the spec the kernel takes. No three.js, no wasm.
 *
 * Everything is world meters and radians (01-conventions §10).
 */

import { segmentsPerTurn } from './arcMath'
import type { Ray } from '../viewport/math'
import {
  addV3,
  crossV3,
  dotV3,
  normalizeV3,
  perpComponentV3,
  rayPlaneIntersect,
  subV3,
  type V3,
} from '../viewport/geoHelpers'

export type { V3 }

export interface PlaneDef {
  point: V3
  normal: V3
}

/** How far an axis endpoint may sit off the profile plane and still count as
 *  in it, in meters. The kernel's own plane tolerance is tighter; this only
 *  absorbs f32 round trips of picked points. */
export const AXIS_PLANE_TOL = 1e-5

/** Shortest usable axis, meters. */
export const MIN_AXIS_LEN = 1e-6

/** Whole-turn default and the range a typed angle may take (2π each way). */
export const FULL_TURN = Math.PI * 2

/** Angle snap: 15° steps, within 3° of a step. */
export const ANGLE_STEP = (15 * Math.PI) / 180
export const ANGLE_SNAP_TOL = (3 * Math.PI) / 180

/** The segment counts a turn may take (the kernel's `3..=1024`), checked
 *  as the count is typed so the refusal names the range at once. */
export const MIN_SEGMENTS = 3
export const MAX_SEGMENTS = 1024
export const SEGMENTS_RANGE = `A turn takes ${MIN_SEGMENTS} to ${MAX_SEGMENTS} segments.`
export function segmentsInRange(n: number): boolean {
  return Number.isInteger(n) && n >= MIN_SEGMENTS && n <= MAX_SEGMENTS
}

/** The axis line through `a` and `b`, or null when they coincide. */
export function axisFromPoints(a: V3, b: V3): { point: V3; dir: V3 } | null {
  if (Math.hypot(b[0] - a[0], b[1] - a[1], b[2] - a[2]) < MIN_AXIS_LEN) return null
  const dir = normalizeV3(subV3(b, a))
  return dir === null ? null : { point: a, dir }
}

/** Whether both axis points lie in `plane`. */
export function axisInPlane(a: V3, b: V3, plane: PlaneDef, tol = AXIS_PLANE_TOL): boolean {
  const n = normalizeV3(plane.normal)
  if (n === null) return false
  return (
    Math.abs(dotV3(subV3(a, plane.point), n)) <= tol && Math.abs(dotV3(subV3(b, plane.point), n)) <= tol
  )
}

/** Distance from `p` to the line through `point` along unit `dir`. */
export function distToAxis(p: V3, point: V3, dir: V3): number {
  const v = perpComponentV3(subV3(p, point), dir)
  return Math.hypot(v[0], v[1], v[2])
}

/**
 * A representative point of the profile to measure the angle from: the mean
 * of the ring vertices, or the vertex farthest from the axis when the mean
 * sits on the axis (a profile symmetric about it).
 */
export function referencePoint(ring: readonly V3[], axisPoint: V3, axisDir: V3): V3 | null {
  if (ring.length === 0) return null
  const mean: V3 = [0, 0, 0]
  for (const p of ring) {
    mean[0] += p[0]
    mean[1] += p[1]
    mean[2] += p[2]
  }
  mean[0] /= ring.length
  mean[1] /= ring.length
  mean[2] /= ring.length
  if (distToAxis(mean, axisPoint, axisDir) > 1e-9) return mean
  let best: V3 | null = null
  let bestD = 1e-9
  for (const p of ring) {
    const d = distToAxis(p, axisPoint, axisDir)
    if (d > bestD) {
      bestD = d
      best = p
    }
  }
  return best
}

/** The frame the angle is measured in: `g` on the axis, `e1` toward the
 *  profile, `e2 = axis × e1` (so a positive angle turns right-handed about
 *  the axis). */
export interface RevolveFrame {
  g: V3
  axis: V3
  e1: V3
  e2: V3
}

export function revolveFrame(axisPoint: V3, axisDir: V3, reference: V3): RevolveFrame | null {
  const axis = normalizeV3(axisDir)
  if (axis === null) return null
  const toRef = subV3(reference, axisPoint)
  const along = dotV3(toRef, axis)
  const g = addV3(axisPoint, [axis[0] * along, axis[1] * along, axis[2] * along])
  const e1 = normalizeV3(perpComponentV3(toRef, axis))
  if (e1 === null) return null
  const e2 = crossV3(axis, e1)
  return { g, axis, e1, e2 }
}

/**
 * The angle, in (0, 2π], the pointer asks for: the ray's hit on the plane
 * through `g` perpendicular to the axis, measured from `e1`. Null when the
 * ray runs parallel to that plane (hold the last angle).
 */
export function pointerAngle(frame: RevolveFrame, ray: Ray): number | null {
  const hit = rayPlaneIntersectBothWays(ray, frame.g, frame.axis)
  if (hit === null) return null
  const d = subV3(hit, frame.g)
  let a = Math.atan2(dotV3(d, frame.e2), dotV3(d, frame.e1))
  if (a <= 0) a += FULL_TURN
  return a
}

/** `rayPlaneIntersect` but also accepting a hit behind the ray origin: the
 *  angle plane can lie anywhere relative to the eye. */
function rayPlaneIntersectBothWays(ray: Ray, point: V3, normal: V3): V3 | null {
  const forward = rayPlaneIntersect(ray.origin as V3, ray.direction as V3, point, normal)
  if (forward !== null) return forward
  const denom = dotV3(ray.direction as V3, normal)
  if (Math.abs(denom) < 1e-10) return null
  const t = dotV3(subV3(point, ray.origin as V3), normal) / denom
  return [
    ray.origin[0] + t * ray.direction[0],
    ray.origin[1] + t * ray.direction[1],
    ray.origin[2] + t * ray.direction[2],
  ]
}

/** Snap to the nearest 15° step when within 3° of it. A reading within the
 *  window of the profile's own direction is the full turn, never a zero
 *  angle (which the kernel refuses). */
export function snapAngle(angle: number): number {
  const step = Math.round(angle / ANGLE_STEP) * ANGLE_STEP
  if (Math.abs(angle - step) > ANGLE_SNAP_TOL) return angle
  return step <= 0 ? FULL_TURN : step
}

/** Segments per full turn for a profile spun about the axis: the drawing
 *  tools' rule at the profile's largest radius, so the tool matches the API
 *  default. */
export function revolveSegments(ring: readonly V3[], axisPoint: V3, axisDir: V3): number {
  let maxRho = 0
  for (const p of ring) maxRho = Math.max(maxRho, distToAxis(p, axisPoint, axisDir))
  return segmentsPerTurn(maxRho)
}

/** The JSON the wasm `revolve*` methods take for the sweep. */
export function revolveSpecJson(axisPoint: V3, axis: V3, angle: number, segments: number): string {
  return JSON.stringify({ axis_point: axisPoint, axis, angle, segments })
}

/** The JSON profile for a sketch region or a solid face (ids as decimal strings). */
export function revolveProfileJson(
  profile: { kind: 'region'; sketch: bigint; region: bigint } | { kind: 'face'; object: bigint; face: bigint },
): string {
  return profile.kind === 'region'
    ? JSON.stringify({ region: { sketch: profile.sketch.toString(), region: profile.region.toString() } })
    : JSON.stringify({ face: { object: profile.object.toString(), face: profile.face.toString() } })
}
