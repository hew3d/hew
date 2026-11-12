/**
 * viewTurn — the path a view-die turn takes (view-die design §2.3): the
 * camera orbits the FIXED orbit target at a FIXED distance while its eye
 * direction rotates from where it is to where the click sends it. Nothing
 * is lerped in a straight line, so a 180° flip (Front to Back, Top to
 * Bottom, a second click on the face already in view) never passes the
 * eye through the target, a 90° turn never pulls it in toward the target,
 * and the orbit pivot never drifts: a turn cancelled at any frame leaves
 * the camera on the same sphere about the same target.
 *
 * The rotation axis is chosen once per turn, deterministically: the axis
 * of the shortest arc between the two directions, and for an exactly
 * opposite pair (where every perpendicular axis is shortest) the world-up
 * component perpendicular to the start, so a horizontal flip swings round
 * blue, falling back to the screen-right axis (`up × start`) when the
 * start looks straight down or up, so Top and Bottom swap over the front.
 *
 * Pure math, no three.js: unit-tested on its own.
 */

export type V3 = [number, number, number]

const dot = (a: V3, b: V3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
const cross = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
const len = (a: V3): number => Math.hypot(a[0], a[1], a[2])
const scale = (a: V3, k: number): V3 => [a[0] * k, a[1] * k, a[2] * k]
const unit = (a: V3): V3 | null => {
  const l = len(a)
  return l > 1e-12 && Number.isFinite(l) ? scale(a, 1 / l) : null
}

const WORLD_UP: V3 = [0, 0, 1]

/** The rotation that takes unit direction `from` to unit direction `to`:
 *  a unit axis and an angle in [0, π]. */
export function viewTurnRotation(from: V3, to: V3): { axis: V3; angle: number } {
  const c = Math.min(1, Math.max(-1, dot(from, to)))
  const axis = unit(cross(from, to))
  // A clear shortest arc.
  if (axis !== null && len(cross(from, to)) > 1e-9) return { axis, angle: Math.acos(c) }
  if (c > 0) return { axis: WORLD_UP, angle: 0 } // already there
  // Exactly opposite: world up's part across the start, else screen right.
  const upAcross = unit([
    WORLD_UP[0] - from[0] * from[2],
    WORLD_UP[1] - from[1] * from[2],
    WORLD_UP[2] - from[2] * from[2],
  ])
  if (upAcross !== null && len(cross(WORLD_UP, from)) > 0.2) return { axis: upAcross, angle: Math.PI }
  const right = unit(cross(WORLD_UP, from)) ?? [1, 0, 0]
  return { axis: right, angle: Math.PI }
}

/** `v` rotated by `angle` about unit `axis` (Rodrigues). */
function rotate(v: V3, axis: V3, angle: number): V3 {
  const cs = Math.cos(angle)
  const sn = Math.sin(angle)
  const k = cross(axis, v)
  const d = dot(axis, v) * (1 - cs)
  return [
    v[0] * cs + k[0] * sn + axis[0] * d,
    v[1] * cs + k[1] * sn + axis[1] * d,
    v[2] * cs + k[2] * sn + axis[2] * d,
  ]
}

/** The eye direction (target to eye, unit) at progress `t` in [0, 1] of the
 *  turn `rotation` (from `viewTurnRotation(from, to)`) starting at `from`. */
export function viewTurnDirection(from: V3, rotation: { axis: V3; angle: number }, t: number): V3 {
  const d = rotate(from, rotation.axis, rotation.angle * t)
  return unit(d) ?? from
}

/** The eye position at progress `t`: on the sphere of radius `distance`
 *  about the fixed `target`. */
export function viewTurnEye(target: V3, distance: number, from: V3, rotation: { axis: V3; angle: number }, t: number): V3 {
  const d = viewTurnDirection(from, rotation, t)
  return [target[0] + d[0] * distance, target[1] + d[1] * distance, target[2] + d[2] * distance]
}
