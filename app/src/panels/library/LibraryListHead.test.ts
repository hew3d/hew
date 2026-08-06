/**
 * LibraryListHead — pure-logic tests for the List view's resize math and
 * column-width persistence. The actual pointer-drag wiring isn't exercised
 * here: jsdom has no `PointerEvent` implementation (the same gap
 * `viewport/fovDrag.ts`/`zoomWindowDrag.ts` work around), so this file
 * follows the same convention — the clamping math is pure and exported,
 * tested directly on plain numbers; `LibraryDialog.test.tsx` covers the
 * component wiring (sort headers, initial widths from storage, and which
 * columns render a divider at all).
 */

import { beforeEach, describe, expect, it } from 'vitest'
import { LIST_COL_MIN, loadListColWidths, nextColumnWidth, persistListColWidths } from './LibraryListHead'

describe('nextColumnWidth', () => {
  it('adds the pointer delta to the starting width', () => {
    expect(nextColumnWidth(220, 40, 140)).toBe(260)
  })

  it('shrinks on a negative delta', () => {
    expect(nextColumnWidth(220, -40, 140)).toBe(180)
  })

  it('floors at the minimum rather than going smaller', () => {
    expect(nextColumnWidth(220, -10_000, 140)).toBe(140)
  })

  it('never goes below the minimum even starting AT it', () => {
    expect(nextColumnWidth(140, -5, 140)).toBe(140)
  })
})

describe('loadListColWidths / persistListColWidths', () => {
  beforeEach(() => {
    localStorage.removeItem('hew.library.listCols')
  })

  it('falls back to the built-in defaults when nothing is stored', () => {
    expect(loadListColWidths()).toEqual({ name: 220, type: 92 })
  })

  it('round-trips a persisted value', () => {
    persistListColWidths({ name: 300, type: 120 })
    expect(loadListColWidths()).toEqual({ name: 300, type: 120 })
  })

  it('treats a stored width below the current minimum as absent, not trusted verbatim', () => {
    localStorage.setItem('hew.library.listCols', JSON.stringify({ name: 10, type: 5 }))
    expect(loadListColWidths()).toEqual({ name: 220, type: 92 })
    expect(LIST_COL_MIN.name).toBeGreaterThan(10)
    expect(LIST_COL_MIN.type).toBeGreaterThan(5)
  })

  it('degrades to the default on malformed JSON rather than throwing', () => {
    localStorage.setItem('hew.library.listCols', 'not json')
    expect(loadListColWidths()).toEqual({ name: 220, type: 92 })
  })
})
