import { describe, it, expect } from 'vitest'
import { formatLengthIn, metersFromUnit, getLengthUnitSuffix } from './units'

describe('formatLengthIn', () => {
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

  it('converts meters to feet', () => {
    // 1.5 m = 4.92126 ft -> trimmed to 2 decimals
    expect(formatLengthIn(1.5, 'ft')).toBe('4.92 ft')
  })

  it('converts meters to inches', () => {
    // 1.5 m = 59.0551 in -> trimmed to 2 decimals
    expect(formatLengthIn(1.5, 'in')).toBe('59.06 in')
  })

  it('trims trailing zeros without leaving a dangling decimal point', () => {
    expect(formatLengthIn(1, 'm')).toBe('1 m')
    expect(formatLengthIn(0.5, 'm')).toBe('0.5 m')
  })

  it('round-trips a whole-number conversion cleanly', () => {
    // 0.3048 m == exactly 1 ft
    expect(formatLengthIn(0.3048, 'ft')).toBe('1 ft')
    // 0.0254 m == exactly 1 in
    expect(formatLengthIn(0.0254, 'in')).toBe('1 in')
  })
})

describe('metersFromUnit', () => {
  it('converts centimeters to meters', () => {
    expect(metersFromUnit(30, 'cm')).toBeCloseTo(0.3, 10)
  })

  it('converts meters to meters (identity)', () => {
    expect(metersFromUnit(1.5, 'm')).toBeCloseTo(1.5, 10)
  })

  it('converts feet to meters', () => {
    expect(metersFromUnit(1, 'ft')).toBeCloseTo(0.3048, 10)
  })

  it('converts inches to meters', () => {
    expect(metersFromUnit(2, 'in')).toBeCloseTo(0.0508, 10)
  })

  it('converts millimeters to meters', () => {
    expect(metersFromUnit(1000, 'mm')).toBeCloseTo(1, 10)
  })
})

describe('getLengthUnitSuffix', () => {
  it('returns the suffix for each unit', () => {
    expect(getLengthUnitSuffix('m')).toBe('m')
    expect(getLengthUnitSuffix('cm')).toBe('cm')
    expect(getLengthUnitSuffix('mm')).toBe('mm')
    expect(getLengthUnitSuffix('ft')).toBe('ft')
    expect(getLengthUnitSuffix('in')).toBe('in')
  })
})
