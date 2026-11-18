/* oxlint-disable lingui/no-unlocalized-strings -- plugin install copy and identifiers; English only until the feature graduates. */
/**
 * Plugin-written text, made safe to show. A manifest string can hide
 * characters or pass for Hew's own words, so anything the app shows that a
 * plugin wrote goes through `cleanText` first, the same rules the Rust side
 * applies before it builds a consent payload.
 */

import type { LocalizableString, PluginManifest } from './types'

/** What Rust's `char::is_whitespace` accepts (Unicode White_Space). Each
 *  becomes one space; JavaScript's `\s` differs (it takes U+FEFF and not
 *  U+0085), so the set is spelled out. */
const WHITESPACE = /[\u0009-\u000D\u0020\u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]/
/** Other control characters (C0, DEL, C1). Removed. */
const CONTROL = /[\u0000-\u001F\u007F-\u009F]/
/** Characters that hide or reorder text, or draw as blank: the bidirectional
 *  controls, zero-width characters, soft hyphen, variation selectors, tag
 *  characters, the blank Hangul and braille fillers, and the Arabic and
 *  Syriac format marks. Kept in step with `is_invisible` and `clean_text` in
 *  `crates/plugin-host`. */
const INVISIBLE =
  /[\u00AD\u034F\u0600-\u0605\u061C\u06DD\u070F\u08E2\u115F\u1160\u17B4\u17B5\u180B-\u180F\u200B-\u200F\u202A-\u202E\u2060-\u206F\u2800\u3164\uFE00-\uFE0F\uFEFF\uFFA0\uFFF0-\uFFFB\u{1D173}-\u{1D17A}\u{E0000}-\u{E0FFF}]/u
/** Combining marks; a run of more than two is cut to two. */
const COMBINING = /[\u0300-\u036F\u1AB0-\u1AFF\u1DC0-\u1DFF\u20D0-\u20FF\uFE20-\uFE2F]/
const MAX_COMBINING_RUN = 2

export const MAX_NAME_CHARS = 64

/** The same rules as `clean_text` in `crates/plugin-host`: whitespace runs
 *  become one space and the ends are trimmed; other control and invisible
 *  characters are removed; at most two combining marks in a row stay; text
 *  over `max` characters is cut to fit with a trailing ellipsis. */
export function cleanText(text: string, max: number): string {
  const out: string[] = []
  let space = false
  let marks = 0
  for (const c of text) {
    if (WHITESPACE.test(c)) {
      space = true
      marks = 0
      continue
    }
    if (CONTROL.test(c) || INVISIBLE.test(c)) continue
    if (COMBINING.test(c)) {
      if (marks >= MAX_COMBINING_RUN) continue
      marks++
    } else {
      marks = 0
    }
    if (space && out.length > 0) out.push(' ')
    space = false
    out.push(c)
  }
  if (max === 0) return ''
  if (out.length <= max) return out.join('')
  return out.slice(0, max - 1).join('').trimEnd() + '\u2026'
}

/** The English form of a manifest string, or null for a catalog key. */
function english(text: LocalizableString | null | undefined): string | null {
  if (typeof text === 'string') return text
  if (text === null || text === undefined || typeof text !== 'object') return null
  if ('$t' in text) return null
  const map = text as Record<string, string>
  const pick = map.en ?? Object.values(map)[0]
  return typeof pick === 'string' ? pick : null
}

/** The name a row shows: the manifest's English name, cleaned; the id when
 *  there is none. */
export function displayName(manifest: Pick<PluginManifest, 'id' | 'name'>): string {
  const name = cleanText(english(manifest.name) ?? '', MAX_NAME_CHARS)
  return name === '' ? manifest.id : name
}

/** Orders by code point, as the desktop store's sorted map orders its keys
 *  (UTF-8 byte order); `<` on strings compares UTF-16 code units, which puts
 *  a character outside the Basic Multilingual Plane before U+E000..U+FFFF. */
export function compareCodePoints(a: string, b: string): number {
  let i = 0
  let j = 0
  while (i < a.length && j < b.length) {
    const x = a.codePointAt(i) as number
    const y = b.codePointAt(j) as number
    if (x !== y) return x < y ? -1 : 1
    i += x > 0xffff ? 2 : 1
    j += y > 0xffff ? 2 : 1
  }
  return Number(i < a.length) - Number(j < b.length)
}
