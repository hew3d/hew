import { describe, expect, it } from 'vitest'
import type { Font } from 'opentype.js'
import { contourSelfIntersects, validateGlyphOutlines } from './outlineValidation'
import type { FlattenCommand, Pt2 } from './flatten'

/** A minimal stand-in for opentype.js's `Font`, exposing only the surface
 *  `validateGlyphOutlines` reads (`unitsPerEm`, `charToGlyph`). Building a
 *  real font from bytes just to pin this pure-geometry classifier would
 *  couple the test to a specific font file's contents (and its parse); a
 *  constructed outline is exactly what the design doc calls for. */
function fakeFont(unitsPerEm: number, glyphs: Record<string, FlattenCommand[]>): Font {
  return {
    unitsPerEm,
    charToGlyph: (char: string) => {
      const commands = glyphs[char]
      return { getPath: () => ({ commands: commands ?? [] }) }
    },
  } as unknown as Font
}

const CLEAN_SQUARE: FlattenCommand[] = [
  { type: 'M', x: 0, y: 0 },
  { type: 'L', x: 10, y: 0 },
  { type: 'L', x: 10, y: 10 },
  { type: 'L', x: 0, y: 10 },
  { type: 'Z' },
]

// A crossed (bowtie) quadrilateral: two "wings" that cross instead of a
// simple loop, the contour-level shape a real self-intersecting glyph
// default instance (a variable font's undelta'd `glyf` outline) produces.
// Asymmetric, so its net signed area happens to be nonzero — exercised
// alongside `SYMMETRIC_BOWTIE` below, whose net signed area is exactly
// zero, to cover both cases now that `validateGlyphOutlines` dedupes via
// `dedupeContourPoints` rather than `weldContour` (see that function's doc
// comment) and so no longer exempts a zero-net-area self-crossing contour.
const BOWTIE: FlattenCommand[] = [
  { type: 'M', x: 0, y: 0 },
  { type: 'L', x: 10, y: 9 },
  { type: 'L', x: 9, y: 0 },
  { type: 'L', x: 0, y: 10 },
  { type: 'Z' },
]

describe('contourSelfIntersects', () => {
  it('a simple square is not self-intersecting', () => {
    const square: Pt2[] = [
      [0, 0],
      [10, 0],
      [10, 10],
      [0, 10],
    ]
    expect(contourSelfIntersects(square)).toBe(false)
  })

  it('a bowtie (crossed) quadrilateral is self-intersecting', () => {
    const bowtie: Pt2[] = [
      [0, 0],
      [10, 10],
      [10, 0],
      [0, 10],
    ]
    expect(contourSelfIntersects(bowtie)).toBe(true)
  })

  it('a triangle can never self-cross (too few edges)', () => {
    const tri: Pt2[] = [
      [0, 0],
      [10, 0],
      [5, 10],
    ]
    expect(contourSelfIntersects(tri)).toBe(false)
  })

  it('a concave (but simple, non-crossing) polygon is not flagged', () => {
    // A "pac-man" notch cut into a square — concave, but every edge stays
    // non-crossing.
    const notched: Pt2[] = [
      [0, 0],
      [10, 0],
      [10, 10],
      [5, 5],
      [0, 10],
    ]
    expect(contourSelfIntersects(notched)).toBe(false)
  })
})

// A PERFECTLY symmetric bowtie: two crossed lobes whose shoelace areas
// cancel to EXACTLY zero net signed area (unlike BOWTIE above, which was
// deliberately built asymmetric to dodge this). This is the shape
// adversarial-review finding 3 is about: `weldContour`'s degenerate-area
// drop (`flatten.ts`) treats "zero net signed area" as "no real geometry"
// and discards the contour before `contourSelfIntersects` ever sees it —
// even though a self-crossing bowtie has real extent and is never correct
// glyph geometry. A font's actual undelta'd default instance can produce
// exactly this shape (two symmetric crossed lobes), so this is not just a
// synthetic edge case.
const SYMMETRIC_BOWTIE: FlattenCommand[] = [
  { type: 'M', x: 0, y: 0 },
  { type: 'L', x: 10, y: 10 },
  { type: 'L', x: 10, y: 0 },
  { type: 'L', x: 0, y: 10 },
  { type: 'Z' },
]

describe('validateGlyphOutlines', () => {
  it('flags a symmetric bowtie whose net signed area is exactly zero (finding 3: must not be silently exempted)', () => {
    const font = fakeFont(1000, { X: SYMMETRIC_BOWTIE })
    const warnings = validateGlyphOutlines(font, 'X', 0.1)
    expect(warnings).toEqual([{ char: 'X' }])
  })

  it('flags a glyph whose flattened outline self-intersects', () => {
    const font = fakeFont(1000, { X: BOWTIE })
    const warnings = validateGlyphOutlines(font, 'X', 0.1)
    expect(warnings).toEqual([{ char: 'X' }])
  })

  it('does not flag a glyph with a clean simple outline', () => {
    const font = fakeFont(1000, { A: CLEAN_SQUARE })
    const warnings = validateGlyphOutlines(font, 'A', 0.1)
    expect(warnings).toEqual([])
  })

  it('checks every distinct non-whitespace character, deduplicated', () => {
    const font = fakeFont(1000, { A: CLEAN_SQUARE, X: BOWTIE })
    const warnings = validateGlyphOutlines(font, 'AXA XA', 0.1)
    expect(warnings).toEqual([{ char: 'X' }])
  })

  it('a glyph with no outline (space, unmapped char) is never flagged', () => {
    const font = fakeFont(1000, { A: CLEAN_SQUARE })
    // 'A' is clean; '?' and the space have no registered outline at all.
    const warnings = validateGlyphOutlines(font, 'A ?', 0.1)
    expect(warnings).toEqual([])
  })

  it("an 'O'-style outer+counter (nested, non-crossing) loop pair is not flagged", () => {
    const outer: FlattenCommand[] = [
      { type: 'M', x: 0, y: 0 },
      { type: 'L', x: 20, y: 0 },
      { type: 'L', x: 20, y: 20 },
      { type: 'L', x: 0, y: 20 },
      { type: 'Z' },
    ]
    const inner: FlattenCommand[] = [
      { type: 'M', x: 5, y: 5 },
      { type: 'L', x: 5, y: 15 },
      { type: 'L', x: 15, y: 15 },
      { type: 'L', x: 15, y: 5 },
      { type: 'Z' },
    ]
    const font = fakeFont(1000, { O: [...outer, ...inner] })
    const warnings = validateGlyphOutlines(font, 'O', 0.1)
    expect(warnings).toEqual([])
  })
})
