/**
 * lineInput — pure helpers for LineTool's chained-segment gesture.
 *
 * No three.js or DOM imports — fully testable in Node/vitest. Mirrors the
 * "pure geometry extracted for testing" convention used by moveInput.ts and
 * viewport/geoHelpers.ts.
 */

/** 3-element number tuple for conciseness (matches geoHelpers' V3). */
export type V3 = [number, number, number]

/** Euclidean distance between two points. */
export function segmentLength(a: V3, b: V3): number {
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2])
}

/**
 * Normalized direction from `a` to `b`. Returns null if `a` and `b` are
 * coincident (distance below `epsilon`), since no direction is defined.
 */
export function directionBetween(a: V3, b: V3, epsilon = 1e-9): V3 | null {
  const dx = b[0] - a[0]
  const dy = b[1] - a[1]
  const dz = b[2] - a[2]
  const len = Math.hypot(dx, dy, dz)
  if (len < epsilon) return null
  return [dx / len, dy / len, dz / len]
}

/** Cross product `a` × `b`. */
export function crossV3(a: V3, b: V3): V3 {
  return [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ]
}

/** Dot product `a`·`b`. */
export function dotV3(a: V3, b: V3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/** Below this cross/reject magnitude, two (unit) directions are treated as
 *  parallel by `rehomePlaneNormal` — a UI heuristic tolerance, not a kernel
 *  geometry tolerance (contrast `kernel::tol::PLANE_DIST`, which governs
 *  whether the KERNEL accepts a point on a plane once one is chosen).
 *
 *  For two unit vectors, `|cross|` ≈ sin(θ) between them, so this doubles
 *  as an angular tolerance: 1e-2 ≈ 0.57° (small-angle approximation, θ ≈
 *  sin θ near 0). That headroom matters, not just the exact zero case —
 *  `prevDir`/`segDir` reaching this branch is EXACTLY parallel only when a
 *  chain literally continues straight ahead with no floating-point noise
 *  anywhere in the pipeline; in practice they arrive with a small residual
 *  angle from ordinary inference/projection imprecision. A too-tight
 *  epsilon (formerly 1e-6, θ ≈ 0.00006° — tighter than any real pointer
 *  input ever lands) can't tell that "genuinely meant to be parallel, plus
 *  noise" case apart from an actual small-angle turn, so it almost always
 *  takes the OTHER branch (spans `prevDir` × `segDir`) instead of the
 *  intended parallel fallback. That branch is numerically ill-conditioned
 *  precisely when the two directions are nearly parallel: its normalized
 *  result is the axis a near-zero perturbation of `segDir` rotates around,
 *  which is hypersensitive to exactly WHICH direction that perturbation
 *  happens to point — two inputs differing only by which axis an
 *  imperceptible (~0.006°) deviation landed on can flip the resulting plane
 *  by 90° (`lineInput.test.ts`'s "playtest-2 review finding C" case) even
 *  though a user would see both as "continuing straight ahead". 1e-2 gives
 *  enough margin over realistic pointer/inference noise to route that case
 *  to the stable view-facing fallback instead, while staying far below any
 *  angle a user could deliberately aim for as a turn. */
const REHOME_PARALLEL_EPS = 1e-2

/**
 * The unit normal of the plane a locked Line segment re-homes onto (tool-
 * parity playtest2 §2b) when its own direction `segDir` (already
 * normalized, anchor → the endpoint the lock resolved) leaves the CURRENT
 * frozen sketch plane. Pure — no wasm/three dependency, so the plane-choice
 * logic is unit-testable without a Scene fixture.
 *
 * - `prevDir` (the previous segment's direction, normalized — null at the
 *   first segment of a chain, which has no previous segment) spans a plane
 *   with `segDir` when the two are not parallel: `normal = normalize(prevDir
 *   × segDir)`. This keeps an L-shaped chain coplanar, so it can still close
 *   a region.
 * - Otherwise (no previous segment, or it's parallel to `segDir`): the
 *   view-facing plane containing `segDir` — `normal` = the component of
 *   `viewDir` perpendicular to `segDir` (`viewDir` rejected from `segDir`,
 *   normalized) — so the new plane faces the camera rather than receding
 *   edge-on into the screen.
 * - If even THAT degenerates (the camera is aimed almost exactly along the
 *   locked line, so its rejection from `segDir` is ~zero): an arbitrary
 *   plane containing `segDir`, picked the same way `facePlaneBasis` derives
 *   a reference axis (a component `segDir` isn't dominantly aligned with),
 *   so the result is always a well-defined unit vector. Neither of the
 *   design's own two rules covers this case; it exists only so a camera
 *   aimed straight down the segment doesn't produce a degenerate plane.
 */
export function rehomePlaneNormal(segDir: V3, prevDir: V3 | null, viewDir: V3): V3 {
  if (prevDir !== null) {
    const cross = crossV3(prevDir, segDir)
    const len = Math.hypot(cross[0], cross[1], cross[2])
    if (len > REHOME_PARALLEL_EPS) return [cross[0] / len, cross[1] / len, cross[2] / len]
  }

  const along = dotV3(viewDir, segDir)
  const rejected: V3 = [
    viewDir[0] - segDir[0] * along,
    viewDir[1] - segDir[1] * along,
    viewDir[2] - segDir[2] * along,
  ]
  const rlen = Math.hypot(rejected[0], rejected[1], rejected[2])
  if (rlen > REHOME_PARALLEL_EPS) return [rejected[0] / rlen, rejected[1] / rlen, rejected[2] / rlen]

  const refAxis: V3 = Math.abs(segDir[0]) > Math.abs(segDir[1]) ? [0, 1, 0] : [1, 0, 0]
  const arbitrary = crossV3(segDir, refAxis)
  const alen = Math.hypot(arbitrary[0], arbitrary[1], arbitrary[2])
  return [arbitrary[0] / alen, arbitrary[1] / alen, arbitrary[2] / alen]
}

/** Below this magnitude, a direction's projection onto the plane (or two
 *  projected directions' 2D cross product) is treated as degenerate by
 *  `fromPointCandidate` — either "this axis has no usable in-plane
 *  component" (near-perpendicular to the plane) or "these two projected
 *  directions are parallel" (no unique intersection). A plain numerical
 *  guard, not a kernel geometry tolerance. */
const FROM_POINT_DEGENERATE_EPS = 1e-9

/** Largest component along the plane normal a `fromPointCandidate` input may
 *  have and still count as lying in the plane: for points, metres off the
 *  plane (the kernel's own plane tolerance scale, `GROUND_PLANE_EPS`); for
 *  unit directions, the sine of the angle out of the plane. Anything beyond
 *  it is genuinely 3-D, and a 2-D intersection of its shadow would name a
 *  point the real lines never meet at. */
const FROM_POINT_PLANE_EPS = 1e-9

function sub(a: V3, b: V3): V3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/**
 * The from-point closing-inference candidate (LineTool's module doc —
 * SketchUp's classic "draw three sides of a square, the fourth snaps
 * shut"): where the CURRENT segment, from `segStart`, meets an infinite
 * line through an earlier chain vertex `p` drawn along `axisDir`.
 *
 * Everything is computed in the plane's own 2D (u, v) coordinates, so the
 * inputs must really lie in it: `p` must be on the plane (a chain vertex
 * committed before the chain re-homed onto this plane need not be), and
 * `axisDir` and `lockDir` must run in the plane (a world axis oblique to a
 * tilted plane, or a lock that leaves the plane, does not). Any input with
 * a component along the plane normal beyond `FROM_POINT_PLANE_EPS` yields no
 * candidate — projecting it would intersect shadows of lines that never
 * meet in 3-D. An `axisDir` with (nearly) no component IN the plane (the
 * blue axis on a ground-plane chain) is rejected the same way.
 *
 * - `lockDir` non-null (the current segment is axis-locked along it): the
 *   candidate is the intersection of the ray `segStart + s * lockDir` (s >
 *   0 — a segment can only close FORWARD, never behind where it started)
 *   with the line `p + t * axisDir`. Null if the two projected directions
 *   are parallel, or the solved `s` isn't positive.
 * - `lockDir` null (unlocked): the candidate is the foot of the
 *   perpendicular from `cursor` onto the line `p + t * axisDir`.
 *
 * Returns null whenever no well-defined candidate exists (see above) —
 * never throws.
 */
export function fromPointCandidate(
  p: V3,
  axisDir: V3,
  segStart: V3,
  cursor: V3,
  lockDir: V3 | null,
  planeOrigin: V3,
  planeU: V3,
  planeV: V3,
  epsilon = FROM_POINT_DEGENERATE_EPS,
): V3 | null {
  const toUv = (pt: V3): [number, number] => {
    const dx = pt[0] - planeOrigin[0]
    const dy = pt[1] - planeOrigin[1]
    const dz = pt[2] - planeOrigin[2]
    return [dx * planeU[0] + dy * planeU[1] + dz * planeU[2], dx * planeV[0] + dy * planeV[1] + dz * planeV[2]]
  }
  const dirUv = (dir: V3): [number, number] => [dotV3(dir, planeU), dotV3(dir, planeV)]

  // In-plane guard: the plane normal is u × v for an orthonormal basis.
  const n: V3 = crossV3(planeU, planeV)
  const offPlane = (pt: V3): boolean =>
    Math.abs((pt[0] - planeOrigin[0]) * n[0] + (pt[1] - planeOrigin[1]) * n[1] + (pt[2] - planeOrigin[2]) * n[2]) >
    FROM_POINT_PLANE_EPS
  if (offPlane(p)) return null
  if (Math.abs(dotV3(axisDir, n)) > FROM_POINT_PLANE_EPS) return null
  if (lockDir !== null && Math.abs(dotV3(lockDir, n)) > FROM_POINT_PLANE_EPS) return null

  const pUv = toUv(p)
  const dUv = dirUv(axisDir)
  if (Math.hypot(dUv[0], dUv[1]) < epsilon) return null

  let resultUv: [number, number]
  if (lockDir !== null) {
    const sUv = toUv(segStart)
    const uUv = dirUv(lockDir)
    if (Math.hypot(uUv[0], uUv[1]) < epsilon) return null
    // Solve sUv + s*uUv = pUv + t*dUv for s (2×2 linear system; standard
    // line-line intersection in 2D — see the doc comment above).
    const denom = uUv[0] * dUv[1] - uUv[1] * dUv[0]
    if (Math.abs(denom) < epsilon) return null // parallel
    const rx = pUv[0] - sUv[0]
    const ry = pUv[1] - sUv[1]
    const s = (rx * dUv[1] - ry * dUv[0]) / denom
    if (s <= epsilon) return null // must close FORWARD from segStart
    resultUv = [sUv[0] + s * uUv[0], sUv[1] + s * uUv[1]]
  } else {
    // The foot of the perpendicular, built as `p + t * axisDir` in 3-D (the
    // guards above put `p` and `axisDir` in the plane, so this is the same
    // point the 2-D solve names): a coordinate the line holds constant
    // comes back as `p`'s own bits, which a snap that must equal an
    // existing corner exactly depends on.
    const t = dotV3(sub(cursor, p), axisDir) / dotV3(axisDir, axisDir)
    return [p[0] + t * axisDir[0], p[1] + t * axisDir[1], p[2] + t * axisDir[2]]
  }

  return [
    planeOrigin[0] + resultUv[0] * planeU[0] + resultUv[1] * planeV[0],
    planeOrigin[1] + resultUv[0] * planeU[1] + resultUv[1] * planeV[1],
    planeOrigin[2] + resultUv[0] * planeU[2] + resultUv[1] * planeV[2],
  ]
}
