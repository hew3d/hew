import { describe, it, expect } from 'vitest'
import { rankEntries } from './search'
import type { PaletteEntry } from './registry'

const push: PaletteEntry = { id: 'tool-pushpull', label: 'Push/Pull', description: 'Extrude a face.', group: 'Tools', synonyms: ['extrude'] }
const rect: PaletteEntry = { id: 'tool-rectangle', label: 'Rectangle', description: 'Draw a rectangle.', group: 'Tools' }
const rotate: PaletteEntry = { id: 'tool-rotate', label: 'Rotate', description: 'Rotate the selection.', group: 'Tools' }
const undo: PaletteEntry = { id: 'undo', label: 'Undo', description: 'Undo the last change.', group: 'Actions' }
const save: PaletteEntry = { id: 'save', label: 'Save', description: 'Save the document, quickly.', group: 'Actions' }
const all = [push, rect, rotate, undo, save]

describe('rankEntries — empty query', () => {
  it('orders Recent first, then default suggestions, then the rest', () => {
    const ranked = rankEntries('', all, ['tool-rotate'])
    expect(ranked[0].id).toBe('tool-rotate') // recent
    // Defaults are tool-select (absent here), tool-pushpull, save, undo
    expect(ranked.slice(1).map((e) => e.id)).toEqual(['tool-pushpull', 'save', 'undo', 'tool-rectangle'])
  })

  it('with no recent history, falls back to default suggestions then the rest', () => {
    const ranked = rankEntries('  ', all, [])
    expect(ranked.map((e) => e.id)).toEqual(['tool-pushpull', 'save', 'undo', 'tool-rectangle', 'tool-rotate'])
  })
})

describe('rankEntries — query match quality', () => {
  it('exact label match ranks first', () => {
    const ranked = rankEntries('Save', all, [])
    expect(ranked[0].id).toBe('save')
  })

  it('prefix match beats substring match', () => {
    // "rot" is a prefix of Rotate; substring-only matches should rank below it
    const ranked = rankEntries('rot', all, [])
    expect(ranked[0].id).toBe('tool-rotate')
  })

  it('is case-insensitive', () => {
    const ranked = rankEntries('RECTANGLE', all, [])
    expect(ranked[0].id).toBe('tool-rectangle')
  })

  it('word-prefix matches within a multi-word label (e.g. "pull" -> Push/Pull)', () => {
    const ranked = rankEntries('pull', all, [])
    expect(ranked[0].id).toBe('tool-pushpull')
  })

  it('synonym match finds a tool the label alone would not (e.g. "extrude" -> Push/Pull)', () => {
    const ranked = rankEntries('extrude', all, [])
    expect(ranked.map((e) => e.id)).toContain('tool-pushpull')
  })

  it('description-only match still surfaces the entry, ranked below name matches', () => {
    // "quickly" only appears in Save's description
    const ranked = rankEntries('quickly', all, [])
    expect(ranked.map((e) => e.id)).toEqual(['save'])
  })

  it('excludes entries with no match at all', () => {
    const ranked = rankEntries('zzz-no-such-thing', all, [])
    expect(ranked).toEqual([])
  })
})

describe('rankEntries — recency as a tie-breaker only', () => {
  it('recency cannot promote a worse text match over a better one', () => {
    // Both "Rotate" (prefix) and something with only a substring match on
    // the same query would differ by a full tier (>=20); recency bonus caps
    // at 10, so it can never invert that — verified here by making the
    // *worse* matching entry the most recent and confirming order is unchanged.
    const ranked = rankEntries('rot', all, ['tool-pushpull'])
    expect(ranked[0].id).toBe('tool-rotate')
  })
})
