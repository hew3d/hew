import { describe, it, expect } from 'vitest'
import {
  axisColorForDirection,
  axisColorsForTheme,
  AXIS_COLORS,
  DARK_AXIS_COLORS,
  LIGHT_AXIS_COLORS,
} from './axisColors'

const TOL_2DEG = Math.cos((2 * Math.PI) / 180)

describe('axisColorForDirection', () => {
  it('matches +X exactly: red, axis 0, snapped to (1,0,0)', () => {
    const m = axisColorForDirection([1, 0, 0], TOL_2DEG)
    expect(m).not.toBeNull()
    expect(m!.color).toBe(AXIS_COLORS[0])
    expect(m!.axis).toBe(0)
    expect(m!.snapped).toEqual([1, 0, 0])
  })

  it('matches -Y: green, axis 1, snapped to (0,-1,0)', () => {
    const m = axisColorForDirection([0, -1, 0], TOL_2DEG)
    expect(m).not.toBeNull()
    expect(m!.color).toBe(AXIS_COLORS[1])
    expect(m!.axis).toBe(1)
    expect(m!.snapped[0]).toBeCloseTo(0)
    expect(m!.snapped[1]).toBeCloseTo(-1)
    expect(m!.snapped[2]).toBeCloseTo(0)
  })

  it('matches +Z: blue, axis 2', () => {
    const m = axisColorForDirection([0, 0, 1], TOL_2DEG)
    expect(m).not.toBeNull()
    expect(m!.color).toBe(AXIS_COLORS[2])
    expect(m!.axis).toBe(2)
  })

  it('returns null for a direction exactly 45° between two axes', () => {
    const d = Math.SQRT1_2
    const m = axisColorForDirection([d, d, 0], TOL_2DEG)
    expect(m).toBeNull()
  })

  it('returns null for a ~zero-length direction', () => {
    expect(axisColorForDirection([0, 0, 0], TOL_2DEG)).toBeNull()
  })

  it('catches a direction within the tolerance (1.5° off +X)', () => {
    const rad = (1.5 * Math.PI) / 180
    const m = axisColorForDirection([Math.cos(rad), Math.sin(rad), 0], TOL_2DEG)
    expect(m).not.toBeNull()
    expect(m!.axis).toBe(0)
  })

  it('does not catch a direction outside the tolerance (5° off +X)', () => {
    const rad = (5 * Math.PI) / 180
    const m = axisColorForDirection([Math.cos(rad), Math.sin(rad), 0], TOL_2DEG)
    expect(m).toBeNull()
  })
})

describe('axisColorsForTheme', () => {
  it('AXIS_COLORS is an alias of DARK_AXIS_COLORS (no behavior change at existing call sites)', () => {
    expect(AXIS_COLORS).toEqual(DARK_AXIS_COLORS)
  })

  it('returns DARK_AXIS_COLORS for "dark"', () => {
    expect(axisColorsForTheme('dark')).toEqual(DARK_AXIS_COLORS)
  })

  it('returns LIGHT_AXIS_COLORS for "light"', () => {
    expect(axisColorsForTheme('light')).toEqual(LIGHT_AXIS_COLORS)
  })

  it('dark and light triples differ', () => {
    expect(LIGHT_AXIS_COLORS).not.toEqual(DARK_AXIS_COLORS)
  })
})
