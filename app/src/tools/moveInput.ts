/**
 * moveInput — pure helpers for axis-lock and numeric VCB entry in MoveTool.
 *
 * No three.js or DOM imports — fully testable in Node/vitest.
 */

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
