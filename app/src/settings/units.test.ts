import { describe, it, expect } from 'vitest'
import {
  formatLengthIn,
  metersFromUnit,
  getLengthUnitSuffix,
  parseLengthToMeters,
  parseDimensionsToMeters,
  typedReadout,
  LENGTH_SYSTEM_OF,
  LENGTH_FORMATS_BY_SYSTEM,
  DEFAULT_FORMAT_FOR_SYSTEM,
} from './units'

/** Every display format — explicit units must parse identically in all of them. */
const ALL_FORMATS = ['m', 'cm', 'mm', 'arch', 'frac_in', 'dec_in'] as const

describe('formatLengthIn — metric (unchanged goldens)', () => {
  it('formats meters with trimmed precision', () => {
    expect(formatLengthIn(1.5, 'm')).toBe('1.5 m')
    expect(formatLengthIn(2, 'm')).toBe('2 m')
    expect(formatLengthIn(0, 'm')).toBe('0 m')
  })

  it('converts meters to centimeters', () => {
    expect(formatLengthIn(1.5, 'cm')).toBe('150 cm')
  })

  it('converts meters to millimeters', () => {
    expect(formatLengthIn(1.5, 'mm')).toBe('1500 mm')
  })

  it('trims trailing zeros without leaving a dangling decimal point', () => {
    expect(formatLengthIn(1, 'm')).toBe('1 m')
    expect(formatLengthIn(0.5, 'm')).toBe('0.5 m')
  })

  it('round-trips a whole-number conversion cleanly', () => {
    // 0.0254 m == exactly 1 in -> exercised via dec_in below; here just
    // confirm metric round-trips still hold.
    expect(formatLengthIn(0.01, 'cm')).toBe('1 cm')
    expect(formatLengthIn(0.001, 'mm')).toBe('1 mm')
  })
})

describe('formatLengthIn — dec_in', () => {
  it('formats decimal inches with trimmed precision', () => {
    expect(formatLengthIn(60.125 * 0.0254, 'dec_in')).toBe('60.125"')
  })

  it('formats zero', () => {
    expect(formatLengthIn(0, 'dec_in')).toBe('0"')
  })

  it('formats negative values with a leading minus', () => {
    expect(formatLengthIn(-1 * 0.0254, 'dec_in')).toBe('-1"')
  })

  it('trims trailing zeros', () => {
    expect(formatLengthIn(1 * 0.0254, 'dec_in')).toBe('1"')
    expect(formatLengthIn(0.5 * 0.0254, 'dec_in')).toBe('0.5"')
  })
})

describe('formatLengthIn — frac_in', () => {
  it('formats a whole + fraction', () => {
    expect(formatLengthIn(60.125 * 0.0254, 'frac_in')).toBe('60-1/8"')
  })

  it('formats a pure fraction with whole omitted', () => {
    expect(formatLengthIn(0.5 * 0.0254, 'frac_in')).toBe('1/2"')
  })

  it('formats a whole number with no fraction', () => {
    expect(formatLengthIn(60 * 0.0254, 'frac_in')).toBe('60"')
  })

  it('formats zero', () => {
    expect(formatLengthIn(0, 'frac_in')).toBe('0"')
  })

  it('formats negative values with a leading minus', () => {
    expect(formatLengthIn(-60.125 * 0.0254, 'frac_in')).toBe('-60-1/8"')
  })

  it('reduces a fraction to lowest terms', () => {
    // 60 + 8/16 -> 60-1/2 (not 60-8/16)
    expect(formatLengthIn(60.5 * 0.0254, 'frac_in')).toBe('60-1/2"')
  })

  it('carries the fraction up to the next whole inch when rounding to 1/16', () => {
    // 11.97" rounds to 1/16 -> 191.5/16 -> rounds to 192/16 = 12" exactly.
    expect(formatLengthIn(11.97 * 0.0254, 'frac_in')).toBe('12"')
  })
})

describe('formatLengthIn — arch', () => {
  it('formats feet + inches + fraction', () => {
    expect(formatLengthIn(63.125 * 0.0254, 'arch')).toBe('5\' 3-1/8"')
  })

  it('omits the feet part when 0 feet', () => {
    expect(formatLengthIn(3.125 * 0.0254, 'arch')).toBe('3-1/8"')
  })

  it('shows just feet when whole feet with 0 inches', () => {
    expect(formatLengthIn(5 * 12 * 0.0254, 'arch')).toBe("5'")
  })

  it('formats zero as 0"', () => {
    expect(formatLengthIn(0, 'arch')).toBe('0"')
  })

  it('carries inches up to a foot (11.97" -> 1\')', () => {
    expect(formatLengthIn(11.97 * 0.0254, 'arch')).toBe("1'")
  })

  it('carries inches up to a foot when already past whole feet (e.g. 4\' 11.97" -> 5\')', () => {
    expect(formatLengthIn((4 * 12 + 11.97) * 0.0254, 'arch')).toBe("5'")
  })

  it('formats negative values with a leading minus', () => {
    expect(formatLengthIn(-63.125 * 0.0254, 'arch')).toBe('-5\' 3-1/8"')
  })

  it('reduces the fraction to lowest terms', () => {
    expect(formatLengthIn((5 * 12 + 3.5) * 0.0254, 'arch')).toBe('5\' 3-1/2"')
  })
})

describe('metersFromUnit', () => {
  it('converts centimeters to meters', () => {
    expect(metersFromUnit(30, 'cm')).toBeCloseTo(0.3, 10)
  })

  it('converts meters to meters (identity)', () => {
    expect(metersFromUnit(1.5, 'm')).toBeCloseTo(1.5, 10)
  })

  it('converts millimeters to meters', () => {
    expect(metersFromUnit(1000, 'mm')).toBeCloseTo(1, 10)
  })

  it('treats a bare value in an imperial format as inches (exact 0.0254 m/in)', () => {
    expect(metersFromUnit(1, 'dec_in')).toBeCloseTo(0.0254, 12)
    expect(metersFromUnit(2, 'frac_in')).toBeCloseTo(0.0508, 12)
    expect(metersFromUnit(12, 'arch')).toBeCloseTo(0.3048, 12)
  })
})

describe('getLengthUnitSuffix', () => {
  it('returns the suffix for each metric/decimal/fractional format', () => {
    expect(getLengthUnitSuffix('m')).toBe('m')
    expect(getLengthUnitSuffix('cm')).toBe('cm')
    expect(getLengthUnitSuffix('mm')).toBe('mm')
    expect(getLengthUnitSuffix('dec_in')).toBe('"')
    expect(getLengthUnitSuffix('frac_in')).toBe('"')
  })

  it('returns an empty suffix for arch (composite feet+inches)', () => {
    expect(getLengthUnitSuffix('arch')).toBe('')
  })
})

describe('parseLengthToMeters — metric', () => {
  it('parses a bare decimal in the active metric unit', () => {
    expect(parseLengthToMeters('1.5', 'm')).toBeCloseTo(1.5, 10)
    expect(parseLengthToMeters('150', 'cm')).toBeCloseTo(1.5, 10)
    expect(parseLengthToMeters('1500', 'mm')).toBeCloseTo(1.5, 10)
  })

  it('parses a bare fraction in the active metric unit (regression: "3 1/2" in meters mode)', () => {
    // Previously the buffer/readout accepted "3 1/2" in metric modes but the
    // parse returned null, so Enter silently no-oped.
    expect(parseLengthToMeters('3 1/2', 'm')).toBeCloseTo(3.5, 10)
    expect(parseLengthToMeters('1/2', 'm')).toBeCloseTo(0.5, 10)
    expect(parseLengthToMeters('3-1/2', 'cm')).toBeCloseTo(0.035, 12)
    expect(parseLengthToMeters('6-3/4', 'mm')).toBeCloseTo(0.00675, 12)
    expect(parseLengthToMeters('-3 1/2', 'm')).toBeCloseTo(-3.5, 10)
  })

  it('returns null for an incomplete fraction in metric modes', () => {
    expect(parseLengthToMeters('3 1/', 'm')).toBeNull()
    expect(parseLengthToMeters('1/', 'cm')).toBeNull()
    expect(parseLengthToMeters('1/0', 'm')).toBeNull() // zero denominator
  })

  it('returns null for empty input', () => {
    expect(parseLengthToMeters('', 'm')).toBeNull()
    expect(parseLengthToMeters('   ', 'cm')).toBeNull()
  })

  it('returns null for invalid input', () => {
    expect(parseLengthToMeters('abc', 'm')).toBeNull()
  })
})

describe('parseLengthToMeters — imperial', () => {
  it('parses a bare number as inches', () => {
    expect(parseLengthToMeters('60', 'dec_in')).toBeCloseTo(60 * 0.0254, 12)
    expect(parseLengthToMeters('60.125', 'frac_in')).toBeCloseTo(60.125 * 0.0254, 12)
    expect(parseLengthToMeters('60', 'arch')).toBeCloseTo(60 * 0.0254, 12)
  })

  it('parses feet only', () => {
    expect(parseLengthToMeters("5'", 'arch')).toBeCloseTo(5 * 0.3048, 12)
  })

  it('parses inches only', () => {
    expect(parseLengthToMeters('3"', 'arch')).toBeCloseTo(3 * 0.0254, 12)
  })

  it('parses feet + inches, no space', () => {
    expect(parseLengthToMeters('5\'3"', 'arch')).toBeCloseTo((5 * 12 + 3) * 0.0254, 12)
  })

  it('parses feet + inches, with a space', () => {
    expect(parseLengthToMeters('5\' 3"', 'arch')).toBeCloseTo((5 * 12 + 3) * 0.0254, 12)
  })

  it('parses feet + inches + fraction with a hyphen', () => {
    expect(parseLengthToMeters('5\' 3-1/2"', 'arch')).toBeCloseTo((5 * 12 + 3.5) * 0.0254, 12)
  })

  it('parses feet + inches + fraction with a space', () => {
    expect(parseLengthToMeters('5\' 3 1/2"', 'arch')).toBeCloseTo((5 * 12 + 3.5) * 0.0254, 12)
  })

  it('parses inches + fraction with a space', () => {
    expect(parseLengthToMeters('3 1/2"', 'arch')).toBeCloseTo(3.5 * 0.0254, 12)
  })

  it('parses inches + fraction with a hyphen', () => {
    expect(parseLengthToMeters('3-1/2"', 'arch')).toBeCloseTo(3.5 * 0.0254, 12)
  })

  it('parses a fraction alone', () => {
    expect(parseLengthToMeters('1/2"', 'arch')).toBeCloseTo(0.5 * 0.0254, 12)
  })

  it('accepts the feet-inch-fraction grammar regardless of active imperial format', () => {
    for (const format of ['arch', 'frac_in', 'dec_in'] as const) {
      expect(parseLengthToMeters('5\' 3-1/2"', format)).toBeCloseTo((5 * 12 + 3.5) * 0.0254, 12)
    }
  })

  it('round-trips exact inch/foot constants', () => {
    expect(parseLengthToMeters('1"', 'arch')).toBeCloseTo(0.0254, 12)
    expect(parseLengthToMeters("1'", 'arch')).toBeCloseTo(0.3048, 12)
  })

  it('returns null for empty input', () => {
    expect(parseLengthToMeters('', 'arch')).toBeNull()
  })

  it('returns null for garbage input', () => {
    expect(parseLengthToMeters('abc', 'arch')).toBeNull()
  })
})

describe('parseLengthToMeters — sign handling (leading minus negates the WHOLE value)', () => {
  it('regression: "-1/2\"" is -0.5", not +0.5"', () => {
    // The old grammar consumed the leading '-' as the feet-inch separator,
    // silently dropping the sign.
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('-1/2"', format)).toBeCloseTo(-0.5 * 0.0254, 12)
    }
  })

  it('regression: "-5\'6\"" is -66", not -54"', () => {
    // The old grammar applied the minus to the feet only (-60" + 6").
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters("-5'6\"", format)).toBeCloseTo(-66 * 0.0254, 12)
    }
  })

  it('regression: a dangling fraction hyphen ("24-") is incomplete input, not 24', () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('24-', format)).toBeNull()
    }
  })

  it('keeps working negative forms working', () => {
    expect(parseLengthToMeters('-24', 'arch')).toBeCloseTo(-24 * 0.0254, 12)
    expect(parseLengthToMeters('-3.5', 'dec_in')).toBeCloseTo(-3.5 * 0.0254, 12)
    expect(parseLengthToMeters('-3 1/2', 'arch')).toBeCloseTo(-3.5 * 0.0254, 12)
    expect(parseLengthToMeters('-6-3/4"', 'arch')).toBeCloseTo(-6.75 * 0.0254, 12)
    expect(parseLengthToMeters("-5'", 'arch')).toBeCloseTo(-5 * 0.3048, 12)
    expect(parseLengthToMeters('-3"', 'm')).toBeCloseTo(-3 * 0.0254, 12)
    expect(parseLengthToMeters("-5' 2-1/4\"", 'arch')).toBeCloseTo(-(5 * 12 + 2.25) * 0.0254, 12)
  })

  it('rejects interior, doubled, or dangling signs', () => {
    expect(parseLengthToMeters("5'-6\"", 'arch')).toBeNull() // interior minus
    expect(parseLengthToMeters('--24', 'arch')).toBeNull()   // doubled sign
    expect(parseLengthToMeters('3--1/2"', 'arch')).toBeNull() // doubled fraction hyphen
    expect(parseLengthToMeters('24-"', 'arch')).toBeNull()   // dangling hyphen before mark
    expect(parseLengthToMeters('-', 'arch')).toBeNull()
    expect(parseLengthToMeters('-', 'm')).toBeNull()
  })

  it('parses mark-less imperial fractions with the sign applied to the whole value', () => {
    expect(parseLengthToMeters('3 1/2', 'arch')).toBeCloseTo(3.5 * 0.0254, 12)
    expect(parseLengthToMeters('-3 1/2', 'frac_in')).toBeCloseTo(-3.5 * 0.0254, 12)
  })
})

describe('parseLengthToMeters — explicit metric suffixes work in ANY mode', () => {
  it('parses cm/mm/m/km suffixes regardless of the active format', () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('1cm', format)).toBeCloseTo(0.01, 12)
      expect(parseLengthToMeters('100mm', format)).toBeCloseTo(0.1, 12)
      expect(parseLengthToMeters('2.5m', format)).toBeCloseTo(2.5, 12)
      expect(parseLengthToMeters('3km', format)).toBeCloseTo(3000, 9)
    }
  })

  it('accepts a space between the number and the suffix', () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('1 cm', format)).toBeCloseTo(0.01, 12)
      expect(parseLengthToMeters('100 mm', format)).toBeCloseTo(0.1, 12)
    }
  })

  it('is case-insensitive', () => {
    expect(parseLengthToMeters('1CM', 'arch')).toBeCloseTo(0.01, 12)
    expect(parseLengthToMeters('2.5M', 'dec_in')).toBeCloseTo(2.5, 12)
    expect(parseLengthToMeters('100Mm', 'm')).toBeCloseTo(0.1, 12)
  })

  it('preserves the sign convention (leading minus negates)', () => {
    expect(parseLengthToMeters('-1cm', 'm')).toBeCloseTo(-0.01, 12)
    expect(parseLengthToMeters('-2.5m', 'arch')).toBeCloseTo(-2.5, 12)
  })

  it('a bare number still follows the display format', () => {
    expect(parseLengthToMeters('2', 'mm')).toBeCloseTo(0.002, 12)
    expect(parseLengthToMeters('2', 'cm')).toBeCloseTo(0.02, 12)
    expect(parseLengthToMeters('2', 'dec_in')).toBeCloseTo(2 * 0.0254, 12)
  })

  it('returns null for a suffix with no number or an unknown suffix', () => {
    expect(parseLengthToMeters('cm', 'm')).toBeNull()
    expect(parseLengthToMeters('1zm', 'm')).toBeNull()
  })

  it('does not support summed quantities like "1m 20cm"', () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('1m 20cm', format)).toBeNull()
    }
  })
})

describe('parseLengthToMeters — ft/in word suffixes work in ANY mode', () => {
  it('parses "ft" and "in" suffixes regardless of the active format', () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('5ft', format)).toBeCloseTo(5 * 0.3048, 12)
      expect(parseLengthToMeters('6in', format)).toBeCloseTo(6 * 0.0254, 12)
      expect(parseLengthToMeters('5 ft', format)).toBeCloseTo(5 * 0.3048, 12)
      expect(parseLengthToMeters('6 IN', format)).toBeCloseTo(6 * 0.0254, 12)
    }
  })
})

describe('parseLengthToMeters — explicit feet/inch marks work in ANY mode', () => {
  it("parses 5' (feet) in every format, including metric", () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters("5'", format)).toBeCloseTo(5 * 0.3048, 12)
    }
  })

  it('parses 23" (inches) in every format, including metric', () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('23"', format)).toBeCloseTo(23 * 0.0254, 12)
    }
  })

  it("parses 5'6\" (feet + inches) in every format", () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('5\'6"', format)).toBeCloseTo((5 * 12 + 6) * 0.0254, 12)
    }
  })

  it('parses fractional inches 5/8" in every format', () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('5/8"', format)).toBeCloseTo(0.625 * 0.0254, 12)
    }
  })

  it('parses 2-1/4" in every format', () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('2-1/4"', format)).toBeCloseTo(2.25 * 0.0254, 12)
    }
  })

  it("parses 5' 2-1/4\" in every format", () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('5\' 2-1/4"', format)).toBeCloseTo((5 * 12 + 2.25) * 0.0254, 12)
    }
  })

  it("parses 5' 5/8\" in every format", () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('5\' 5/8"', format)).toBeCloseTo((5 * 12 + 0.625) * 0.0254, 12)
    }
  })

  it('still rejects garbage carrying a mark', () => {
    expect(parseLengthToMeters('abc"', 'm')).toBeNull()
    expect(parseLengthToMeters("abc'", 'arch')).toBeNull()
    expect(parseLengthToMeters('"', 'm')).toBeNull()
  })
})

describe('parseDimensionsToMeters', () => {
  it('parses mixed explicit-unit components — "1cm,100mm" — in ANY mode', () => {
    for (const format of ALL_FORMATS) {
      const dims = parseDimensionsToMeters('1cm,100mm', format)
      expect(dims).not.toBeNull()
      expect(dims![0]).toBeCloseTo(0.01, 12)
      expect(dims![1]).toBeCloseTo(0.1, 12)
    }
  })

  it('parses mixed metric/imperial components', () => {
    const dims = parseDimensionsToMeters("5',23\"", 'm')
    expect(dims).not.toBeNull()
    expect(dims![0]).toBeCloseTo(5 * 0.3048, 12)
    expect(dims![1]).toBeCloseTo(23 * 0.0254, 12)
  })

  it('interprets bare components in the display format', () => {
    expect(parseDimensionsToMeters('3,4', 'm')).toEqual([3, 4])
    const cm = parseDimensionsToMeters('3,4', 'cm')
    expect(cm![0]).toBeCloseTo(0.03, 12)
    expect(cm![1]).toBeCloseTo(0.04, 12)
    const inches = parseDimensionsToMeters('3,4', 'dec_in')
    expect(inches![0]).toBeCloseTo(3 * 0.0254, 12)
    expect(inches![1]).toBeCloseTo(4 * 0.0254, 12)
  })

  it('accepts x/X separators and surrounding spaces', () => {
    expect(parseDimensionsToMeters('3x4', 'm')).toEqual([3, 4])
    expect(parseDimensionsToMeters('3X4', 'm')).toEqual([3, 4])
    expect(parseDimensionsToMeters('3 x 4', 'm')).toEqual([3, 4])
    expect(parseDimensionsToMeters(' 3 , 4 ', 'm')).toEqual([3, 4])
  })

  it('keeps the legacy space-separated pair', () => {
    expect(parseDimensionsToMeters('3 4', 'm')).toEqual([3, 4])
    const mixed = parseDimensionsToMeters('1cm 2cm', 'arch')
    expect(mixed![0]).toBeCloseTo(0.01, 12)
    expect(mixed![1]).toBeCloseTo(0.02, 12)
  })

  it('accepts a comma/x after a length that itself contains a space (regression: "5\' 3\"" then ",")', () => {
    // The typed space inside "5' 3\"" must never consume the dims-separator
    // slot — a later `,`/`x` starts the second dimension.
    const dims = parseDimensionsToMeters("5' 3\",2'", 'm')
    expect(dims).not.toBeNull()
    expect(dims![0]).toBeCloseTo((5 * 12 + 3) * 0.0254, 12)
    expect(dims![1]).toBeCloseTo(2 * 0.3048, 12)
    const x = parseDimensionsToMeters("5' 3\" x 2'", 'arch')
    expect(x![0]).toBeCloseTo((5 * 12 + 3) * 0.0254, 12)
    expect(x![1]).toBeCloseTo(2 * 0.3048, 12)
  })

  it('treats a single component as a square', () => {
    expect(parseDimensionsToMeters('3', 'm')).toEqual([3, 3])
    const cm = parseDimensionsToMeters('1cm', 'arch')
    expect(cm![0]).toBeCloseTo(0.01, 12)
    expect(cm![1]).toBeCloseTo(0.01, 12)
    const archSquare = parseDimensionsToMeters("5' 3\"", 'm')
    expect(archSquare![0]).toBeCloseTo((5 * 12 + 3) * 0.0254, 12)
    expect(archSquare![1]).toBeCloseTo((5 * 12 + 3) * 0.0254, 12)
  })

  it('returns null for empty/malformed input', () => {
    expect(parseDimensionsToMeters('', 'm')).toBeNull()
    expect(parseDimensionsToMeters('   ', 'm')).toBeNull()
    expect(parseDimensionsToMeters(',', 'm')).toBeNull()
    expect(parseDimensionsToMeters('3,', 'm')).toBeNull()
    expect(parseDimensionsToMeters(',4', 'm')).toBeNull()
    expect(parseDimensionsToMeters('3,4,5', 'm')).toBeNull()
    expect(parseDimensionsToMeters('abc', 'm')).toBeNull()
    expect(parseDimensionsToMeters('3,abc', 'm')).toBeNull()
    expect(parseDimensionsToMeters('24-', 'arch')).toBeNull() // dangling fraction hyphen
    expect(parseDimensionsToMeters('3,24-', 'arch')).toBeNull()
  })

  it('returns null for non-positive components', () => {
    expect(parseDimensionsToMeters('0', 'm')).toBeNull()
    expect(parseDimensionsToMeters('-3', 'm')).toBeNull()
    expect(parseDimensionsToMeters('3,0', 'm')).toBeNull()
    expect(parseDimensionsToMeters('3,-4', 'm')).toBeNull()
    expect(parseDimensionsToMeters('-1cm,2cm', 'm')).toBeNull()
  })
})

describe('LengthFormat <-> LengthSystem grouping', () => {
  it('groups metric formats', () => {
    expect(LENGTH_SYSTEM_OF.m).toBe('metric')
    expect(LENGTH_SYSTEM_OF.cm).toBe('metric')
    expect(LENGTH_SYSTEM_OF.mm).toBe('metric')
  })

  it('groups imperial formats', () => {
    expect(LENGTH_SYSTEM_OF.arch).toBe('imperial')
    expect(LENGTH_SYSTEM_OF.frac_in).toBe('imperial')
    expect(LENGTH_SYSTEM_OF.dec_in).toBe('imperial')
  })

  it('lists each system\'s formats', () => {
    expect(LENGTH_FORMATS_BY_SYSTEM.metric).toEqual(['m', 'cm', 'mm'])
    expect(LENGTH_FORMATS_BY_SYSTEM.imperial).toEqual(['arch', 'frac_in', 'dec_in'])
  })

  it('has a default format per system', () => {
    expect(DEFAULT_FORMAT_FOR_SYSTEM.metric).toBe('m')
    expect(DEFAULT_FORMAT_FOR_SYSTEM.imperial).toBe('arch')
  })
})

describe('typedReadout', () => {
  it('appends the display suffix while the buffer is a bare number', () => {
    expect(typedReadout('5', 'm')).toBe('5 m')
    expect(typedReadout('3.5', 'cm')).toBe('3.5 cm')
    expect(typedReadout('12', 'mm')).toBe('12 mm')
    expect(typedReadout('3', 'frac_in')).toBe('3 "')
  })

  it('drops the suffix the instant the buffer carries an explicit unit', () => {
    expect(typedReadout("5'", 'm')).toBe("5'")
    expect(typedReadout('60"', 'm')).toBe('60"')
    expect(typedReadout('10c', 'm')).toBe('10c') // mid-typing "10cm"
    expect(typedReadout('10cm', 'm')).toBe('10cm')
    expect(typedReadout('100mm', 'cm')).toBe('100mm')
    expect(typedReadout('5ft', 'mm')).toBe('5ft')
    expect(typedReadout("5' 6-3/4\"", 'm')).toBe("5' 6-3/4\"")
    expect(typedReadout("5'", 'frac_in')).toBe("5'") // imperial mode too
  })

  it('keeps the suffix for a bare fraction (still display units)', () => {
    expect(typedReadout('3 1/2', 'frac_in')).toBe('3 1/2 "')
    // Metric modes too — and the parse now agrees (see the metric fraction
    // tests): what the readout shows is what Enter commits.
    expect(typedReadout('3 1/2', 'm')).toBe('3 1/2 m')
    expect(typedReadout('1/2', 'cm')).toBe('1/2 cm')
  })

  it('appends no suffix while the component does not parse as a bare number', () => {
    // An unparseable buffer must not be dressed up to look committable.
    expect(typedReadout('3 1/', 'm')).toBe('3 1/')
    expect(typedReadout('24-', 'm')).toBe('24-')       // dangling fraction hyphen
    expect(typedReadout('24-', 'frac_in')).toBe('24-')
    expect(typedReadout('3 1', 'm')).toBe('3 1')       // fraction not yet begun
  })

  it('re-applies the rule per component in a dims buffer', () => {
    expect(typedReadout('10cm,5', 'm')).toBe('10cm,5 m')   // second side bare again
    expect(typedReadout('10cm,5mm', 'm')).toBe('10cm,5mm') // …until it gets a unit
    expect(typedReadout('3,4', 'cm')).toBe('3,4 cm')
    expect(typedReadout('3x4', 'm')).toBe('3x4 m')
    expect(typedReadout("5',23\"", 'm')).toBe("5',23\"")
  })

  it('appends nothing after a dangling separator or on an empty/sign-only buffer', () => {
    expect(typedReadout('', 'm')).toBe('')
    expect(typedReadout('-', 'm')).toBe('-')
    expect(typedReadout('10cm,', 'm')).toBe('10cm,')
    expect(typedReadout('3x', 'm')).toBe('3x')
  })

  it('never appends for the composite arch format (no single suffix)', () => {
    expect(typedReadout('5', 'arch')).toBe('5')
  })
})
