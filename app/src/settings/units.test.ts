import { describe, it, expect } from 'vitest'
import {
  formatLengthIn,
  metersFromUnit,
  getLengthUnitSuffix,
  parseLengthToMeters,
  LENGTH_SYSTEM_OF,
  LENGTH_FORMATS_BY_SYSTEM,
  DEFAULT_FORMAT_FOR_SYSTEM,
} from './units'

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
