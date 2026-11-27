/**
 * filletCorner2d — the plane geometry of rounding or chamfering a 2D corner
 * (a corner of a drawn shape): the tangent points, the arc, and the size the
 * pointer asks for. The kernel owns the committed result; this is only the
 * preview and the sizing, so it mirrors the kernel's own corner rules
 * (`Sketch::blend_corner`).
 *
 * A corner is described by its vertex `c` and the far end of each of its two
 * straight edges, `p` and `q`.
 */

import type { V3 } from './filletMath'
import { arcSegments } from './arcMath'

export interface Corner2d {
  c: V3
  p: V3
  q: V3
}

function sub(a: V3, b: V3): V3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
function add(a: V3, b: V3): V3 {
  return [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
function scale(a: V3, k: number): V3 {
  return [a[0] * k, a[1] * k, a[2] * k]
}
function dot(a: V3, b: V3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
function cross(a: V3, b: V3): V3 {
  return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
function len(a: V3): number {
  return Math.hypot(a[0], a[1], a[2])
}
function unit(a: V3): V3 | null {
  const l = len(a)
  return l > 0 ? scale(a, 1 / l) : null
}

/** The two unit edge directions leaving the corner, or null if degenerate. */
export function cornerDirections(g: Corner2d): { dp: V3; dq: V3 } | null {
  const dp = unit(sub(g.p, g.c))
  const dq = unit(sub(g.q, g.c))
  return dp === null || dq === null ? null : { dp, dq }
}

/** The interior angle at the corner, in (0, π); null when degenerate or
 *  straight (the kernel refuses a straight corner). */
export function cornerAngle(g: Corner2d): number | null {
  const d = cornerDirections(g)
  if (d === null) return null
  const cos = Math.max(-1, Math.min(1, dot(d.dp, d.dq)))
  const theta = Math.acos(cos)
  return theta > 1e-6 && theta < Math.PI - 1e-6 ? theta : null
}

/** The largest tangent length that stays on both edges. */
export function cornerMaxSetback(g: Corner2d): number {
  return Math.min(len(sub(g.p, g.c)), len(sub(g.q, g.c)))
}

/**
 * The tangent length the pointer asks for: the distance from the corner to
 * the pointer's position projected on the nearer edge (the edge the pointer
 * leans toward), per the design. `at` is the pointer on the corner's plane.
 */
export function cornerSetbackFromPointer(g: Corner2d, at: V3): number {
  const d = cornerDirections(g)
  if (d === null) return 0
  const v = sub(at, g.c)
  return Math.max(0, dot(v, d.dp), dot(v, d.dq))
}

/** Round radius for tangent length `s`: `r = s · tan(θ/2)`. */
export function cornerRadiusFromSetback(s: number, theta: number): number {
  return s * Math.tan(theta / 2)
}

/** Tangent length for round radius `r`: `s = r / tan(θ/2)`. */
export function cornerSetbackFromRadius(r: number, theta: number): number {
  return r / Math.tan(theta / 2)
}

/**
 * The polyline of a rounded corner from the tangent point on edge p, along
 * the arc, to the tangent point on edge q. Null when the radius does not fit
 * the shorter edge or the corner is degenerate.
 */
export function roundedCornerPolyline(g: Corner2d, radius: number, segments: number): V3[] | null {
  const dirs = cornerDirections(g)
  const theta = cornerAngle(g)
  if (dirs === null || theta === null || !(radius > 0)) return null
  const s = cornerSetbackFromRadius(radius, theta)
  if (s > cornerMaxSetback(g)) return null
  const t1 = add(g.c, scale(dirs.dp, s))
  const t2 = add(g.c, scale(dirs.dq, s))
  const bis = unit(add(dirs.dp, dirs.dq))
  if (bis === null) return null
  const center = add(g.c, scale(bis, radius / Math.sin(theta / 2)))
  const a = sub(t1, center)
  const b = sub(t2, center)
  const n = Math.max(1, Math.round(segments))
  // Sweep the short way from a to b about the plane normal.
  const normal = unit(cross(a, b))
  if (normal === null) return null
  const sweep = Math.acos(Math.max(-1, Math.min(1, dot(a, b) / (len(a) * len(b)))))
  const out: V3[] = [t1]
  for (let i = 1; i < n; i++) {
    const ang = (sweep * i) / n
    // Rodrigues' rotation of a about `normal` by `ang`.
    const ca = Math.cos(ang)
    const sa = Math.sin(ang)
    const rot = add(add(scale(a, ca), scale(cross(normal, a), sa)), scale(normal, dot(normal, a) * (1 - ca)))
    out.push(add(center, rot))
  }
  out.push(t2)
  return out
}

/** The default facet count of a rounded corner: the drawing tools' count for
 *  the turn `π − θ` at this radius (the kernel's `arc_segments`). */
export function cornerDefaultSegments(radius: number, theta: number): number {
  return arcSegments(radius, Math.PI - theta)
}

/**
 * The segment of a chamfered corner: the tangent points `setback_a` along the
 * edge that leads the counter-clockwise order seen from `normal` and
 * `setback_b` along the other (the kernel's convention, so a pair typed as
 * `a,b` lands where the kernel puts it). Null when a setback does not fit.
 */
export function chamferedCornerPolyline(g: Corner2d, setbackA: number, setbackB: number, normal: V3): V3[] | null {
  const dirs = cornerDirections(g)
  if (dirs === null || cornerAngle(g) === null) return null
  // `a` is the direction from which `b` is reached clockwise: for a unit
  // square corner at the origin, a runs along +y and b along +x.
  const pToQCcw = dot(cross(dirs.dp, dirs.dq), normal) > 0
  const [da, db, la, lb] = pToQCcw
    ? [dirs.dq, dirs.dp, len(sub(g.q, g.c)), len(sub(g.p, g.c))]
    : [dirs.dp, dirs.dq, len(sub(g.p, g.c)), len(sub(g.q, g.c))]
  if (!(setbackA > 0) || !(setbackB > 0) || setbackA > la || setbackB > lb) return null
  return [add(g.c, scale(da, setbackA)), add(g.c, scale(db, setbackB))]
}
