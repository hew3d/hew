import { existsSync, readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeEach, describe, expect, it } from 'vitest'
import { requiresPreview } from '../test/requiresPreview'
import { cleanText, displayName } from './pluginText'
import { errorText, PluginError, toPluginError } from './pluginError'

beforeEach((ctx) => requiresPreview(ctx, 'plugins'))

describe('cleanText', () => {
  it('removes bidirectional, zero-width and control characters and tidies space', () => {
    expect(cleanText('A‮b​c\u0007  d\n\te', 64)).toBe('Abc d e')
    expect(cleanText('﻿  hi  ', 64)).toBe('hi')
    expect(cleanText('x\u{E0041}y', 64)).toBe('xy')
  })
  it('cuts to the cap with an ellipsis, counting characters', () => {
    expect(cleanText('abcdefghij', 5)).toBe('abcd…')
    expect(Array.from(cleanText('日'.repeat(10), 4))).toHaveLength(4)
    expect(cleanText('abc', 3)).toBe('abc')
  })
})

describe('cleanText rules', () => {
  it('treats every Rust whitespace (including U+0085) as one collapsing space', () => {
    expect(cleanText('a\u0085b\u00A0\u2003c\u3000d\u2028e', 64)).toBe('a b c d e')
    // U+FEFF is not whitespace for Rust: it is removed, not a space.
    expect(cleanText('a\uFEFFb', 64)).toBe('ab')
  })
  it('removes the blank fillers and Arabic and Syriac format marks', () => {
    for (const c of ['\u2800', '\u3164', '\u115F', '\u1160', '\uFFA0', '\u0600', '\u0605', '\u061C', '\u06DD', '\u070F', '\u08E2', '\u180E']) {
      expect(cleanText(`a${c}b`, 64), c.codePointAt(0)!.toString(16)).toBe('ab')
    }
  })
  it('keeps at most two combining marks in a row', () => {
    expect(cleanText('e\u0301\u0302\u0303\u0304x', 64)).toBe('e\u0301\u0302x')
    expect(cleanText('a\u20D0\u20D1\u20D2 b\u0300\u0301\u0302', 64)).toBe('a\u20D0\u20D1 b\u0300\u0301')
  })
})

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')
const TABLE = join(ROOT, 'crates/plugin-host/tests/fixtures/clean_text.json')

describe('cleanText against the table the Rust side shares', () => {
  it('gives every row the same answer as clean_text', () => {
    // The table is written by the core crate; this fails until it exists.
    expect(existsSync(TABLE), `${TABLE} is missing: the shared clean_text rows have not landed`).toBe(true)
    const rows = JSON.parse(readFileSync(TABLE, 'utf8')) as { in: string; max: number; out: string }[]
    expect(rows.length).toBeGreaterThan(0)
    for (const row of rows) expect(cleanText(row.in, row.max), JSON.stringify(row.in)).toBe(row.out)
  })
})

describe('displayName', () => {
  it('uses the English name, cleaned, else the id', () => {
    expect(displayName({ id: 'a.b', name: 'Tool‮' })).toBe('Tool')
    expect(displayName({ id: 'a.b', name: { de: 'Werkzeug', en: 'Tool' } })).toBe('Tool')
    expect(displayName({ id: 'a.b', name: { de: 'Werkzeug' } })).toBe('Werkzeug')
    expect(displayName({ id: 'a.b', name: { $t: 'key' } })).toBe('a.b')
    expect(displayName({ id: 'a.b', name: '​' })).toBe('a.b')
    expect(Array.from(displayName({ id: 'a.b', name: 'n'.repeat(200) })).length).toBe(64)
  })
})

describe('errors', () => {
  it('splits the shell form and the kernel form', () => {
    expect(toPluginError('reserved_id: "org.hew3d.x" is kept.')).toMatchObject({ refusal: 'reserved_id', message: '"org.hew3d.x" is kept.' })
    expect(toPluginError('INSTALL_PACKAGE_NOT_STORE_ONLY: Compressed.')).toMatchObject({ refusal: 'package_not_store_only', message: 'Compressed.' })
    expect(toPluginError('MANIFEST_MANIFEST_MALFORMED: Bad.')).toMatchObject({ refusal: 'manifest_malformed' })
    expect(toPluginError('PLUGIN_BAD_INPUT: x')).toMatchObject({ refusal: 'plugin_bad_input' })
  })
  it('keeps text that has no name, and colons inside the text', () => {
    expect(errorText(new Error('The plugin folder could not be changed: disk full.'))).toBe('The plugin folder could not be changed: disk full.')
    expect(errorText('io: a: b')).toBe('a: b')
    const e = new PluginError('x', 'y')
    expect(toPluginError(e)).toBe(e)
  })
})
