// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { i18n } from '@lingui/core'
import { msg } from '@lingui/core/macro'
import { activateLocale } from './activate'
import { isRtl, textDirection, horizontalSign } from './direction'
import { SHIPPED_LOCALES } from './locales'

const here = dirname(fileURLToPath(import.meta.url))
const appRoot = resolve(here, '../..')

// Simple PO parser for unit testing key parity
interface PoEntry {
  msgid: string
  msgstr: string
  context?: string
}

function parsePoSimple(content: string): PoEntry[] {
  const entries: PoEntry[] = []
  let cur: Partial<PoEntry> | null = null
  let field: 'msgid' | 'msgstr' | 'msgctxt' | null = null

  for (const raw of content.split('\n')) {
    const line = raw.trim()
    if (line === '') {
      if (cur?.msgid) {
        entries.push(cur as PoEntry)
      }
      cur = null
      field = null
      continue
    }
    if (line.startsWith('#')) continue

    const ctxtMatch = /^msgctxt\s+"(.*)"$/.exec(line)
    if (ctxtMatch) {
      if (!cur) cur = { msgid: '', msgstr: '' }
      cur.context = ctxtMatch[1]
      field = 'msgctxt'
      continue
    }

    const idMatch = /^msgid\s+"(.*)"$/.exec(line)
    if (idMatch) {
      if (!cur) cur = { msgid: '', msgstr: '' }
      cur.msgid = idMatch[1]
      field = 'msgid'
      continue
    }

    const strMatch = /^msgstr\s+"(.*)"$/.exec(line)
    if (strMatch) {
      if (!cur) cur = { msgid: '', msgstr: '' }
      cur.msgstr = strMatch[1]
      field = 'msgstr'
      continue
    }

    const contMatch = /^"(.*)"$/.exec(line)
    if (contMatch && cur && field) {
      if (field === 'msgid') cur.msgid += contMatch[1]
      else if (field === 'msgstr') cur.msgstr += contMatch[1]
      else if (field === 'msgctxt') cur.context = (cur.context ?? '') + contMatch[1]
    }
  }

  if (cur?.msgid) {
    entries.push(cur as PoEntry)
  }
  return entries
}

const SYSTEM_DEFAULT = msg`System default`
const OBJECT_TREE = msg`Object Tree`
const MEASUREMENTS = msg`Measurements`

describe('Arabic Localization and RTL Support', () => {
  it('ar is registered in SHIPPED_LOCALES', () => {
    expect(SHIPPED_LOCALES).toContain('ar')
  })

  it('switching to ar sets document direction to rtl and lang to ar', async () => {
    await activateLocale('ar')
    expect(i18n.locale).toBe('ar')
    expect(document.documentElement.lang).toBe('ar')
    expect(document.documentElement.dir).toBe('rtl')
    expect(isRtl()).toBe(true)
    expect(textDirection()).toBe('rtl')
    expect(horizontalSign()).toBe(-1)
  })

  it('switching back to en sets document direction to ltr and lang to en', async () => {
    await activateLocale('en')
    expect(i18n.locale).toBe('en')
    expect(document.documentElement.lang).toBe('en')
    expect(document.documentElement.dir).toBe('ltr')
    expect(isRtl()).toBe(false)
    expect(textDirection()).toBe('ltr')
    expect(horizontalSign()).toBe(1)
  })

  it('every entry in en/messages.po exists and is non-empty in ar/messages.po', () => {
    const enPath = resolve(appRoot, 'src/locales/en/messages.po')
    const arPath = resolve(appRoot, 'src/locales/ar/messages.po')

    const enEntries = parsePoSimple(readFileSync(enPath, 'utf8'))
    const arEntries = parsePoSimple(readFileSync(arPath, 'utf8'))

    expect(enEntries.length).toBeGreaterThan(0)
    expect(arEntries.length).toBe(enEntries.length)

    const arMap = new Map<string, string>()
    for (const e of arEntries) {
      const key = `${e.context ?? ''}\u0004${e.msgid}`
      arMap.set(key, e.msgstr)
    }

    const missingOrEmpty: string[] = []
    for (const e of enEntries) {
      const key = `${e.context ?? ''}\u0004${e.msgid}`
      const translation = arMap.get(key)
      if (translation === undefined || translation.trim() === '') {
        missingOrEmpty.push(e.msgid)
      }
    }

    expect(missingOrEmpty).toEqual([])
  })

  it('translates core CAD terms accurately in Arabic', async () => {
    await activateLocale('ar')
    expect(i18n._(SYSTEM_DEFAULT)).toBe('افتراضي النظام')
    await activateLocale('en')
  })
})
