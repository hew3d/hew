import { describe, expect, it } from 'vitest'
import { parseGradientStops } from './shopGradientBackground'

describe('parseGradientStops', () => {
  it('parses the light --viewport-bg token (3 explicit-percent stops)', () => {
    expect(parseGradientStops('linear-gradient(180deg,#edeae3 0%,#e2ded5 60%,#d8d3c9 100%)')).toEqual([
      { color: '#edeae3', offset: 0 },
      { color: '#e2ded5', offset: 0.6 },
      { color: '#d8d3c9', offset: 1 },
    ])
  })

  it('parses the dark --viewport-bg token (2 bare stops, spread evenly)', () => {
    expect(parseGradientStops('linear-gradient(180deg, #17181c, #121317)')).toEqual([
      { color: '#17181c', offset: 0 },
      { color: '#121317', offset: 1 },
    ])
  })

  it('parses an angle-less gradient (direction keyword form)', () => {
    expect(parseGradientStops('linear-gradient(to bottom, #111111, #222222)')).toEqual([
      { color: '#111111', offset: 0 },
      { color: '#222222', offset: 1 },
    ])
  })

  it('tolerates surrounding whitespace/newlines (a live getComputedStyle read)', () => {
    expect(parseGradientStops('  linear-gradient(180deg, #111 0%, #222 100%)  \n')).toEqual([
      { color: '#111', offset: 0 },
      { color: '#222', offset: 1 },
    ])
  })

  it('returns [] for a non-gradient value', () => {
    expect(parseGradientStops('#1a1a1a')).toEqual([])
    expect(parseGradientStops('')).toEqual([])
    expect(parseGradientStops('radial-gradient(#111, #222)')).toEqual([])
  })
})
