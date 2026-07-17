/**
 * moveInput — pure helpers for axis-lock and numeric VCB entry in MoveTool.
 *
 * No three.js or DOM imports — fully testable in Node/vitest. (The
 * `LengthFormat` type import below is type-only — erased at build time —
 * so it doesn't pull in any runtime dependency.)
 */

import type { LengthFormat } from '../settings/units'

/**
 * Map an arrow key to an axis index.
 *
 * ArrowRight → 0 (X), ArrowLeft → 1 (Y), ArrowUp → 2 (Z).
 * ArrowDown and all other keys → null (the caller treats ArrowDown as "clear").
 */
export function arrowToAxis(key: string): 0 | 1 | 2 | null {
  if (key === 'ArrowRight') return 0
  if (key === 'ArrowLeft') return 1
  if (key === 'ArrowUp') return 2
  return null
}

/**
 * Immutably edit a numeric string buffer.
 *
 * Rules:
 * - Digits 0–9 and `.` are appended.
 * - A second `.` is rejected (buffer returned unchanged).
 * - `-` toggles/replaces a leading minus sign:
 *     - If buffer starts with `-`, strip it.
 *     - Otherwise, prepend `-`.
 * - `Backspace` removes the last character.
 * - Any other key is ignored.
 *
 * The buffer is always a raw string that may be empty, or contain a
 * leading `-`, trailing `.`, etc.  parseDistance() handles the coercion.
 */
export function editNumericBuffer(buf: string, key: string): string {
  if (key === 'Backspace') {
    return buf.slice(0, -1)
  }

  if (key === '-') {
    return buf.startsWith('-') ? buf.slice(1) : '-' + buf
  }

  if (key === '.') {
    if (buf.includes('.')) return buf  // reject second dot
    return buf + '.'
  }

  if (key >= '0' && key <= '9') {
    return buf + key
  }

  return buf
}

/**
 * Letters that can appear in an explicit unit suffix ("mm", "cm", "km", "m",
 * "ft", "in") — accepted by the length buffers so an explicit unit can be
 * typed in ANY display mode. Deliberately excludes `x`/`X` (the dimensions
 * separator).
 */
const UNIT_SUFFIX_LETTER_RE = /^[mckftinMCKFTIN]$/

/**
 * True if `key` is a keystroke the single-length VCB accepts (digits, dot,
 * minus, Backspace, the feet/inch/fraction marks `'` `"` `/`, space, and
 * explicit unit-suffix letters). Tools use this to decide whether to route
 * a key into `editLengthBuffer`.
 */
export function isLengthInputKey(key: string): boolean {
  return key === 'Backspace' || /^[0-9.\-'"/ ]$/.test(key) || UNIT_SUFFIX_LETTER_RE.test(key)
}

/**
 * Immutably edit a length VCB string buffer.
 *
 * `editNumericBuffer` PLUS acceptance of the explicit-unit grammar tokens,
 * in EVERY display format (the display format only governs how a bare
 * number is interpreted at parse time — an explicit unit is always
 * typeable, see `parseLengthToMeters`):
 *   - `'` (feet), `"` (inches), `/` (fraction), space (separator between
 *     inches and a fraction, e.g. "5' 3 1/2\"", or before a unit suffix,
 *     e.g. "1 cm");
 *   - unit-suffix letters for "mm"/"cm"/"km"/"m"/"ft"/"in".
 * The `_format` parameter is kept for call-site compatibility; the accepted
 * token set no longer depends on it.
 *
 * No three.js or DOM imports — fully testable in Node/vitest.
 */
export function editLengthBuffer(buf: string, key: string, _format: LengthFormat): string {
  if (key === 'Backspace') {
    return buf.slice(0, -1)
  }

  if (key === '-') {
    // Leading `-` is a sign; typed again on an otherwise-empty buffer it
    // toggles back off. After a digit it's a literal fraction hyphen
    // ("6-3/4\"") and must appear where typed — NOT flip the sign (the old
    // behavior turned "5' 6" + `-` into "-5' 6", which read as a negative).
    if (buf === '') return '-'
    if (buf === '-') return ''
    const last = buf[buf.length - 1]
    if (last >= '0' && last <= '9') return buf + key
    return buf
  }

  if (key === '.') {
    // Reject a second dot within the current numeric token — i.e. since the
    // last grammar token boundary (', ", /, space), same spirit as
    // editNumericBuffer's single-dot rule but scoped to the active token.
    const lastBoundary = Math.max(
      buf.lastIndexOf("'"),
      buf.lastIndexOf('"'),
      buf.lastIndexOf('/'),
      buf.lastIndexOf(' '),
    )
    const currentToken = buf.slice(lastBoundary + 1)
    if (currentToken.includes('.')) return buf
    return buf + '.'
  }

  if (key >= '0' && key <= '9') {
    return buf + key
  }

  if (key === "'" || key === '"' || key === '/' || key === ' ') {
    return buf + key
  }

  if (UNIT_SUFFIX_LETTER_RE.test(key)) {
    return buf + key
  }

  return buf
}

/**
 * Parse the numeric buffer to a finite number, or null if the string is
 * empty, just `-`, just `.`, or not a finite number.
 */
export function parseDistance(buf: string): number | null {
  if (buf === '' || buf === '-' || buf === '.' || buf === '-.') return null
  const n = parseFloat(buf)
  if (!isFinite(n)) return null
  return n
}

/**
 * Immutably edit a "dimensions" string buffer — like `editNumericBuffer` but
 * also tolerant of a separator between two values: comma or `x`/`X`
 * (e.g. while typing "3,4", "3x4", "3 x 4").
 *
 * Kept deliberately forgiving — this is only responsible for letting the
 * user type freely; `parseDimensions` does the real validation at commit
 * time (Enter).
 *
 * Rules:
 * - Digits 0–9 are appended.
 * - `.` is appended, rejected if the current numeric token (since the last
 *   separator or length-grammar boundary) already has a dot.
 * - `,` and `x`/`X` are the ONLY dims separators, appended verbatim (so the
 *   buffer can be reformatted/parsed later); a second separator is rejected
 *   once one is already present.
 * - A space is NOT a separator: it is part of the length grammar itself
 *   ("5' 3\"", "1 cm", "3 1/2") and is appended freely (except at the start
 *   of the buffer), so "5' 3\"" followed by `,` still accepts the second
 *   dimension. This deliberately replaces the old behavior where a typed
 *   space consumed the one separator slot and locked out `,`/`x`.
 * - Explicit-unit tokens are appended so each side can carry its own unit
 *   ("1cm,100mm", "5',23\""): unit-suffix letters (except `x`/`X`, which
 *   stay separators), `'`, `"`, and `/`.
 * - `-` after a digit is a literal fraction hyphen ("5' 6-3/4\""); anywhere
 *   else it's ignored (dimensions must be positive; no sign toggling).
 * - `Backspace` removes the last character.
 * - Any other key is ignored.
 */
export function editDimsBuffer(buf: string, key: string): string {
  if (key === 'Backspace') {
    return buf.slice(0, -1)
  }

  if (key === '-') {
    const last = buf[buf.length - 1]
    return last >= '0' && last <= '9' ? buf + key : buf
  }

  if (key === ',' || key === 'x' || key === 'X') {
    if (/[,xX]/.test(buf)) return buf // reject a second separator
    if (buf === '') return buf        // can't start with a separator
    return buf + key
  }

  if (key === ' ') {
    // Space is part of the LENGTH grammar ("5' 3\"", "1 cm", "3 1/2"),
    // never the dims separator — only `,`/`x`/`X` separate the two
    // dimensions. Reject it only at the very start of the buffer.
    if (buf === '') return buf
    return buf + key
  }

  if (key === '.') {
    // Reject a second dot within the current numeric token — since the last
    // dims separator or length-grammar boundary (', ", /, space), matching
    // editLengthBuffer's per-token rule.
    const lastBoundary = Math.max(
      buf.lastIndexOf(','),
      buf.lastIndexOf('x'),
      buf.lastIndexOf('X'),
      buf.lastIndexOf("'"),
      buf.lastIndexOf('"'),
      buf.lastIndexOf('/'),
      buf.lastIndexOf(' '),
    )
    const currentToken = buf.slice(lastBoundary + 1)
    if (currentToken.includes('.')) return buf
    return buf + '.'
  }

  if (key >= '0' && key <= '9') {
    return buf + key
  }

  if (key === "'" || key === '"' || key === '/' || UNIT_SUFFIX_LETTER_RE.test(key)) {
    return buf + key
  }

  return buf
}

/**
 * Parse a typed dimensions buffer into `[width, depth]` (both raw
 * display-unit numbers — NOT converted to meters).
 *
 * Accepts a single value ("3" → [3, 3], a square) or two values separated by
 * a comma, `x`/`X`, or space, with optional surrounding spaces
 * ("3,4" / "3x4" / "3 x 4" → [3, 4]).
 *
 * Returns null if the buffer is empty/malformed, or if either side is not a
 * finite number > 0.
 */
export function parseDimensions(buf: string): [number, number] | null {
  const trimmed = buf.trim()
  if (trimmed === '') return null

  // Split on a single comma/x/X (with optional surrounding spaces) OR plain
  // whitespace. Deliberately NOT filtering empty segments — a leading,
  // trailing, or doubled separator (e.g. "3,", ",4", "3,,4") must produce an
  // empty part so it's rejected below, rather than silently treated as a
  // single value.
  const parts = trimmed.split(/\s*[,xX]\s*|\s+/)

  if (parts.length === 1) {
    const n = parseFloat(parts[0])
    if (!isFinite(n) || n <= 0) return null
    return [n, n]
  }

  if (parts.length === 2) {
    if (parts[0] === '' || parts[1] === '') return null
    const w = parseFloat(parts[0])
    const d = parseFloat(parts[1])
    if (!isFinite(w) || w <= 0 || !isFinite(d) || d <= 0) return null
    return [w, d]
  }

  return null
}

/**
 * Immutably edit the array-copy VCB buffer (the "N× / N÷" refinement typed
 * right after a Move+copy commit).
 *
 * Grammar: digits plus ONE mode token — `x`/`X`/`*` (multiply: N total
 * copies at the committed spacing) or `/` (divide: N copies splitting the
 * committed distance) — in EITHER order, matching SketchUp: the trailing
 * form `5x` / `5/` and the leading form `x5` / `/5` are both accepted.
 * Rules:
 * - A mode token is accepted into an empty buffer (leading form) or after
 *   digits (trailing form); a second mode token is rejected.
 * - Digits start or extend the number: a bare leading digit begins the
 *   trailing form, and digits keep appending after a leading mode token —
 *   but not after a trailing one (`5x3` is malformed).
 * - `x`/`X`/`*` all normalize to `x` in the buffer.
 * - `Backspace` removes the last character.
 * - Any other key is ignored (buffer returned unchanged).
 */
export function editArrayBuffer(buf: string, key: string): string {
  if (key === 'Backspace') {
    return buf.slice(0, -1)
  }

  if (key === 'x' || key === 'X' || key === '*' || key === '/') {
    const token = key === '/' ? '/' : 'x'
    if (buf === '') return token                 // leading form: xN / /N
    if (/^\d+$/.test(buf)) return buf + token    // trailing form: Nx / N/
    return buf // one mode token only
  }

  if (key >= '0' && key <= '9') {
    // Empty or digits-only buffer: (start of) the trailing form. A leading
    // mode token takes digits after it. A COMPLETED trailing form does not.
    if (buf === '' || /^\d+$/.test(buf) || /^[x/]\d*$/.test(buf)) return buf + key
    return buf
  }

  return buf
}

/**
 * Parse the array-copy buffer to a spec, or null when it is empty or
 * malformed (`x`, `3`, `/0`, `x0`, junk). Both token orders parse (`x5` and
 * `5x`, `/5` and `5/`). `multiply` = external array (N total copies at the
 * committed spacing, continuing along the vector); `divide` = internal
 * array (N copies evenly dividing the committed distance). Over-large
 * counts parse — the caller checks them against the kernel's cap
 * (`Scene.max_array_count()`, the single source of truth) and refuses with
 * feedback, so they aren't silently ignored.
 */
export function parseArraySpec(
  buf: string,
): { mode: 'multiply' | 'divide'; count: number } | null {
  const m = /^([xX*/])(\d+)$/.exec(buf) ?? /^(\d+)([xX*/])$/.exec(buf)
  if (m === null) return null
  const [digits, mode] = /\d/.test(m[1]) ? [m[1], m[2]] : [m[2], m[1]]
  const count = parseInt(digits, 10)
  if (!Number.isFinite(count) || count < 1) return null
  return { mode: mode === '/' ? 'divide' : 'multiply', count }
}

/**
 * Compute base + normalize(dir) * distance.
 *
 * If `dir` is ~zero (length < 1e-12), returns `base` unchanged.
 */
export function pointAlong(
  base: [number, number, number],
  dir: [number, number, number],
  distance: number,
): [number, number, number] {
  const len = Math.sqrt(dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2])
  if (len < 1e-12) return [base[0], base[1], base[2]]
  const s = distance / len
  return [base[0] + dir[0] * s, base[1] + dir[1] * s, base[2] + dir[2] * s]
}
