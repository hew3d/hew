import { describe, expect, it } from 'vitest'

// The .po reader is a plain Node script module shared by the i18n scripts.
const { catalogCoverage, coverageReport, parsePo } = await import('../../scripts/po.mjs')

const PO = `
msgid ""
msgstr ""
"Language: de\\n"

msgid "Line"
msgstr "Linie"

msgctxt "tool"
msgid "Scale"
msgstr ""

msgid "{n, plural, one {# face} other {# faces}}"
msgstr "{n, plural, one {# Fläche} other {# Flächen}}"

#~ msgid "Gone"
#~ msgstr ""
`

describe('catalogCoverage', () => {
  it('counts translated and untranslated messages, skipping header and obsolete entries', () => {
    const c = catalogCoverage(parsePo(PO))
    expect(c.total).toBe(3)
    expect(c.translated).toBe(2)
    expect(c.missing.map((e) => e.msgid)).toEqual(['Scale'])
    expect(c.missing[0].msgctxt).toBe('tool')
    expect(c.percent).toBe(66.6)
  })

  it('never rounds a catalog with a gap up to 100', () => {
    const entries = parsePo(
      Array.from({ length: 2999 }, (_, i) => `msgid "m${i}"\nmsgstr "x"\n`).join('\n') + '\nmsgid "last"\nmsgstr ""\n',
    )
    const c = catalogCoverage(entries)
    expect(c.missing).toHaveLength(1)
    expect(c.percent).toBe(99.9)
  })

  it('reports a complete and an empty catalog', () => {
    expect(catalogCoverage(parsePo('msgid "a"\nmsgstr "b"\n')).percent).toBe(100)
    expect(catalogCoverage([])).toEqual({ total: 0, translated: 0, missing: [], percent: 0 })
  })
})

describe('coverageReport', () => {
  const catalogs: Record<string, string> = {
    de: PO,
    fr: 'msgid "a"\nmsgstr "b"\n',
    xx: 'msgid ""\nmsgstr ""\n"Language: xx\\n"\n',
  }
  const read = (locale: string) => {
    const text = catalogs[locale]
    if (text === undefined) throw new Error(`no catalog for ${locale}`)
    return text
  }
  const texts = (r: { lines: { text: string }[] }) => r.lines.map((l) => l.text)

  it('reports a lagging catalog without failing', () => {
    const r = coverageReport({ locales: ['de', 'fr'], read })
    expect(r.failed).toBe(false)
    expect(texts(r)).toEqual([
      'i18n-check: de 2 of 3 messages translated (66.6%), 1 show in English',
      'i18n-check: fr complete (1 messages)',
    ])
    expect(r.lines.every((l: { error: boolean }) => !l.error)).toBe(true)
  })

  it('names each untranslated message, with its context, under --list', () => {
    const r = coverageReport({ locales: ['de'], read, list: true })
    expect(r.failed).toBe(false)
    expect(texts(r)[1]).toBe('  [tool] "Scale"')
    expect(r.lines).toHaveLength(2)
  })

  it('counts a plural with one empty form and a CRLF catalog correctly', () => {
    const plural = 'msgid "one"\nmsgid_plural "many"\nmsgstr[0] "eins"\nmsgstr[1] ""\n'
    expect(catalogCoverage(parsePo(plural)).missing).toHaveLength(1)
    expect(catalogCoverage(parsePo(PO.replace(/\n/g, '\r\n')))).toMatchObject({ total: 3, translated: 2 })
  })

  it('fails only for a catalog that cannot be read or holds no messages', () => {
    const r = coverageReport({ locales: ['zz', 'xx', 'fr'], read })
    expect(r.failed).toBe(true)
    expect(r.lines.map((l: { error: boolean }) => l.error)).toEqual([true, true, false])
    expect(texts(r)[0]).toContain('zz is shipped but its catalog cannot be read: no catalog for zz')
    expect(texts(r)[1]).toBe('i18n-check: xx is shipped but its catalog holds no messages')
  })
})
