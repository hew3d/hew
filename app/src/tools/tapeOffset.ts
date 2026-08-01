/**
 * tapeOffset — pure geometry helpers for TapeMeasureTool's offset-from-edge
 * mode (tape-measure-rework design, WP-2): given a source edge and a
 * candidate direction (a drawing axis, or an arbitrary plane normal),
 * resolves the direction an offset is actually measured along, and reads
 * back the scalar distance a cursor has moved along it.
 *
 * No three.js or DOM imports — fully testable in Node/vitest. (The `V3`
 * type import below is type-only — erased at build time — so it doesn't
 * pull in any runtime dependency; likewise the `DrawingAxes` import. The
 * small vector-math this module needs is reimplemented locally rather than
 * importing geoHelpers' runtime exports, mirroring `moveInput.ts`'s
 * self-contained `pointAlong`.)
 */

import type { V3 } from '../viewport/geoHelpers'
import type { DrawingAxes } from './drawingAxes'

/**
 * "Along the edge" admission angle (degrees) for a candidate offset
 * direction — mirrors the kernel's `SOFT_AXIS_EDGE_ON_DEG`
 * (`crates/inference/src/lib.rs`, kept in sync at 3.0°): a candidate axis
 * within this many degrees of the source edge's direction, OR of its exact
 * reverse, can't usefully define a direction perpendicular to the edge —
 * the perpendicular component would be tiny and numerically unstable — so
 * it's rejected outright rather than silently collapsing toward zero
 * length.
 */
export const AXIS_ALONG_EDGE_DEG = 3.0

const AXIS_ALONG_EDGE_RAD = (AXIS_ALONG_EDGE_DEG * Math.PI) / 180

/** `sin(AXIS_ALONG_EDGE_DEG)` — the cross-product-magnitude threshold
 *  `offsetDirForAxis`/`viableOffsetAxes` compare against. Symmetric under
 *  negating either input, so it catches both the parallel and
 *  anti-parallel "along the edge" cases with a single comparison. */
export const AXIS_ALONG_EDGE_SIN = Math.sin(AXIS_ALONG_EDGE_RAD)

/** `cos(AXIS_ALONG_EDGE_DEG)` — provided alongside `AXIS_ALONG_EDGE_SIN`
 *  for callers/tests that prefer the dot-product form of the same test
 *  (as the kernel's `axis_is_edge_on` does). Not used internally here. */
export const AXIS_ALONG_EDGE_COS = Math.cos(AXIS_ALONG_EDGE_RAD)

function dot(a: V3, b: V3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

function cross(a: V3, b: V3): V3 {
  return [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ]
}

function sub(a: V3, b: V3): V3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

function length(v: V3): number {
  return Math.hypot(v[0], v[1], v[2])
}

/** Normalize `v`, or `null` if its length is below 1e-9 (degenerate — no
 *  direction to normalize to). */
function normalize(v: V3): V3 | null {
  const len = length(v)
  if (len < 1e-9) return null
  return [v[0] / len, v[1] / len, v[2] / len]
}

/**
 * Resolve a candidate drawing-axis `a` into the offset direction it defines
 * relative to a source edge's unit direction `d`: `a` projected into the
 * plane perpendicular to `d`, normalized.
 *
 * Returns `null` when `a` is "along the edge" and so not a viable offset
 * direction — `|d × a| < AXIS_ALONG_EDGE_SIN`, i.e. the angle between `a`
 * and `d` (or between `a` and `d`'s reverse) is under `AXIS_ALONG_EDGE_DEG`
 * — rather than returning a direction that's numerically near-meaningless.
 *
 * Both `d` and `a` are assumed to already be unit vectors.
 */
export function offsetDirForAxis(d: V3, a: V3): V3 | null {
  if (length(cross(d, a)) < AXIS_ALONG_EDGE_SIN) return null
  const k = dot(a, d)
  return normalize([a[0] - d[0] * k, a[1] - d[1] * k, a[2] - d[2] * k])
}

/**
 * Resolve a candidate plane normal `m` into the offset-plane normal
 * relative to a source edge's unit direction `d`: `m` projected into the
 * plane perpendicular to `d`, normalized.
 *
 * Returns `null` when the result is degenerate — `m` nearly parallel to
 * `d`, leaving a near-zero-length remainder after subtracting `d`'s
 * projection. When `m` is already exactly perpendicular to `d`, the
 * subtracted projection is zero and this returns `m` unchanged (up to
 * floating point) — callers rely on that identity.
 *
 * Both `d` and `m` are assumed to already be unit vectors.
 */
export function offsetPlaneNormal(d: V3, m: V3): V3 | null {
  const k = dot(m, d)
  return normalize([m[0] - d[0] * k, m[1] - d[1] * k, m[2] - d[2] * k])
}

/**
 * Which of a drawing-axes frame's three axes (red/green/blue) are viable
 * offset directions relative to a source edge's unit direction `d`, per the
 * same `AXIS_ALONG_EDGE_DEG` rule as `offsetDirForAxis`. Evaluated against
 * `frame`'s actual axis vectors, so a moved (non-world-identity) frame is
 * honored exactly like the world-identity one.
 */
export function viableOffsetAxes(d: V3, frame: DrawingAxes): [boolean, boolean, boolean] {
  return [
    offsetDirForAxis(d, frame.x) !== null,
    offsetDirForAxis(d, frame.y) !== null,
    offsetDirForAxis(d, frame.z) !== null,
  ]
}

/**
 * Project point `p` onto the plane through `origin` with unit normal `n`:
 * `p - ((p - origin)·n)*n`.
 */
export function projectPointOntoPlane(p: V3, origin: V3, n: V3): V3 {
  const k = dot(sub(p, origin), n)
  return [p[0] - n[0] * k, p[1] - n[1] * k, p[2] - n[2] * k]
}

/**
 * Signed scalar offset distance of `cursor` from `edgePoint`, measured along
 * unit direction `u`: `(cursor - edgePoint)·u`.
 */
export function signedOffsetAlong(cursor: V3, edgePoint: V3, u: V3): number {
  return dot(sub(cursor, edgePoint), u)
}

/**
 * The largest distance `stationOnAxisFromRay` will place its result from
 * `p0`, as a multiple of the distance from `rayOrigin` to `p0` — mirrors the
 * kernel's own `closest_point_on_line_to_ray` reach clamp
 * (`crates/inference/src/lib.rs`'s `MAX_AXIS_REACH_FACTOR`, kept in sync at
 * the same value 5.0): as `rayDir` approaches parallel to `axis`, the
 * unclamped station races toward infinity in a way that is NOT bounded by
 * scene scale on its own — a distance-scaled clamp protects every caller,
 * not just a differently-tuned angle threshold.
 */
const MAX_AXIS_REACH_FACTOR = 5.0

/**
 * Station (signed scalar distance from `p0`) on the axis line `p0 + t*axis`
 * closest to the ray `rayOrigin + s*rayDir` — a TS-side mirror of the
 * kernel's own `closest_point_on_line_to_ray` (`crates/inference/src/
 * lib.rs`), specialized to a line that passes through `p0` (so `p0` is
 * simultaneously the line's own origin and its `reach_ref`, collapsing that
 * function's re-origining step to a no-op).
 *
 * Used by `TapeMeasureTool`'s measure-stage axis lock (the tape-measure-
 * rework locked-axis fix) for the `'ground'`/`'plane'` snap-kind fallback:
 * when nothing real is under the cursor, the station has to come from the
 * cursor RAY itself, not from naively projecting the ground/plane hit
 * point's own coordinates onto the axis — that naive projection is only
 * correct when the ray happens to be perpendicular to the axis, and is
 * silently wrong (or, for a ray that never reaches the ground at all,
 * undefined) everywhere else.
 *
 * `axis` and `rayDir` must both already be unit vectors. If the ray is
 * (near-)parallel to `axis`, `denom` collapses toward zero and the
 * ordinary two-line derivation below is unusable there (it would blow up,
 * or divide by zero at exact parallelism); this mirrors the kernel's own
 * parallel-lines branch instead of returning `0` — the kernel's
 * `project_onto_line(origin, line_dir, ray_origin)` projects the RAY
 * ORIGIN (the camera eye, since these rays are cast from it) onto the
 * line, which stays smoothly stable as the ray direction jitters near
 * that degenerate alignment rather than discontinuously snapping to a
 * fixed point. Specialized to the scalar station this function returns
 * (rather than a point) and to a line through `p0`, that fallback is
 * simply `dot(rayOrigin - p0, axis)` — the station of the ray origin's
 * own foot on the axis. Otherwise (the non-degenerate case) the raw
 * station is clamped to `±MAX_AXIS_REACH_FACTOR` times the distance from
 * `rayOrigin` to `p0`, mirroring the kernel's own clamp.
 */
export function stationOnAxisFromRay(
  p0: V3,
  axis: V3,
  rayOrigin: V3,
  rayDir: V3,
): number {
  const b = dot(axis, rayDir)
  const denom = 1 - b * b
  if (Math.abs(denom) < 1e-12) return dot(sub(rayOrigin, p0), axis)
  const w = sub(p0, rayOrigin)
  const t = (b * dot(rayDir, w) - dot(axis, w)) / denom
  const bound = length(w) * MAX_AXIS_REACH_FACTOR
  return Math.min(Math.max(t, -bound), bound)
}
