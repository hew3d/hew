import { describe, expect, it } from 'vitest'
import { drawKindFor } from './drawKind'

describe('drawKindFor', () => {
  it('imported drawing is plain, owned or not', () => {
    expect(drawKindFor(true, false)).toBe('plain')
    expect(drawKindFor(true, true)).toBe('plain')
  })
  it('drawing made in Hew keeps its ink', () => {
    expect(drawKindFor(false, true)).toBe('attached')
    expect(drawKindFor(false, false)).toBe('free')
  })
})
