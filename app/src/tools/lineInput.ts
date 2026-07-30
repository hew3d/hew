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
