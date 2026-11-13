// Pure parts of scripts/i18n-menus.mjs, shared with nativeMenuParity.test.ts.
// The table is read by its documented shape (one entry per line) rather than
// executed, so no macro transform is needed here.

import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { PSEUDO_LOCALE, SOURCE_LOCALE } from '../src/i18n/locales.ts'
import { parsePo, poLookup } from './po.mjs'

export const APP_DIR = resolve(dirname(fileURLToPath(import.meta.url)), '..')
export const MENU_LOCALES_DIR = resolve(APP_DIR, '../shells/tauri/src-tauri/locales')

/**
 * The text a `msg` template yields at runtime. The table is read from source
 * rather than executed, so the escapes a template literal would decode
 * (`\n` above all) must be decoded here; left raw they reach the native
 * dialog as a literal backslash and "n".
 */
export function decodeTemplateEscapes(text) {
  return text.replace(/\\(u\{[0-9a-fA-F]+\}|u[0-9a-fA-F]{4}|x[0-9a-fA-F]{2}|[\s\S])/g, (_, e) => {
    const wide = e.length > 1
    switch (wide ? e[0] : e) {
      case 'n':
        return '\n'
      case 't':
        return '\t'
      case 'r':
        return '\r'
      case 'u':
        if (!wide) return e
        return String.fromCodePoint(parseInt(e.replace(/[u{}]/g, ''), 16))
      case 'x':
        if (!wide) return e
        return String.fromCharCode(parseInt(e.slice(1), 16))
      default:
        return e
    }
  })
}

/** [{ key, context, message }] from the table's source text. */
export function readNativeMenuTable(source) {
  const rows = []
  const simple = /^\s*'([^']+)':\s*msg`([^`]+)`,?\s*$/
  const ctx = /^\s*'([^']+)':\s*msg\(\{\s*context:\s*'([^']+)',\s*message:\s*'([^']+)'\s*\}\),?\s*$/
  for (const line of source.split('\n')) {
    let m = simple.exec(line)
    if (m) {
      rows.push({ key: m[1], context: '', message: decodeTemplateEscapes(m[2]) })
      continue
    }
    m = ctx.exec(line)
    if (m) rows.push({ key: m[1], context: m[2], message: decodeTemplateEscapes(m[3]) })
  }
  return rows
}

export function labelsFor(rows, lookup) {
  const out = {}
  for (const { key, context, message } of rows) {
    const translated = lookup?.get(`${context}\u0004${message}`)
    out[key] = translated ? translated : message
  }
  return out
}

export function render(labels) {
  return JSON.stringify(labels, null, 2) + '\n'
}


export function readTableRows() {
  const source = readFileSync(resolve(APP_DIR, 'src/i18n/nativeMenu.ts'), 'utf8')
  const rows = readNativeMenuTable(source)
  // The parser reads one documented shape per line; an entry written any
  // other way would vanish silently, so count every keyed line too.
  const keyedLines = (source.match(/^\s*'[^']+':\s*msg/gm) ?? []).length
  if (keyedLines !== rows.length) {
    throw new Error(
      `i18n-menus: nativeMenu.ts has ${keyedLines} entries but ${rows.length} parsed; keep one key: msg\`…\` or msg({ context, message }) per line`,
    )
  }
  return rows
}

const ACCENTS = {
  a: 'à', b: 'ƀ', c: 'ć', d: 'ď', e: 'ē', f: 'ƒ', g: 'ĝ', h: 'ĥ', i: 'ĩ', j: 'ĵ', k: 'ķ', l: 'ĺ', m: 'ɱ',
  n: 'ń', o: 'ō', p: 'ƥ', q: 'ɋ', r: 'ŕ', s: 'ś', t: 'ţ', u: 'ũ', v: 'ṽ', w: 'ŵ', x: 'ẋ', y: 'ý', z: 'ź',
}

/**
 * Pseudo-localize a menu label the way the app's pseudo-locale reads
 * (accented letters, ⟦ ⟧ markers, 30% padding), keeping {placeholders} and
 * newlines intact. The native menu has no compiled catalog to borrow from,
 * so this mirrors lingui.config.ts's pseudoLocale for the one table.
 */
export function pseudoLabel(text) {
  const body = text.replace(/\{\w+\}|\n|[A-Za-z]/g, (m) => {
    if (m.length > 1 || m === '\n') return m
    const lower = ACCENTS[m.toLowerCase()] ?? m
    return m === m.toLowerCase() ? lower : lower.toUpperCase()
  })
  return `⟦ ${body}${'.'.repeat(Math.ceil(text.length * 0.3))} ⟧`
}

/** The JSON text for `locale`'s menu file. */
export function menuFileFor(rows, locale) {
  if (locale === PSEUDO_LOCALE) {
    const english = labelsFor(rows, null)
    return render(Object.fromEntries(Object.entries(english).map(([k, v]) => [k, pseudoLabel(v)])))
  }
  const lookup =
    locale === SOURCE_LOCALE
      ? null
      : poLookup(parsePo(readFileSync(resolve(APP_DIR, 'src/locales', locale, 'messages.po'), 'utf8')))
  return render(labelsFor(rows, lookup))
}
