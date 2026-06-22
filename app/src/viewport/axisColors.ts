/**
 * axisColors — shared world-axis color constants + a "does this direction lie
 * near a world axis" test, factored out of CueLayer (M10 on-axis cue) so the
 * Protractor tool can reuse the exact same colors/tolerance for its
 * axis-coloring + axis-snapping preview behavior.
 *
 * No three.js import — pure math, testable in Node/vitest.
 */

/** World axis colors: X=red, Y=green, Z=blue. Index = axis (0=X, 1=Y, 2=Z). */
export const AXIS_COLORS: [number, number, number] = [0xff2222, 0x22cc22, 0x2222ff]

const WORLD_AXIS: readonly [number, number, number][] = [
  [1, 0, 0],
  [0, 1, 0],
  [0, 0, 1],
]

export interface AxisColorMatch {
  color: number
  axis: 0 | 1 | 2
  /** The input direction snapped exactly onto the matched axis (unit length, same sign). */
  snapped: [number, number, number]
}

/**
 * Test whether a (unit) direction lies within `tolDot` (a cosine threshold,
 * e.g. `Math.cos(2 * Math.PI / 180)` for a 2° tolerance) of a world axis,
 * in either polarity (+X/-X both match axis 0, etc).
 *
 * Returns the matched axis's color, axis index, and the direction snapped
 * exactly onto that axis (preserving the input's sign along that axis, zero
 * elsewhere) — or null if no axis is within tolerance.
 *
 * `dir` need not be pre-normalized; it is normalized internally. A ~zero
 * vector (length < 1e-9) never matches.
 */
export function axisColorForDirection(
  dir: readonly [number, number, number],
  tolDot: number,
): AxisColorMatch | null {
  const [x, y, z] = dir
  const len = Math.sqrt(x * x + y * y + z * z)
  if (len < 1e-9) return null
  const ux = x / len, uy = y / len, uz = z / len

  for (let axis = 0 as 0 | 1 | 2; axis < 3; axis++) {
    const [ax, ay, az] = WORLD_AXIS[axis]
    const dot = ux * ax + uy * ay + uz * az
    if (Math.abs(dot) > tolDot) {
      const sign = dot >= 0 ? 1 : -1
      return {
        color: AXIS_COLORS[axis],
        axis,
        snapped: [ax * sign, ay * sign, az * sign],
      }
    }
  }
  return null
}
