import { describe, it, expect, afterEach } from 'vitest'
import {
  isFreePick,
  displayedLength,
  displayedAngleDeg,
  displayedFactor,
  sagittaForDisplayedRadius,
} from './displayedValue'
import type { Snap } from './types'
import {
  formatLength,
  parseLengthToMeters,
  setLengthUnit,
  getLengthUnit,
  type LengthFormat,
  setPrecision,
} from '../settings/units'

const snap = (kind: string, extra: Partial<Snap> = {}): Snap => ({ x: 0, y: 0, z: 0, kind, ...extra })

describe('isFreePick', () => {
  it.each(['plane', 'ground', 'on-face', 'parallel', 'perpendicular'])('%s is free', (k) => {
    expect(isFreePick(snap(k))).toBe(true)
  })

  it('on-axis is free only without projectedFrom', () => {
    expect(isFreePick(snap('on-axis'))).toBe(true)
    expect(isFreePick(snap('on-axis', { direction: [1, 0, 0] }))).toBe(true)
    expect(isFreePick(snap('on-axis', { projectedFrom: [1, 2, 3] }))).toBe(false)
  })

  it('an on-axis pick is free only when its axis runs through the anchor', () => {
    // A soft axis from the anchor (0.1234, 0.5678, 0) along red.
    const soft = snap('on-axis', { x: 0.9, y: 0.5678, z: 0, direction: [1, 0, 0] })
    expect(isFreePick(soft, [0.1234, 0.5678, 0])).toBe(true)
    // The red drawing axis itself (y = 0), clicked from that anchor: the
    // displayed length from the anchor would land off the axis.
    const red = snap('on-axis', { x: 0.3, y: 0, z: 0, direction: [1, 0, 0] })
    expect(isFreePick(red, [0.1234, 0.5678, 0])).toBe(false)
    // From an anchor ON the red axis, the same pick stays on it when rounded.
    expect(isFreePick(red, [-2, 0, 0])).toBe(true)
    // No anchor: the direction alone decides nothing.
    expect(isFreePick(red)).toBe(true)
  })

  it.each([
    'endpoint', 'center', 'quadrant', 'midpoint', 'intersection', 'tangent',
    'on-edge', 'on-intersection-line', 'on-guide', 'from-point', 'face', 'something-new', '',
  ])('%s is a hard pick', (k) => {
    expect(isFreePick(snap(k))).toBe(false)
  })

  it('no snap is not free', () => {
    expect(isFreePick(null)).toBe(false)
    expect(isFreePick(undefined)).toBe(false)
  })
})

describe('displayedLength', () => {
  const prev = getLengthUnit()
  afterEach(() => setLengthUnit(prev))

  const formats: LengthFormat[] = ['m', 'cm', 'mm', 'dec_in', 'frac_in', 'arch']
  const samples = [
    0.0093987, 0.0037873, 0.0106123, 1.23456789, 0.5, 2, 0.000049, 0.0127, 0.3048, 7.7777, 123.456,
    0.00949999, 0.00950001, 1e-9, 0.0254 * 11.97, 0.0254 * 11.99, 0.0254 / 32,
  ]

  it.each(formats)('is bit-identical to parse(format(x)) in %s, for either sign', (fmt) => {
    setLengthUnit(fmt)
    for (const x of samples) {
      const parsed = parseLengthToMeters(formatLength(x))!
      expect(Object.is(displayedLength(x), parsed === 0 ? 0 : parsed)).toBe(true)
      const neg = displayedLength(-x)
      expect(neg === 0 ? true : neg === -displayedLength(x)).toBe(true)
    }
  })

  it('at Exact, reads 9.39876 mm as exactly 9.399 mm', () => {
    setLengthUnit('mm')
    setPrecision('exact')
    try {
      expect(displayedLength(0.00939876)).toBe(9.399 * 0.001)
      expect(displayedLength(0.00378732)).toBe(3.787 * 0.001)
      expect(displayedLength(0.0106123)).toBe(10.612 * 0.001)
    } finally {
      setPrecision(1)
    }
  })

  it('at the default mm precision (1 decimal), reads 9.39876 mm as exactly the rounded, ~-marked value', () => {
    setLengthUnit('mm')
    expect(formatLength(0.00939876)).toBe('~9.4 mm')
    expect(displayedLength(0.00939876)).toBe(9.4 * 0.001)
    expect(displayedLength(0.00378732)).toBe(3.8 * 0.001)
    expect(displayedLength(0.0106123)).toBe(10.6 * 0.001)
  })

  it('metric formats show the digits the value needs at Exact, and round at a coarser precision', () => {
    setLengthUnit('m')
    setPrecision('exact')
    setLengthUnit('cm')
    setPrecision('exact')
    try {
      setLengthUnit('m')
      expect(displayedLength(1.23456)).toBe(1.23456)
      setLengthUnit('cm')
      expect(displayedLength(0.123456)).toBe(12.3456 * 0.01)
      setLengthUnit('m')
      setPrecision(3)
      expect(displayedLength(1.23456)).toBe(1.235)
    } finally {
      setLengthUnit('m')
      setPrecision(3)
      setLengthUnit('cm')
      setPrecision(2)
    }
  })

  it('imperial formats show the digits the value needs at Exact', () => {
    setLengthUnit('dec_in')
    setPrecision('exact')
    try {
      expect(displayedLength(0.0254 * 3.14159)).toBe(3.1416 * 0.0254)
    } finally {
      setPrecision(3)
    }
    setLengthUnit('arch')
    // 1' 3-1/4" shown for 15.26 in -> nearest 1/16 in
    expect(displayedLength(0.0254 * 15.26)).toBe(parseLengthToMeters(formatLength(0.0254 * 15.26))!)
  })

  it('keeps the sign and never returns -0', () => {
    setLengthUnit('mm')
    setPrecision('exact')
    try {
      expect(displayedLength(-0.00939876)).toBe(-9.399 * 0.001)
    } finally {
      setPrecision(1)
    }
    expect(Object.is(displayedLength(-0.0000001), 0)).toBe(true)
    expect(Object.is(displayedLength(-0), 0)).toBe(true)
    expect(Object.is(displayedLength(0), 0)).toBe(true)
  })

  it('half-way ties follow the display text (toFixed), identically to typing it', () => {
    setLengthUnit('mm')
    for (const x of [0.00005, 0.00015, 0.00025, 0.00035, 0.0095]) {
      expect(displayedLength(x)).toBe(parseLengthToMeters(formatLength(x))!)
    }
  })

  it('passes non-finite values through', () => {
    expect(displayedLength(NaN)).toBeNaN()
    expect(displayedLength(Infinity)).toBe(Infinity)
  })
})

describe('displayedAngleDeg / displayedFactor', () => {
  it('angle equals parseFloat of the toFixed(1) text', () => {
    for (const d of [37.46, 37.449, 0.04, 89.96, -12.34, 179.95, 15]) {
      expect(displayedAngleDeg(d)).toBe(parseFloat(d.toFixed(1)) === 0 ? 0 : parseFloat(d.toFixed(1)))
    }
    expect(displayedAngleDeg(37.46)).toBe(37.5)
    expect(Object.is(displayedAngleDeg(-0.04), 0)).toBe(true)
  })

  it('factor equals parseFloat of the toFixed(2) text', () => {
    for (const f of [1.61374, 0.499, 2.005, 1.004, 3]) {
      expect(displayedFactor(f)).toBe(parseFloat(f.toFixed(2)))
    }
    expect(displayedFactor(1.61374)).toBe(1.61)
    expect(Object.is(displayedFactor(-0.001), 0)).toBe(true)
  })
})

describe('sagittaForDisplayedRadius', () => {
  const prev = getLengthUnit()
  afterEach(() => setLengthUnit(prev))
  const radiusOf = (c: number, s: number) => (c * c) / (8 * s) + s / 2

  it('a minor arc gets the minor-side sagitta of the displayed radius', () => {
    setLengthUnit('mm')
    const c = 0.02
    const s = 0.0043217 // minor (s < c/2)
    const R = displayedLength(radiusOf(c, s))
    const sd = sagittaForDisplayedRadius(c, s, false)!
    expect(sd).toBeLessThan(c / 2)
    expect(radiusOf(c, sd)).toBeCloseTo(R, 15)
  })

  it('a major arc keeps the major side (s > c/2)', () => {
    setLengthUnit('mm')
    const c = 0.02
    const s = 0.0151234 // major
    const R = displayedLength(radiusOf(c, s))
    const sd = sagittaForDisplayedRadius(c, s, true)!
    expect(sd).toBeGreaterThan(c / 2)
    expect(radiusOf(c, sd)).toBeCloseTo(R, 15)
  })

  it('a semicircle (s = c/2) stays a semicircle when the radius reads c/2', () => {
    setLengthUnit('mm')
    const sd = sagittaForDisplayedRadius(0.02, 0.01, false)!
    expect(sd).toBeCloseTo(0.01, 12)
  })

  it('falls back (null) when the displayed radius cannot span the chord', () => {
    setLengthUnit('m')
    // chord 0.0100004 m, s tiny-ish near semicircle: R ~ 0.005 shows 0.005 < c/2 = 0.0050002
    const c = 0.0100004
    const s = c / 2 - 1e-9
    expect(sagittaForDisplayedRadius(c, s, false)).toBeNull()
  })

  it('rejects degenerate input', () => {
    expect(sagittaForDisplayedRadius(0, 0.01, false)).toBeNull()
    expect(sagittaForDisplayedRadius(0.02, 0, false)).toBeNull()
  })
})
