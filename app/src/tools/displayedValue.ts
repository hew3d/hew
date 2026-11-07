/**
 * "A clicked value is exactly the value the measurement box shows."
 *
 * While the cursor moves, the measurement box (VCB) shows a ROUNDED value
 * ("9.4 mm", "37.5°", "×1.25"). The raw cursor position behind it carries
 * more digits. When the user clicks, the value they saw must be the value
 * they get — exactly as if they had typed it. Tools therefore quantize a
 * click through the SAME text the box shows, then commit through the
 * tool's typed path.
 *
 * Which clicks are quantized (the free/hard rule):
 *
 *  - FREE picks quantize: `plane`, `ground`, `on-face`, `parallel`,
 *    `perpendicular`, and `on-axis` WITHOUT `projectedFrom` whose axis line
 *    runs through the tool's anchor (a bare axis lock or soft axis only
 *    fixes a direction from the anchor; the length along it is still the
 *    user's free choice). An on-axis pick on the drawing axes themselves,
 *    away from the anchor, is the axis line the user aimed at: rounding the
 *    length from the anchor would land a hair off it, so it is hard.
 *  - HARD picks keep the snapped point exactly: `endpoint`, `center`,
 *    `quadrant`, `midpoint`, `intersection`, `tangent`, `on-edge`,
 *    `on-intersection-line`, `on-guide`, `on-axis` WITH `projectedFrom` (the axis line was
 *    projected from a real candidate), LineTool's synthetic `from-point`
 *    alignment, and any kind not listed above. Rounding there would land
 *    a hair off the vertex or edge the user aimed at.
 *
 * Consistent with the file-local sets in PushPullTool / OffsetTool
 * (`HARD_SNAP_KINDS`), PositionTextureTool (`POINT_SNAP_KINDS`) and
 * LineTool (`FROM_POINT_WEAK_SNAP_KINDS`).
 */

import type { Snap } from './types'
import { formatLength, parseLengthToMeters } from '../settings/units'

const FREE_KINDS: ReadonlySet<string> = new Set([
  'plane',
  'ground',
  'on-face',
  'parallel',
  'perpendicular',
])

/** How far (m) the tool's anchor may sit from an on-axis pick's line and
 *  still be on it: float noise only, since a soft axis is built from the
 *  anchor itself. */
const ON_AXIS_ANCHOR_EPS = 1e-9

/** True when the pick carries no geometric commitment beyond the cursor
 *  position, so its displayed value may stand in for the raw one. `anchor`
 *  is the point the tool measures the clicked value from (a line's start,
 *  a circle's center); with it, an on-axis pick is free only when its axis
 *  line runs through the anchor. */
export function isFreePick(
  snap: Snap | null | undefined,
  anchor?: readonly [number, number, number],
): boolean {
  if (snap === null || snap === undefined) return false
  if (FREE_KINDS.has(snap.kind)) return true
  if (snap.kind !== 'on-axis' || snap.projectedFrom !== undefined) return false
  const d = snap.direction
  if (anchor === undefined || d === undefined) return true
  const len = Math.hypot(d[0], d[1], d[2])
  if (!(len > 0)) return true
  const w: [number, number, number] = [anchor[0] - snap.x, anchor[1] - snap.y, anchor[2] - snap.z]
  const t = (w[0] * d[0] + w[1] * d[1] + w[2] * d[2]) / (len * len)
  const off = Math.hypot(w[0] - t * d[0], w[1] - t * d[1], w[2] - t * d[2])
  return off <= ON_AXIS_ANCHOR_EPS
}

/** Normalise -0 to 0. */
function noNegZero(n: number): number {
  return n === 0 ? 0 : n
}

/**
 * The length the measurement box would show for `meters`, re-read as if
 * typed: `parseLengthToMeters(formatLength(|meters|))` in the current
 * display format, with the sign of `meters` kept. Bit-identical to typing
 * the shown text. Falls back to `meters` itself if the text cannot be
 * parsed (never expected).
 */
export function displayedLength(meters: number): number {
  if (!Number.isFinite(meters)) return meters
  const mag = parseLengthToMeters(formatLength(Math.abs(meters)))
  if (mag === null || !Number.isFinite(mag)) return meters
  return noNegZero(meters < 0 ? -mag : mag)
}

/** The angle (degrees) the box shows as `toFixed(1)°`, re-read as typed. */
export function displayedAngleDeg(deg: number): number {
  if (!Number.isFinite(deg)) return deg
  return noNegZero(parseFloat(deg.toFixed(1)))
}

/** The scale factor the box shows as `toFixed(2)`, re-read as typed. */
export function displayedFactor(f: number): number {
  if (!Number.isFinite(f)) return f
  return noNegZero(parseFloat(f.toFixed(2)))
}

/**
 * Bulge-stage arc conversion. The arc tool SHOWS the radius but its click
 * and typed entry both use the SAGITTA. Given the chord length `chord`, a
 * (raw) sagitta `sag` (>= 0) and whether the arc is major (sagitta > chord/2),
 * returns the sagitta whose radius reads as the displayed radius. Returns
 * null when the displayed radius cannot span the chord (R < c/2): the
 * caller falls back to the raw click.
 */
export function sagittaForDisplayedRadius(
  chord: number,
  sag: number,
  major: boolean,
): number | null {
  if (!(chord > 0) || !(sag > 0)) return null
  const rawR = (chord * chord) / (8 * sag) + sag / 2
  const r = displayedLength(rawR)
  const half = chord / 2
  if (!(r >= half)) return null
  const root = Math.sqrt(Math.max(0, r * r - half * half))
  const s = major ? r + root : r - root
  return s > 0 && Number.isFinite(s) ? s : null
}
