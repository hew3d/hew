import { describe, it, expect } from 'vitest'
import {
  arrowToAxis,
  editNumericBuffer,
  parseDistance,
  pointAlong,
} from './moveInput'

describe('arrowToAxis', () => {
  it('ArrowRight → 0 (X)', () => {
    expect(arrowToAxis('ArrowRight')).toBe(0)
  })

  it('ArrowLeft → 1 (Y)', () => {
    expect(arrowToAxis('ArrowLeft')).toBe(1)
  })

  it('ArrowUp → 2 (Z)', () => {
    expect(arrowToAxis('ArrowUp')).toBe(2)
  })

  it('ArrowDown → null', () => {
    expect(arrowToAxis('ArrowDown')).toBeNull()
  })

  it('other keys → null', () => {
    expect(arrowToAxis('a')).toBeNull()
    expect(arrowToAxis('Enter')).toBeNull()
    expect(arrowToAxis('')).toBeNull()
  })
})

describe('editNumericBuffer', () => {
  it('appends digits', () => {
    expect(editNumericBuffer('', '3')).toBe('3')
    expect(editNumericBuffer('3', '5')).toBe('35')
  })

  it('appends a single dot', () => {
    expect(editNumericBuffer('3', '.')).toBe('3.')
    expect(editNumericBuffer('3.', '5')).toBe('3.5')
  })

  it('rejects a second dot', () => {
    expect(editNumericBuffer('3.5', '.')).toBe('3.5')
    expect(editNumericBuffer('3.', '.')).toBe('3.')
  })

  it('Backspace removes the last character', () => {
    expect(editNumericBuffer('35', 'Backspace')).toBe('3')
    expect(editNumericBuffer('3', 'Backspace')).toBe('')
    expect(editNumericBuffer('', 'Backspace')).toBe('')
  })

  it('- prepends a minus on a plain buffer', () => {
    expect(editNumericBuffer('', '-')).toBe('-')
    expect(editNumericBuffer('5', '-')).toBe('-5')
  })

  it('- strips the leading minus when already negative', () => {
    expect(editNumericBuffer('-5', '-')).toBe('5')
    expect(editNumericBuffer('-', '-')).toBe('')
  })

  it('ignores unknown keys', () => {
    expect(editNumericBuffer('3', 'a')).toBe('3')
    expect(editNumericBuffer('3', 'Enter')).toBe('3')
    expect(editNumericBuffer('3', 'ArrowUp')).toBe('3')
  })

  it('allows a leading minus and dot combination', () => {
    expect(editNumericBuffer('-.', '5')).toBe('-.5')
  })
})

describe('parseDistance', () => {
  it('parses simple numbers', () => {
    expect(parseDistance('3')).toBe(3)
    expect(parseDistance('3.5')).toBeCloseTo(3.5)
    expect(parseDistance('-2')).toBe(-2)
    expect(parseDistance('0')).toBe(0)
  })

  it('returns null for empty buffer', () => {
    expect(parseDistance('')).toBeNull()
  })

  it('returns null for lone minus', () => {
    expect(parseDistance('-')).toBeNull()
  })

  it('returns null for lone dot', () => {
    expect(parseDistance('.')).toBeNull()
  })

  it('returns null for minus-dot', () => {
    expect(parseDistance('-.')).toBeNull()
  })

  it('parses negative decimals', () => {
    expect(parseDistance('-1.5')).toBeCloseTo(-1.5)
  })

  it('parses a value starting with a dot', () => {
    // editNumericBuffer can produce ".5" only if user types dot then digits from empty
    // parseFloat('.5') === 0.5
    expect(parseDistance('.5')).toBeCloseTo(0.5)
  })
})

describe('pointAlong', () => {
  it('moves distance along unit X', () => {
    const pt = pointAlong([0, 0, 0], [1, 0, 0], 3)
    expect(pt[0]).toBeCloseTo(3)
    expect(pt[1]).toBeCloseTo(0)
    expect(pt[2]).toBeCloseTo(0)
  })

  it('moves distance along +Z from a non-origin base', () => {
    const pt = pointAlong([1, 2, 3], [0, 0, 1], 5)
    expect(pt[0]).toBeCloseTo(1)
    expect(pt[1]).toBeCloseTo(2)
    expect(pt[2]).toBeCloseTo(8)
  })

  it('normalizes a non-unit direction', () => {
    // dir (3,4,0) has length 5; distance 5 → moves 3 in X, 4 in Y
    const pt = pointAlong([0, 0, 0], [3, 4, 0], 5)
    expect(pt[0]).toBeCloseTo(3)
    expect(pt[1]).toBeCloseTo(4)
    expect(pt[2]).toBeCloseTo(0)
  })

  it('returns base when dir is ~zero', () => {
    const pt = pointAlong([1, 2, 3], [0, 0, 0], 10)
    expect(pt[0]).toBeCloseTo(1)
    expect(pt[1]).toBeCloseTo(2)
    expect(pt[2]).toBeCloseTo(3)
  })

  it('supports negative distance (move in reverse)', () => {
    const pt = pointAlong([0, 0, 0], [1, 0, 0], -4)
    expect(pt[0]).toBeCloseTo(-4)
    expect(pt[1]).toBeCloseTo(0)
    expect(pt[2]).toBeCloseTo(0)
  })
})
