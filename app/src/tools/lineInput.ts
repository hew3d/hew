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
