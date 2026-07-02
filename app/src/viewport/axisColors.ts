/**
 * axisColors — shared world-axis color constants + a "does this direction lie
 * near a world axis" test, factored out of CueLayer (M10 on-axis cue) so the
 * Protractor tool can reuse the exact same colors/tolerance for its
 * axis-coloring + axis-snapping preview behavior.
 *
 * No three.js import — pure math, testable in Node/vitest.
 */

/**
 * World axis colors: X=red, Y=green, Z=blue. Index = axis (0=X, 1=Y, 2=Z).
 *
 * DARK_AXIS_COLORS now matches the Studio design spec's dark axis tokens
 * (`01_design_tokens.md`'s `--axis-red/#e85a60`, `--axis-green/#5fce80`,
 * `--axis-blue/#5f96eb` — same hex bytes, three.js just wants them as
 * numbers) —  retuned these from the pre-Studio bright pure-RGB
 * values `[0xff2222, 0x22cc22, 0x2222ff]`. AXIS_COLORS stays an alias of
 * DARK_AXIS_COLORS so call sites that haven't opted into theme-awareness
 * (via `axisColorsForTheme`) still get a sensible default.
 */
export const DARK_AXIS_COLORS: [number, number, number] = [0xe85a60, 0x5fce80, 0x5f96eb]

/** Light-theme axis colors, deepened for contrast per `01_design_tokens.md`. */
export const LIGHT_AXIS_COLORS: [number, number, number] = [0xd6454b, 0x28a055, 0x2d78e1]

/** @deprecated alias of {@link DARK_AXIS_COLORS} — kept so existing call sites are unaffected. */
export const AXIS_COLORS: [number, number, number] = DARK_AXIS_COLORS

/** Look up the axis-color triple for a resolved theme. */
export function axisColorsForTheme(theme: 'light' | 'dark'): [number, number, number] {
  return theme === 'light' ? LIGHT_AXIS_COLORS : DARK_AXIS_COLORS
}

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
 *
 * `colors` defaults to {@link AXIS_COLORS} (dark) for source compatibility;
 * callers that care about the light/dark distinction pass
 * `axisColorsForTheme(getResolvedTheme())` explicitly — see
 * `ProtractorTool.ts`/`SliceTool.ts`.
 */
export function axisColorForDirection(
  dir: readonly [number, number, number],
  tolDot: number,
  colors: readonly [number, number, number] = AXIS_COLORS,
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
        color: colors[axis],
        axis,
        snapped: [ax * sign, ay * sign, az * sign],
      }
    }
  }
  return null
}
