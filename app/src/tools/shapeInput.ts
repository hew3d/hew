/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * shapeInput — the typed-value grammar of the shape tools (Revolve, Deform,
 * and Follow Me's twist options).
 *
 * One measurements box, three kinds of value besides a plain length:
 *   - an angle: `90`, `90°`, `90d`, `90deg` (a bare number is degrees
 *     wherever a tool asks for an angle),
 *   - a percentage: `60%` (a ratio of 0.6),
 *   - a count: `24s` (segments or stations).
 *
 * The suffixes are terminal: once typed, only Backspace edits the buffer, so
 * "30°5" can never build up. Angles are converted to radians and percentages
 * to ratios here, at the boundary, per 01-conventions §10.
 *
 * No three.js or DOM imports; testable in plain Node.
 */

import { editLengthBuffer, isLengthInputKey } from './moveInput'
import type { LengthFormat } from '../settings/units'

/** A parsed suffix token. `null` from `parseShapeToken` means the buffer is a
 *  bare number (or empty/incomplete), which each tool reads in its own way. */
export type ShapeToken =
  | { kind: 'angle'; radians: number; degrees: number }
  | { kind: 'percent'; ratio: number }
  | { kind: 'count'; n: number }

const NUM = '-?(?:\\d+\\.?\\d*|\\.\\d+)'
const ANGLE_RE = new RegExp(`^(${NUM})(?:°|deg|d)$`)
const PERCENT_RE = new RegExp(`^(${NUM})%$`)
const COUNT_RE = /^(\d+)s$/
const BARE_RE = new RegExp(`^${NUM}$`)

/** The suffix keys this grammar adds on top of the length grammar. */
const SHAPE_SUFFIX_KEY_RE = /^[%°degs]$/

/** True for a keystroke the shape buffer accepts: everything the length
 *  grammar takes plus the angle, percent and count suffix letters. */
export function isShapeInputKey(key: string): boolean {
  return isLengthInputKey(key) || SHAPE_SUFFIX_KEY_RE.test(key)
}

/** Whether `buf` already ends in a terminal suffix (nothing may follow). */
function isFrozen(buf: string): boolean {
  return /(?:°|deg|%|s)$/.test(buf)
}

/**
 * Immutably edit a shape buffer. Suffix keys are accepted only after a number
 * has started (so a bare `s` or `d` stays a tool-switch shortcut), `deg` is
 * spelled out one letter at a time, and a finished suffix freezes the buffer.
 */
export function editShapeBuffer(buf: string, key: string, format: LengthFormat): string {
  if (key === 'Backspace') return buf.slice(0, -1)
  if (isFrozen(buf)) return buf
  if (SHAPE_SUFFIX_KEY_RE.test(key)) {
    const lower = key.toLowerCase()
    const endsDigit = /[0-9.]$/.test(buf)
    if (key === '°' || key === '%' || lower === 's') return endsDigit ? buf + (lower === 's' ? 's' : key) : buf
    if (lower === 'd') return endsDigit ? buf + 'd' : buf
    if (lower === 'e') return /d$/.test(buf) ? buf + 'e' : buf
    if (lower === 'g') return /de$/.test(buf) ? buf + 'g' : buf
    return buf
  }
  // A partial `d`/`de` blocks everything except the next letter of `deg`.
  if (/d(?:e)?$/.test(buf)) return buf
  return editLengthBuffer(buf, key, format)
}

/** Parse a suffix token, or null when the buffer carries none. */
export function parseShapeToken(buf: string): ShapeToken | null {
  const a = ANGLE_RE.exec(buf)
  if (a !== null) {
    const degrees = parseFloat(a[1])
    return Number.isFinite(degrees) ? { kind: 'angle', radians: (degrees * Math.PI) / 180, degrees } : null
  }
  const p = PERCENT_RE.exec(buf)
  if (p !== null) {
    const pct = parseFloat(p[1])
    return Number.isFinite(pct) && pct > 0 ? { kind: 'percent', ratio: pct / 100 } : null
  }
  const c = COUNT_RE.exec(buf)
  if (c !== null) {
    const n = parseInt(c[1], 10)
    return n >= 1 ? { kind: 'count', n } : null
  }
  return null
}

/** A modifier pressed on its own (the Viewport forwards those keydowns
 *  too): not a keystroke a tool's confirmation note should vanish on. */
export function isBareModifier(key: string): boolean {
  return key === 'Alt' || key === 'Shift' || key === 'Control' || key === 'Meta'
}

/** A percentage of zero or less (`0%`): no token, but not a plain number
 *  either, so a tool can say why it was not taken. */
export function isZeroPercent(buf: string): boolean {
  const p = PERCENT_RE.exec(buf)
  return p !== null && !(parseFloat(p[1]) > 0)
}

/** An angle in the buffer: `90°`, `90d`, `90deg`, or a bare number read as degrees. */
export function parseAngleBuffer(buf: string): { radians: number; degrees: number } | null {
  const t = parseShapeToken(buf)
  if (t !== null) return t.kind === 'angle' ? { radians: t.radians, degrees: t.degrees } : null
  if (!BARE_RE.test(buf)) return null
  const degrees = parseFloat(buf)
  return Number.isFinite(degrees) ? { radians: (degrees * Math.PI) / 180, degrees } : null
}

/** A ratio in the buffer: `60%`, or a bare number read as percent. */
export function parseRatioBuffer(buf: string): number | null {
  const t = parseShapeToken(buf)
  if (t !== null) return t.kind === 'percent' ? t.ratio : null
  if (!BARE_RE.test(buf)) return null
  const pct = parseFloat(buf)
  return Number.isFinite(pct) && pct > 0 ? pct / 100 : null
}

/** Readout for the measurements box: a bare number gets the unit its tool
 *  reads it as (`°` or `%`); a buffer with its own suffix shows as typed. */
export function shapeReadout(buf: string, bare: 'angle' | 'percent'): string {
  if (buf === '' || !BARE_RE.test(buf)) return buf
  return bare === 'angle' ? `${buf}°` : `${buf}%`
}

/** Degrees for display: whole numbers plain, otherwise one decimal. */
export function formatDegrees(radians: number): string {
  const deg = (radians * 180) / Math.PI
  const r = Math.round(deg * 10) / 10
  return Number.isInteger(r) ? `${r}°` : `${r.toFixed(1)}°`
}

/** Percent for display, same rounding rule. */
export function formatPercent(ratio: number): string {
  const r = Math.round(ratio * 1000) / 10
  return Number.isInteger(r) ? `${r}%` : `${r.toFixed(1)}%`
}
