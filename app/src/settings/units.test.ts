import { describe, it, expect } from 'vitest'
import { formatLengthIn } from './units'

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
