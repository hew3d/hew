import { describe, it, expect } from 'vitest'
import {
  arrowToAxis,
  editNumericBuffer,
  editLengthBuffer,
  editDimsBuffer,
  isLengthInputKey,
  parseDistance,
  parseDimensions,
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

describe('editLengthBuffer', () => {
  it('matches editNumericBuffer for digits/dot/Backspace in metric formats', () => {
    for (const format of ['m', 'cm', 'mm'] as const) {
      expect(editLengthBuffer('', '3', format)).toBe(editNumericBuffer('', '3'))
      expect(editLengthBuffer('3', '.', format)).toBe(editNumericBuffer('3', '.'))
      expect(editLengthBuffer('3.5', '.', format)).toBe(editNumericBuffer('3.5', '.'))
      expect(editLengthBuffer('35', 'Backspace', format)).toBe(editNumericBuffer('35', 'Backspace'))
    }
  })

  it('- is a leading sign on an empty buffer, toggling back off when re-typed', () => {
    for (const format of ['m', 'arch'] as const) {
      expect(editLengthBuffer('', '-', format)).toBe('-')
      expect(editLengthBuffer('-', '-', format)).toBe('')
      expect(editLengthBuffer('-', '5', format)).toBe('-5')
    }
  })

  it('- after a digit is a literal fraction hyphen, not a sign flip', () => {
    // Typing 5' 6-3/4" keystroke by keystroke must show the hyphen where
    // typed — the old sign-toggle turned "5' 6" + `-` into "-5' 6", which
    // read as a negative value.
    let buf = ''
    for (const key of ['5', "'", ' ', '6', '-', '3', '/', '4', '"']) {
      buf = editLengthBuffer(buf, key, 'm')
    }
    expect(buf).toBe('5\' 6-3/4"')
    // But not after a mark, a space, or another hyphen.
    expect(editLengthBuffer("5'", '-', 'arch')).toBe("5'")
    expect(editLengthBuffer('5 ', '-', 'arch')).toBe('5 ')
    expect(editLengthBuffer('6-', '-', 'arch')).toBe('6-')
  })

  it('accepts feet/inch/fraction tokens in EVERY format (explicit units work in any mode)', () => {
    for (const format of ['m', 'cm', 'mm', 'arch', 'frac_in', 'dec_in'] as const) {
      expect(editLengthBuffer('5', "'", format)).toBe("5'")
      expect(editLengthBuffer("5'", '3', format)).toBe("5'3")
      expect(editLengthBuffer("5'3", '"', format)).toBe('5\'3"')
      expect(editLengthBuffer('3', '/', format)).toBe('3/')
      expect(editLengthBuffer('3', ' ', format)).toBe('3 ')
      expect(editLengthBuffer("5'", ' ', format)).toBe("5' ")
    }
  })

  it('accepts unit-suffix letters in EVERY format so "1cm"/"5ft" can be typed anywhere', () => {
    for (const format of ['m', 'cm', 'mm', 'arch', 'frac_in', 'dec_in'] as const) {
      expect(editLengthBuffer('1', 'c', format)).toBe('1c')
      expect(editLengthBuffer('1c', 'm', format)).toBe('1cm')
      expect(editLengthBuffer('5', 'f', format)).toBe('5f')
      expect(editLengthBuffer('5f', 't', format)).toBe('5ft')
      expect(editLengthBuffer('6', 'i', format)).toBe('6i')
      expect(editLengthBuffer('6i', 'n', format)).toBe('6in')
      expect(editLengthBuffer('3', 'k', format)).toBe('3k')
      // Uppercase letters are accepted too (parsing is case-insensitive).
      expect(editLengthBuffer('1', 'M', format)).toBe('1M')
    }
  })

  it('still accepts digits, dot, minus, Backspace in imperial formats', () => {
    expect(editLengthBuffer('', '6', 'arch')).toBe('6')
    expect(editLengthBuffer('6', '0', 'arch')).toBe('60')
    expect(editLengthBuffer('60', '.', 'arch')).toBe('60.')
    expect(editLengthBuffer('60.', '1', 'arch')).toBe('60.1')
    expect(editLengthBuffer('', '-', 'arch')).toBe('-')
    expect(editLengthBuffer('5', '-', 'arch')).toBe('5-')
    expect(editLengthBuffer('60', 'Backspace', 'arch')).toBe('6')
  })

  it('rejects a second dot within the current token in imperial formats', () => {
    expect(editLengthBuffer('60.1', '.', 'dec_in')).toBe('60.1')
    // After a token boundary (space), a new dot is allowed again.
    expect(editLengthBuffer('5 3.5', '.', 'arch')).toBe('5 3.5')
    expect(editLengthBuffer('5 3', '.', 'arch')).toBe('5 3.')
  })

  it('ignores unknown keys in imperial formats', () => {
    expect(editLengthBuffer('3', 'a', 'arch')).toBe('3')
    expect(editLengthBuffer('3', 'Enter', 'frac_in')).toBe('3')
  })
})

describe('isLengthInputKey', () => {
  it('accepts digits, dot, minus, marks, space, Backspace', () => {
    for (const key of ['0', '9', '.', '-', "'", '"', '/', ' ', 'Backspace']) {
      expect(isLengthInputKey(key)).toBe(true)
    }
  })

  it('accepts unit-suffix letters (both cases)', () => {
    for (const key of ['m', 'c', 'k', 'f', 't', 'i', 'n', 'M', 'F', 'N']) {
      expect(isLengthInputKey(key)).toBe(true)
    }
  })

  it('rejects other keys', () => {
    for (const key of ['a', 'x', 'X', 'q', 'Enter', 'ArrowUp', 'Escape']) {
      expect(isLengthInputKey(key)).toBe(false)
    }
  })
})

describe('editDimsBuffer', () => {
  it('appends digits', () => {
    expect(editDimsBuffer('', '3')).toBe('3')
    expect(editDimsBuffer('3', '4')).toBe('34')
  })

  it('appends a comma separator', () => {
    expect(editDimsBuffer('3', ',')).toBe('3,')
    expect(editDimsBuffer('3,', '4')).toBe('3,4')
  })

  it('appends an x/X separator', () => {
    expect(editDimsBuffer('3', 'x')).toBe('3x')
    expect(editDimsBuffer('3', 'X')).toBe('3X')
  })

  it('appends a space separator', () => {
    expect(editDimsBuffer('3', ' ')).toBe('3 ')
  })

  it('rejects a separator at the start of the buffer', () => {
    expect(editDimsBuffer('', ',')).toBe('')
    expect(editDimsBuffer('', 'x')).toBe('')
    expect(editDimsBuffer('', ' ')).toBe('')
  })

  it('rejects a second separator', () => {
    expect(editDimsBuffer('3,4', ',')).toBe('3,4')
    expect(editDimsBuffer('3x', 'x')).toBe('3x')
    expect(editDimsBuffer('3 ', 'x')).toBe('3 ')
  })

  it('allows a dot per side, rejects a second dot on the same side', () => {
    expect(editDimsBuffer('3.', '5')).toBe('3.5')
    expect(editDimsBuffer('3.5', '.')).toBe('3.5')
    // After a separator, a new dot is allowed (new "side")
    expect(editDimsBuffer('3.5,4', '.')).toBe('3.5,4.')
    expect(editDimsBuffer('3.5,4.', '.')).toBe('3.5,4.')
  })

  it('- after a digit is a literal fraction hyphen; elsewhere ignored (dimensions are unsigned)', () => {
    expect(editDimsBuffer('3', '-')).toBe('3-') // "6-3/4"-style fraction hyphen
    expect(editDimsBuffer('', '-')).toBe('')    // no leading sign
    expect(editDimsBuffer("5'", '-')).toBe("5'") // not after a mark
    expect(editDimsBuffer('3,', '-')).toBe('3,') // not right after a separator
    // Full keystroke sequence for one side: 6-3/4"
    let buf = ''
    for (const key of ['6', '-', '3', '/', '4', '"']) buf = editDimsBuffer(buf, key)
    expect(buf).toBe('6-3/4"')
  })

  it('accepts unit-suffix letters so each side can carry an explicit unit', () => {
    expect(editDimsBuffer('1', 'c')).toBe('1c')
    expect(editDimsBuffer('1c', 'm')).toBe('1cm')
    expect(editDimsBuffer('1cm', ',')).toBe('1cm,')
    expect(editDimsBuffer('1cm,100m', 'm')).toBe('1cm,100mm')
    expect(editDimsBuffer('5', 'f')).toBe('5f')
    expect(editDimsBuffer('5f', 't')).toBe('5ft')
  })

  it('accepts feet/inch marks and fraction slash', () => {
    expect(editDimsBuffer('5', "'")).toBe("5'")
    expect(editDimsBuffer('23', '"')).toBe('23"')
    expect(editDimsBuffer('5/8', '"')).toBe('5/8"')
    expect(editDimsBuffer('1', '/')).toBe('1/')
  })

  it('still treats x/X as the separator, not a unit letter', () => {
    expect(editDimsBuffer('3cm', 'x')).toBe('3cmx')
    expect(editDimsBuffer('3cmx4', 'x')).toBe('3cmx4') // second separator rejected
  })

  it('Backspace removes the last character', () => {
    expect(editDimsBuffer('3,4', 'Backspace')).toBe('3,')
    expect(editDimsBuffer('3', 'Backspace')).toBe('')
    expect(editDimsBuffer('', 'Backspace')).toBe('')
  })

  it('ignores unknown keys', () => {
    expect(editDimsBuffer('3', 'a')).toBe('3')
    expect(editDimsBuffer('3', 'Enter')).toBe('3')
  })
})

describe('parseDimensions', () => {
  it('parses a single value as a square', () => {
    expect(parseDimensions('3')).toEqual([3, 3])
    expect(parseDimensions('2.5')).toEqual([2.5, 2.5])
  })

  it('parses comma-separated values', () => {
    expect(parseDimensions('3,4')).toEqual([3, 4])
  })

  it('parses x-separated values', () => {
    expect(parseDimensions('3x4')).toEqual([3, 4])
    expect(parseDimensions('3X4')).toEqual([3, 4])
  })

  it('parses space-separated values with surrounding spaces', () => {
    expect(parseDimensions('3 x 4')).toEqual([3, 4])
    expect(parseDimensions('3 4')).toEqual([3, 4])
    expect(parseDimensions(' 3 , 4 ')).toEqual([3, 4])
  })

  it('returns null for empty/malformed input', () => {
    expect(parseDimensions('')).toBeNull()
    expect(parseDimensions('   ')).toBeNull()
    expect(parseDimensions(',')).toBeNull()
    expect(parseDimensions('x')).toBeNull()
    expect(parseDimensions('3,')).toBeNull()
    expect(parseDimensions('3,4,5')).toBeNull()
  })

  it('returns null for non-positive or non-finite sides', () => {
    expect(parseDimensions('0')).toBeNull()
    expect(parseDimensions('-3')).toBeNull()
    expect(parseDimensions('3,0')).toBeNull()
    expect(parseDimensions('3,-4')).toBeNull()
    expect(parseDimensions('abc')).toBeNull()
    expect(parseDimensions('3,abc')).toBeNull()
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
