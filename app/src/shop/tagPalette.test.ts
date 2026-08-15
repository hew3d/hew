import { describe, it, expect } from 'vitest'
import { colorForTagPath, hexToRgba, UNFILED_TAG_COLOR } from './tagPalette'

describe('colorForTagPath', () => {
  it('is stable for the same path across calls', () => {
    const a = colorForTagPath(['Structure', 'Roof'])
    const b = colorForTagPath(['Structure', 'Roof'])
    expect(a).toBe(b)
  })

  it('returns a 6-digit hex color', () => {
    expect(colorForTagPath(['Anything'])).toMatch(/^#[0-9a-f]{6}$/)
  })

  it('spreads distinct paths across more than one palette entry', () => {
    const paths = [['A'], ['B'], ['C'], ['D'], ['E'], ['F'], ['G'], ['H']]
    const colors = new Set(paths.map((p) => colorForTagPath(p)))
    expect(colors.size).toBeGreaterThan(1)
  })

  it('never returns the reserved Unfiled color for a real tag path', () => {
    const paths = [['A'], ['B'], ['C'], ['D'], ['E'], ['F'], ['G'], ['H'], ['I'], ['J']]
    for (const p of paths) {
      expect(colorForTagPath(p)).not.toBe(UNFILED_TAG_COLOR)
    }
  })

  it('keys off the joined path, matching PartsSheetSection.label\'s own representation', () => {
    expect(colorForTagPath(['Structure', 'Roof'])).toBe(colorForTagPath(['Structure', 'Roof']))
    // A differently-nested path that joins to the same text is a documented,
    // accepted collision — not tested further than "stable and doesn't throw".
    expect(() => colorForTagPath(['Structure / Roof'])).not.toThrow()
  })
})

describe('hexToRgba', () => {
  it('expands a hex color at the given alpha', () => {
    expect(hexToRgba('#c45d3c', 0.08)).toBe('rgba(196, 93, 60, 0.08)')
  })

  it('expands the reserved Unfiled color', () => {
    expect(hexToRgba(UNFILED_TAG_COLOR, 0.08)).toBe('rgba(160, 125, 46, 0.08)')
  })
})
