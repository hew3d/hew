// @vitest-environment jsdom
//
// The `document`-touching tests below need a DOM; the rest of this project's
// plain `.test.ts` files default to the `node` environment (see
// vitest.config.ts's `environmentMatchGlobs`, which only maps `.test.tsx`).
import { describe, expect, it } from 'vitest'
import { columnCountFromOffsets, measureGridColumns } from './gridMeasure'

describe('columnCountFromOffsets', () => {
  it('counts tiles sharing the first row offset', () => {
    // 5 tiles, 3 per row: row 0 at offset 0, row 1 at offset 120.
    expect(columnCountFromOffsets([0, 0, 0, 120, 120])).toBe(3)
  })

  it('returns the full length when every tile is on one row', () => {
    expect(columnCountFromOffsets([0, 0, 0, 0])).toBe(4)
  })

  it('returns 1 for a single tile', () => {
    expect(columnCountFromOffsets([0])).toBe(1)
  })

  it('returns 1 for an empty grid rather than dividing by zero', () => {
    expect(columnCountFromOffsets([])).toBe(1)
  })

  it('is not confused by out-of-order offsets — only the FIRST value anchors the row', () => {
    // Pathological input (shouldn't happen from real DOM order, but the
    // function must still not blow up or count wrong): first offset 120
    // matches only entries equal to 120.
    expect(columnCountFromOffsets([120, 0, 120, 0])).toBe(2)
  })
})

describe('measureGridColumns', () => {
  it('returns 1 for a null container', () => {
    expect(measureGridColumns(null)).toBe(1)
  })

  it('returns 1 for an empty container', () => {
    const div = document.createElement('div')
    expect(measureGridColumns(div)).toBe(1)
  })

  it('reduces children offsetTops the same way the pure function does', () => {
    // jsdom has no layout engine — every offsetTop reads 0, so every child
    // counts as one row. This still exercises the real DOM traversal (the
    // part `columnCountFromOffsets` alone can't cover); the actual grid
    // math is covered above.
    const div = document.createElement('div')
    div.appendChild(document.createElement('span'))
    div.appendChild(document.createElement('span'))
    div.appendChild(document.createElement('span'))
    expect(measureGridColumns(div)).toBe(3)
  })
})
