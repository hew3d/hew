import { afterEach, describe, it, expect, vi } from 'vitest'
import {
  formatLengthIn,
  formatAreaIn,
  formatVolumeIn,
  metersFromUnit,
  getLengthUnitSuffix,
  parseLengthToMeters,
  parseDimensionsToMeters,
  typedReadout,
  LENGTH_SYSTEM_OF,
  LENGTH_FORMATS_BY_SYSTEM,
  DEFAULT_FORMAT_FOR_SYSTEM,
  homeFramingScale,
  getPrecision,
  setPrecision,
  DEFAULT_PRECISION,
  precisionLabel,
  PRECISION_OPTIONS,
  type Precision,
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
    // (Exact is the default now, so the coarse grid is asked for by name.)
    expect(formatLengthIn(11.97 * 0.0254, 'frac_in', 16)).toBe('~12"')
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
    expect(formatLengthIn(11.97 * 0.0254, 'arch', 16)).toBe("~1'")
  })

  it('carries inches up to a foot when already past whole feet (e.g. 4\' 11.97" -> 5\')', () => {
    expect(formatLengthIn((4 * 12 + 11.97) * 0.0254, 'arch', 16)).toBe("~5'")
  })

  it('formats negative values with a leading minus', () => {
    expect(formatLengthIn(-63.125 * 0.0254, 'arch')).toBe('-5\' 3-1/8"')
  })

  it('reduces the fraction to lowest terms', () => {
    expect(formatLengthIn((5 * 12 + 3.5) * 0.0254, 'arch')).toBe('5\' 3-1/2"')
  })

  it('drops the leading "0-" for a standalone sub-inch value (3/4" not 0-3/4")', () => {
    expect(formatLengthIn(0.75 * 0.0254, 'arch')).toBe('3/4"')
  })

  it('drops the leading "0-" for a standalone sub-inch value with a negative sign', () => {
    expect(formatLengthIn(-0.75 * 0.0254, 'arch')).toBe('-3/4"')
  })

  it('keeps the "0-" inside a composite WITH feet (1\' 0-3/4", not 1\' 3/4")', () => {
    // 12.75" = 1' + 0.75" — the inch part is a sub-inch fraction, but it's
    // paired with a nonzero feet part, so the "0" stays: it's the whole
    // inches count (zero), not a dropped leading digit.
    expect(formatLengthIn(12.75 * 0.0254, 'arch')).toBe('1\' 0-3/4"')
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

describe('parseLengthToMeters — leading-dot decimals ("0.75" without the leading zero)', () => {
  it('parses a bare leading-dot decimal in every metric format', () => {
    expect(parseLengthToMeters('.75', 'm')).toBeCloseTo(0.75, 12)
    expect(parseLengthToMeters('.75', 'cm')).toBeCloseTo(0.0075, 12)
    expect(parseLengthToMeters('.75', 'mm')).toBeCloseTo(0.00075, 12)
  })

  it('parses a bare leading-dot decimal in every imperial format, as inches', () => {
    for (const format of ['arch', 'frac_in', 'dec_in'] as const) {
      expect(parseLengthToMeters('.75', format)).toBeCloseTo(0.75 * 0.0254, 12)
    }
  })

  it('parses a negative leading-dot decimal', () => {
    expect(parseLengthToMeters('-.5', 'm')).toBeCloseTo(-0.5, 12)
    expect(parseLengthToMeters('-.5', 'arch')).toBeCloseTo(-0.5 * 0.0254, 12)
  })

  it('parses a leading-dot decimal with an explicit inch mark', () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('.75"', format)).toBeCloseTo(0.75 * 0.0254, 12)
      expect(parseLengthToMeters('-.5"', format)).toBeCloseTo(-0.5 * 0.0254, 12)
    }
  })

  it('parses a leading-dot decimal with an explicit foot mark', () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters(".5'", format)).toBeCloseTo(0.5 * 0.3048, 12)
    }
  })

  it('parses a leading-dot decimal with an explicit metric suffix', () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('.5cm', format)).toBeCloseTo(0.005, 12)
      expect(parseLengthToMeters('-.5cm', format)).toBeCloseTo(-0.005, 12)
    }
  })

  it('matches exactly what "0.75"/"0.5" would parse to', () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('.75', format)).toBeCloseTo(
        parseLengthToMeters('0.75', format)!,
        12,
      )
      expect(parseLengthToMeters('.75"', format)).toBeCloseTo(
        parseLengthToMeters('0.75"', format)!,
        12,
      )
    }
  })

  it('still rejects a bare dot with no digits', () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('.', format)).toBeNull()
      expect(parseLengthToMeters('."', format)).toBeNull()
      expect(parseLengthToMeters("-.", format)).toBeNull()
    }
  })

  it('still rejects a doubled leading dot', () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('..5', format)).toBeNull()
      expect(parseLengthToMeters('..5"', format)).toBeNull()
    }
  })

  it('still rejects two decimal points in one number', () => {
    for (const format of ALL_FORMATS) {
      expect(parseLengthToMeters('.75.5', format)).toBeNull()
      expect(parseLengthToMeters('.75.5"', format)).toBeNull()
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

describe('homeFramingScale', () => {
  it('keeps the default framing for meter-scale formats', () => {
    expect(homeFramingScale('m')).toBe(1)
    expect(homeFramingScale('arch')).toBe(1)
  })

  it('frames closer for small-scale formats', () => {
    for (const f of ['cm', 'mm', 'frac_in', 'dec_in'] as const) {
      expect(homeFramingScale(f)).toBeLessThan(1)
      expect(homeFramingScale(f)).toBeGreaterThan(0)
    }
    // mm implies a smaller model than cm.
    expect(homeFramingScale('mm')).toBeLessThan(homeFramingScale('cm'))
  })
})

// ---------------------------------------------------------------------------
// Precision: Exact and coarse readouts
// ---------------------------------------------------------------------------

const IN = 0.0254

describe('formatLengthIn — Exact (the default)', () => {
  const cases: [string, number, (typeof ALL_FORMATS)[number], string][] = [
    ['1/3 m in m runs to the 6-decimal ceiling', 1 / 3, 'm', '0.333333 m'],
    ['1/3 m in cm', 1 / 3, 'cm', '33.3333 cm'],
    ['1/3 m in mm', 1 / 3, 'mm', '333.333 mm'],
    ['float noise is not shown', 0.1 + 0.2, 'm', '0.3 m'],
    ['whole value', 2, 'm', '2 m'],
    ['zero', 0, 'm', '0 m'],
    ['negative metric', -1 / 3, 'm', '-0.333333 m'],
    ['9.39876 mm keeps its needed digits', 0.00939876, 'mm', '9.399 mm'],
    ['sub-micrometer residue is dropped', 1.0000004, 'm', '1 m'],
    ['arch exact fraction', 63.125 * IN, 'arch', '5\' 3-1/8"'],
    ['arch 1/64 is exact', IN / 64, 'arch', '1/64"'],
    ['arch 1/32 inch', IN / 32, 'arch', '1/32"'],
    ['arch whole feet', 5 * 12 * IN, 'arch', "5'"],
    ['arch 11.97 in does not round up to a foot', 11.97 * IN, 'arch', '11.97"'],
    ['arch 1/3 inch falls back to decimal inches', IN / 3, 'arch', '0.3333"'],
    ['arch decimal fallback inside the composite', (12 + 2 + 1 / 3) * IN, 'arch', '1\' 2.3333"'],
    ['arch negative with fallback', -(12 + 2 + 1 / 3) * IN, 'arch', '-1\' 2.3333"'],
    ['frac_in exact', 60.125 * IN, 'frac_in', '60-1/8"'],
    ['frac_in 1/32', IN / 32, 'frac_in', '1/32"'],
    ['frac_in 1/3 falls back to decimal', IN / 3, 'frac_in', '0.3333"'],
    ['frac_in negative', -60.125 * IN, 'frac_in', '-60-1/8"'],
    ['dec_in', 60.125 * IN, 'dec_in', '60.125"'],
    ['dec_in 1/64', IN / 64, 'dec_in', '0.0156"'],
    ['dec_in 1/3 inch', IN / 3, 'dec_in', '0.3333"'],
    // 4 decimals can be 1.27 um off; Exact promises 1 um, so it takes a 5th.
    ['dec_in needing a 5th decimal', 0.12345 * IN, 'dec_in', '0.12345"'],
    ['arch needing a 5th decimal', 0.12345 * IN, 'arch', '0.12345"'],
  ]
  it.each(cases)('%s', (_n, meters, fmt, expected) => {
    expect(formatLengthIn(meters, fmt, 'exact')).toBe(expected)
  })

  it('never carries ~', () => {
    for (const fmt of ALL_FORMATS) {
      for (const x of [1 / 3, 0.00939876, 7.7777, IN / 3, -0.123456789]) {
        expect(formatLengthIn(x, fmt, 'exact').includes('~')).toBe(false)
      }
    }
  })
})

describe('formatLengthIn — coarse precision and ~', () => {
  const cases: [string, number, (typeof ALL_FORMATS)[number], Precision, string][] = [
    ['1/3 m at 2 decimals', 1 / 3, 'm', 2, '~0.33 m'],
    ['1/3 m at 0 decimals', 1 / 3, 'm', 0, '~0 m'],
    ['value already on the metric grid', 1.5, 'm', 2, '1.5 m'],
    ['mm at 1 decimal', 0.00939876, 'mm', 1, '~9.4 mm'],
    ['cm at 1 decimal on the grid', 0.125, 'cm', 1, '12.5 cm'],
    ['negative metric', -1 / 3, 'm', 2, '~-0.33 m'],
    ['rounds to zero without a minus sign', -0.0001, 'm', 2, '~0 m'],
    ['sub-micrometer change is not marked', 1.0000004, 'm', 0, '1 m'],
    ['1/32" at 1/16" (tie rounds up)', IN / 32, 'frac_in', 16, '~1/16"'],
    ['1/32" at 1/32" is on the grid', IN / 32, 'frac_in', 32, '1/32"'],
    ['3/16" is on the 1/16 grid', (3 / 16) * IN, 'frac_in', 16, '3/16"'],
    ['0.19" at 1/16"', 0.19 * IN, 'frac_in', 16, '~3/16"'],
    ['arch rounds with ~', (12 + 2 + 0.13) * IN, 'arch', 8, "~1' 2-1/8\""],
    ['arch on the grid', (12 + 2.125) * IN, 'arch', 8, "1' 2-1/8\""],
    ['arch negative', -(12 + 2.13) * IN, 'arch', 8, "~-1' 2-1/8\""],
    ['arch carries to the next foot', 11.97 * IN, 'arch', 16, "~1'"],
    ['arch at 1/2 inch', 3.3 * IN, 'arch', 2, '~3-1/2"'],
    ['dec_in at 2 decimals', 60.07123 * IN, 'dec_in', 2, '~60.07"'],
    ['dec_in at 1 decimal', 60.07 * IN, 'dec_in', 1, '~60.1"'],
    ['dec_in at 0 decimals', 60.6 * IN, 'dec_in', 0, '~61"'],
    ['dec_in on the grid', 60.125 * IN, 'dec_in', 3, '60.125"'],
    ['m at 6 decimals (a rounding under 1 um is not marked)', 1 / 3, 'm', 6, '0.333333 m'],
    ['frac_in at 1 inch', 60.6 * IN, 'frac_in', 1, '~61"'],
    ['frac_in at 1 foot rounds to the nearest 12 inches', 60.6 * IN, 'frac_in', 'foot', '~60"'],
    ['frac_in at 1 foot rounds up past the half', 66 * IN, 'frac_in', 'foot', '~72"'],
    ['frac_in at 1 foot on the grid', 60 * IN, 'frac_in', 'foot', '60"'],
    ['arch at 1 inch', (12 + 2 + 0.6) * IN, 'arch', 1, "~1' 3\""],
    ['arch at 1 foot', (12 * 5 + 7) * IN, 'arch', 'foot', "~6'"],
    ['arch at 1 foot rounding down', (12 * 5 + 5) * IN, 'arch', 'foot', "~5'"],
    ['arch at 1 foot below half a foot', 5 * IN, 'arch', 'foot', '~0"'],
    ['frac_in at 1/2', 60.3 * IN, 'frac_in', 2, '~60-1/2"'],
  ]
  it.each(cases)('%s', (_n, meters, fmt, p, expected) => {
    expect(formatLengthIn(meters, fmt, p)).toBe(expected)
  })
})

describe('Precision setting', () => {
  afterEach(() => {
    localStorage.removeItem('hew.settings.precision')
    for (const f of ALL_FORMATS) setPrecision(DEFAULT_PRECISION[f], f)
  })

  it('defaults each format to the rounding it always had', () => {
    expect(DEFAULT_PRECISION).toEqual({ m: 3, cm: 2, mm: 1, dec_in: 3, frac_in: 16, arch: 16 })
    for (const f of ALL_FORMATS) expect(getPrecision(f)).toBe(DEFAULT_PRECISION[f])
    expect(formatLengthIn(1 / 3, 'm')).toBe('~0.333 m')
    expect(formatLengthIn(1 / 3, 'cm')).toBe('~33.33 cm')
    expect(formatLengthIn(1 / 3, 'mm')).toBe('~333.3 mm')
    expect(formatLengthIn(IN / 3, 'dec_in')).toBe('~0.333"')
    expect(formatLengthIn(60.07 * IN, 'frac_in')).toBe('~60-1/16"')
  })

  it('remembers every format separately and persists under hew.settings.precision', () => {
    setPrecision(5, 'm')
    setPrecision(0, 'cm')
    setPrecision(32, 'arch')
    setPrecision('foot', 'frac_in')
    expect(getPrecision('m')).toBe(5)
    expect(getPrecision('cm')).toBe(0)
    expect(getPrecision('mm')).toBe(1)
    expect(getPrecision('arch')).toBe(32)
    expect(getPrecision('frac_in')).toBe('foot')
    expect(formatLengthIn(1 / 3, 'm')).toBe('~0.33333 m')
    expect(JSON.parse(localStorage.getItem('hew.settings.precision')!)).toEqual({
      m: 5,
      cm: 0,
      mm: 1,
      dec_in: 3,
      frac_in: 'foot',
      arch: 32,
    })
  })

  it('ignores a value the format does not offer', () => {
    setPrecision(16, 'm')
    expect(getPrecision('m')).toBe(3)
    setPrecision(3, 'arch')
    expect(getPrecision('arch')).toBe(16)
    setPrecision('foot', 'dec_in')
    expect(getPrecision('dec_in')).toBe(3)
    setPrecision(7, 'cm')
    expect(getPrecision('cm')).toBe(2)
  })

  it('offers Exact first, then the choices of the format', () => {
    for (const f of ['m', 'cm', 'mm', 'dec_in'] as const) {
      expect(PRECISION_OPTIONS[f]).toEqual(['exact', 0, 1, 2, 3, 4, 5, 6])
    }
    for (const f of ['frac_in', 'arch'] as const) {
      expect(PRECISION_OPTIONS[f]).toEqual(['exact', 'foot', 1, 2, 4, 8, 16, 32, 64])
    }
    expect(precisionLabel('m', 'exact')).toBe('Exact')
    expect(precisionLabel('m', 0)).toBe('Whole meters')
    expect(precisionLabel('dec_in', 2)).toBe('2 decimals')
    expect(precisionLabel('arch', 'foot')).toBe("Nearest 1'")
    expect(precisionLabel('arch', 1)).toBe('Nearest 1"')
    expect(precisionLabel('frac_in', 64)).toBe('Nearest 1/64"')
  })

  describe('migrating the older { metric, imperial } value', () => {
    // Re-import the module so its load-time read sees the stored value.
    async function loadWith(stored: string | null) {
      vi.resetModules()
      if (stored === null) localStorage.removeItem('hew.settings.precision')
      else localStorage.setItem('hew.settings.precision', stored)
      return await import('./units')
    }

    it('applies a metric number to m/cm/mm and an imperial denominator to frac_in/arch', async () => {
      const u = await loadWith(JSON.stringify({ metric: 2, imperial: 8 }))
      expect(u.getPrecision('m')).toBe(2)
      expect(u.getPrecision('cm')).toBe(2)
      expect(u.getPrecision('mm')).toBe(2)
      expect(u.getPrecision('frac_in')).toBe(8)
      expect(u.getPrecision('arch')).toBe(8)
      expect(u.getPrecision('dec_in')).toBe(3)
    })

    it('keeps exact for the formats it covered, and dec_in takes its default', async () => {
      const u = await loadWith(JSON.stringify({ metric: 'exact', imperial: 'exact' }))
      for (const f of ['m', 'cm', 'mm', 'frac_in', 'arch'] as const) expect(u.getPrecision(f)).toBe('exact')
      expect(u.getPrecision('dec_in')).toBe(3)
    })

    it('falls back to the defaults for bad values, and for each half on its own', async () => {
      const u = await loadWith(JSON.stringify({ metric: 99, imperial: 16 }))
      expect(u.getPrecision('m')).toBe(3)
      expect(u.getPrecision('mm')).toBe(1)
      expect(u.getPrecision('arch')).toBe(16)
      const v = await loadWith('{nope')
      expect(v.getPrecision('cm')).toBe(2)
      const w = await loadWith(JSON.stringify({ m: 'x', cm: 4, arch: 3, dec_in: 'foot' }))
      expect(w.getPrecision('m')).toBe(3)
      expect(w.getPrecision('cm')).toBe(4)
      expect(w.getPrecision('arch')).toBe(16)
      expect(w.getPrecision('dec_in')).toBe(3)
    })

    it('starts at the defaults with nothing stored', async () => {
      const u = await loadWith(null)
      expect(u.getPrecision('frac_in')).toBe(16)
    })
  })
})

describe('parse accepts a leading ~', () => {
  it.each(ALL_FORMATS)('strips ~ from a length in %s', (fmt) => {
    expect(parseLengthToMeters('~5cm', fmt)).toBeCloseTo(0.05, 12)
    expect(parseLengthToMeters('~ 5cm', fmt)).toBeCloseTo(0.05, 12)
    expect(parseLengthToMeters('~1\' 2-1/8"', fmt)).toBeCloseTo(14.125 * IN, 12)
    expect(parseLengthToMeters('~3/16"', fmt)).toBeCloseTo((3 / 16) * IN, 12)
    expect(parseLengthToMeters('~-0.33', fmt)).toBe(parseLengthToMeters('-0.33', fmt))
  })

  it('a bare ~ is not a length', () => {
    expect(parseLengthToMeters('~', 'm')).toBeNull()
  })

  it('strips ~ from dimensions, including per component', () => {
    expect(parseDimensionsToMeters('~2,~3', 'm')).toEqual([2, 3])
    expect(parseDimensionsToMeters('~2x3', 'm')).toEqual([2, 3])
  })

  it('round-trips every coarse readout', () => {
    for (const fmt of ALL_FORMATS) {
      const ps = PRECISION_OPTIONS[fmt]
      for (const p of ps) {
        const text = formatLengthIn(0.123456, fmt, p)
        expect(parseLengthToMeters(text, fmt)).not.toBeNull()
      }
    }
  })

  it('a "~" readout parses back to the rounded grid value, not the original', () => {
    const cases: [number, (typeof ALL_FORMATS)[number], Precision, string, number][] = [
      [0.123456, 'm', 2, '~0.12 m', 0.12],
      [0.123456, 'cm', 1, '~12.3 cm', 0.123],
      [0.123456, 'mm', 0, '~123 mm', 0.123],
      [(12 + 2 + 0.13) * IN, 'arch', 8, "~1' 2-1/8\"", 14.125 * IN],
      [4.86 * IN, 'frac_in', 8, '~4-7/8"', 4.875 * IN],
      [60.07123 * IN, 'dec_in', 2, '~60.07"', 60.07 * IN],
      // decimal inches inside the architectural composite
      [(12 + 2 + 0.07) * IN, 'arch', 16, "~1' 2-1/16\"", 14.0625 * IN],
    ]
    for (const [meters, fmt, p, text, grid] of cases) {
      expect(formatLengthIn(meters, fmt, p)).toBe(text)
      expect(parseLengthToMeters(text, fmt)).toBeCloseTo(grid, 12)
    }
  })

  it('an Exact decimal fallback inside arch parses back within a micrometer', () => {
    const meters = (12 + 2 + 1 / 3) * IN
    const text = formatLengthIn(meters, 'arch', 'exact')
    expect(text).toBe('1\' 2.3333"')
    expect(Math.abs(parseLengthToMeters(text, 'arch')! - meters)).toBeLessThanOrEqual(1e-6)
  })
})

describe('formatAreaIn / formatVolumeIn', () => {
  it('area_and_volume_switch_between_small_and_large_units', () => {
    // Metric: cm² below 1 m², else m²; cm³ below 1 m³, else m³.
    expect(formatAreaIn(0.5, 'm')).toBe('5000 cm\u00b2')
    expect(formatAreaIn(1.6, 'mm')).toBe('1.6 m\u00b2')
    expect(formatVolumeIn(0.0005, 'cm')).toBe('500 cm\u00b3')
    expect(formatVolumeIn(0.1, 'm')).toBe('100000 cm\u00b3')
    expect(formatVolumeIn(2, 'm')).toBe('2 m\u00b3')
    // Imperial: in² below 1 ft², else ft²; in³ below 1 ft³, else ft³.
    expect(formatAreaIn(0.0645160, 'dec_in')).toBe('100 in\u00b2')
    expect(formatAreaIn(0.3, 'arch')).toBe('~3.23 ft\u00b2')
    expect(formatVolumeIn(0.001, 'frac_in')).toBe('~61.02 in\u00b3')
    expect(formatVolumeIn(0.1, 'arch')).toBe('~3.53 ft\u00b3')
  })

  it('non_finite_input_is_a_dash', () => {
    for (const bad of [NaN, Infinity, -Infinity]) {
      expect(formatAreaIn(bad, 'm')).toBe('\u2014')
      expect(formatVolumeIn(bad, 'dec_in')).toBe('\u2014')
    }
  })

  it('precision_sets_the_decimals', () => {
    expect(formatAreaIn(0.123456, 'm', 4)).toBe('1234.56 cm\u00b2')
    expect(formatAreaIn(0.123456, 'm', 1)).toBe('~1234.6 cm\u00b2')
    expect(formatVolumeIn(2.34567, 'cm', 3)).toBe('~2.346 m\u00b3')
    expect(formatAreaIn(1.23456, 'dec_in', 3)).toBe('~13.289 ft\u00b2')
    expect(formatAreaIn(1.23456, 'm', 0)).toBe('~1 m\u00b2')
  })

  it('fraction_formats_still_show_decimals_for_area_and_volume', () => {
    expect(formatVolumeIn(0.001, 'frac_in', 16)).toBe('~61.02 in\u00b3')
    expect(formatAreaIn(0.3, 'arch', 'foot')).toBe('~3.23 ft\u00b2')
    expect(formatVolumeIn(2, 'm', 'exact')).toBe('2 m\u00b3')
    expect(formatVolumeIn(0.123456, 'm', 'exact')).toBe('123456 cm\u00b3')
    expect(formatVolumeIn(0.00123456, 'cm', 'exact')).toBe('1234.56 cm\u00b3')
  })

  it('a_rounded_value_carries_the_same_tilde_as_a_length', () => {
    expect(formatAreaIn(1.4, 'm', 0)).toBe('~1 m\u00b2')
    expect(formatAreaIn(0.00004, 'm', 0)).toBe('~0 cm\u00b2')
    expect(formatAreaIn(1.5, 'm', 2)).toBe('1.5 m\u00b2')
    expect(formatLengthIn(1.4, 'm', 0)).toBe('~1 m')
  })

  it('zero_reads_zero', () => {
    expect(formatAreaIn(0, 'm')).toBe('0 cm\u00b2')
    expect(formatVolumeIn(0, 'm')).toBe('0 cm\u00b3')
    expect(formatVolumeIn(0, 'arch')).toBe('0 in\u00b3')
    expect(formatAreaIn(0, 'dec_in')).toBe('0 in\u00b2')
  })

  it('rounding_at_the_boundary_promotes_to_the_larger_unit', () => {
    expect(formatAreaIn(0.99999999, 'm')).toBe('1 m\u00b2')
    expect(formatAreaIn(0.09999, 'm')).toBe('999.9 cm\u00b2')
  })
})
