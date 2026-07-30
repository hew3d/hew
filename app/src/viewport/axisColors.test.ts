import { describe, it, expect } from 'vitest'
import {
  axisColorForDirection,
  axisColorsForTheme,
  AXIS_COLORS,
  DARK_AXIS_COLORS,
  LIGHT_AXIS_COLORS,
} from './axisColors'
import type { DrawingAxes } from '../tools/drawingAxes'

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

  // ── Movable drawing axes (tool-parity §4): a non-identity frame ──
  const ROTATED_FRAME: DrawingAxes = {
    origin: [0, 0, 0],
    x: [0, 1, 0],
    y: [-1, 0, 0],
    z: [0, 0, 1],
  }

  it('world code path is unchanged at identity — no frame argument matches literal world axes', () => {
    const m = axisColorForDirection([1, 0, 0], TOL_2DEG)
    expect(m).not.toBeNull()
    expect(m!.axis).toBe(0)
    expect(m!.color).toBe(AXIS_COLORS[0])
  })

  it('a rotated frame\'s red (X) axis direction yields the red color and axis 0, not a miss', () => {
    // The frame's red axis is world [0,1,0] — a direction that is NOT within
    // tolerance of literal world X at all, so a pass under the un-fixed
    // (world-only) primitive would return null here.
    const m = axisColorForDirection(ROTATED_FRAME.x, TOL_2DEG, AXIS_COLORS, ROTATED_FRAME)
    expect(m).not.toBeNull()
    expect(m!.axis).toBe(0)
    expect(m!.color).toBe(AXIS_COLORS[0])
    expect(m!.snapped).toEqual(ROTATED_FRAME.x)
  })

  it('a rotated frame\'s green (Y) axis direction yields the green color and axis 1', () => {
    const m = axisColorForDirection(ROTATED_FRAME.y, TOL_2DEG, AXIS_COLORS, ROTATED_FRAME)
    expect(m).not.toBeNull()
    expect(m!.axis).toBe(1)
    expect(m!.color).toBe(AXIS_COLORS[1])
  })

  it('literal world X reads as the frame\'s green axis (not red) when the frame has rotated onto it', () => {
    // ROTATED_FRAME.y is [-1,0,0] — world X is anti-parallel to the frame's
    // OWN green axis here, so it must match axis 1 (green), never axis 0
    // (red) as an unfixed world-only comparison would report.
    const m = axisColorForDirection([1, 0, 0], TOL_2DEG, AXIS_COLORS, ROTATED_FRAME)
    expect(m).not.toBeNull()
    expect(m!.axis).toBe(1)
    expect(m!.color).toBe(AXIS_COLORS[1])
  })

  it('a direction off every axis of BOTH frames returns null regardless of `frame`', () => {
    const d = 1 / Math.sqrt(3)
    const m = axisColorForDirection([d, d, d], TOL_2DEG, AXIS_COLORS, ROTATED_FRAME)
    expect(m).toBeNull()
  })

  it('omitting `frame` still defaults to world identity (source-compatible with every existing caller)', () => {
    const withDefault = axisColorForDirection([1, 0, 0], TOL_2DEG)
    const withExplicitWorld = axisColorForDirection([1, 0, 0], TOL_2DEG, AXIS_COLORS, {
      origin: [0, 0, 0],
      x: [1, 0, 0],
      y: [0, 1, 0],
      z: [0, 0, 1],
    })
    expect(withDefault).toEqual(withExplicitWorld)
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
