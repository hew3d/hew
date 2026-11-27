/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * curveInput — the typed-value grammar shared by the Fillet tool and the
 * other curve tools (Revolve, Deform, Loft, Array along a path). Array along
 * Path reads its buffer through `parseArrayInput`, where a bare whole number
 * is a count.
 *
 * One buffer, one parser: `parseCurveInput` turns what the user typed into a
 * tagged token and the tool decides what the token means in its stage. A bare
 * number is a length in the current display unit, as everywhere else.
 *
 *   "6" "6mm" "1/4\"" "2-1/4\""   a length
 *   "6,3"                         a pair of lengths (Chamfer: two distances)
 *   "3,45°" "3,45deg"             a length and an angle (Chamfer: distance, angle)
 *   "30°" "30deg"                 an angle
 *   "60%"                         a percentage, as a ratio (0.6)
 *   "100%→40%" "100%->40%"        a ramp from one percentage to another
 *   "8s"                          a segment count
 *   "5x" "x5"                     a copy count
 *
 * No three.js or DOM imports, so it is fully testable in Node.
 */

import { parseLengthToMeters, type LengthFormat } from '../settings/units'

export type CurveToken =
  | { kind: 'length'; meters: number }
  | { kind: 'pair'; a: number; b: number }
  | { kind: 'lengthAngle'; meters: number; radians: number }
  | { kind: 'angle'; radians: number }
  | { kind: 'percent'; ratio: number }
  | { kind: 'ramp'; from: number; to: number }
  | { kind: 'segments'; n: number }
  | { kind: 'count'; n: number }

export type CurveParseResult = CurveToken | { error: string }

const NUMBER = String.raw`-?(?:\d+\.?\d*|\.\d+)`
const ANGLE_RE = new RegExp(`^(${NUMBER})\\s*(?:°|deg)$`, 'i')
const LENGTH_ANGLE_RE = new RegExp(`^(.+?)\\s*,\\s*(${NUMBER})\\s*(?:°|deg)$`, 'i')
const PERCENT_RE = new RegExp(`^(${NUMBER})\\s*%$`)
const RAMP_RE = new RegExp(`^(${NUMBER})\\s*%\\s*(?:→|->|>)\\s*(${NUMBER})\\s*%$`)
const SEGMENTS_RE = /^(\d+)\s*s$/i
const COUNT_RE = /^(?:(\d+)\s*[x*]|[x*]\s*(\d+))$/i

/** Keys the grammar accepts in its buffer, beyond what a plain length takes:
 *  the comma of a pair, the degree sign, the percent sign, the ramp arrow,
 *  and the letters of `s`, `x`, `deg` and the unit suffixes. */
const CURVE_KEY_RE = /^[0-9.,\-/'"°%sx*>→mcinftdeg]$/i

/** True when `key` can extend a curve-input buffer. */
export function isCurveInputKey(key: string): boolean {
  return key === 'Backspace' || CURVE_KEY_RE.test(key)
}

/** Appends or removes one key. The grammar is permissive while typing (a
 *  half-typed `3,4` is fine); `parseCurveInput` judges the whole buffer. */
export function editCurveBuffer(buf: string, key: string): string {
  if (key === 'Backspace') return buf.slice(0, -1)
  if (key === 'X' || key === '*') return buf + 'x'
  if (!isCurveInputKey(key)) return buf
  return buf + key
}

/** Parses the buffer. `format` is the display unit a bare number is in. */
export function parseCurveInput(text: string, format: LengthFormat): CurveParseResult {
  const t = text.trim()
  if (t === '') return { error: 'Type a value.' }

  const ramp = RAMP_RE.exec(t)
  if (ramp !== null) {
    const from = parseFloat(ramp[1]) / 100
    const to = parseFloat(ramp[2]) / 100
    if (!(from > 0) || !(to > 0)) return { error: 'Both scales must be above 0%.' }
    return { kind: 'ramp', from, to }
  }
  const percent = PERCENT_RE.exec(t)
  if (percent !== null) return { kind: 'percent', ratio: parseFloat(percent[1]) / 100 }

  const segments = SEGMENTS_RE.exec(t)
  if (segments !== null) {
    const n = parseInt(segments[1], 10)
    return n >= 1 ? { kind: 'segments', n } : { error: 'Use at least 1 segment.' }
  }
  const count = COUNT_RE.exec(t)
  if (count !== null) {
    const n = parseInt(count[1] ?? count[2], 10)
    return n >= 1 ? { kind: 'count', n } : { error: 'Use at least 1 copy.' }
  }

  const angle = ANGLE_RE.exec(t)
  if (angle !== null) return { kind: 'angle', radians: (parseFloat(angle[1]) * Math.PI) / 180 }

  const lengthAngle = LENGTH_ANGLE_RE.exec(t)
  if (lengthAngle !== null) {
    const meters = parseLengthToMeters(lengthAngle[1], format)
    if (meters === null) return { error: 'That length is not valid.' }
    return { kind: 'lengthAngle', meters, radians: (parseFloat(lengthAngle[2]) * Math.PI) / 180 }
  }

  if (t.includes(',')) {
    const parts = t.split(',')
    if (parts.length !== 2) return { error: 'Type two lengths, like 6,3.' }
    const a = parseLengthToMeters(parts[0], format)
    const b = parseLengthToMeters(parts[1], format)
    if (a === null || b === null) return { error: 'Type two lengths, like 6,3.' }
    return { kind: 'pair', a, b }
  }

  const meters = parseLengthToMeters(t, format)
  if (meters === null) return { error: 'That length is not valid.' }
  return { kind: 'length', meters }
}

export type ArrayToken = Extract<CurveToken, { kind: 'count' | 'ramp' }>

/**
 * Array along Path's reading of a buffer: only a count or a scale ramp mean
 * anything there, and a bare whole number is a count (`12` means `12x`).
 * Any other token (a length, a pair, an angle) is refused in plain words.
 */
export function parseArrayInput(text: string, format: LengthFormat): ArrayToken | { error: string } {
  const t = text.trim()
  const r = /^\d+$/.test(t) ? parseCurveInput(`${t}x`, format) : parseCurveInput(text, format)
  if ('error' in r || r.kind === 'count' || r.kind === 'ramp') return r
  return { error: 'Type a count like 12 or 12x, or a scale like 100%→40%.' }
}
