import { describe, expect, it } from 'vitest'
import { editCurveBuffer, isCurveInputKey, parseArrayInput, parseCurveInput } from './curveInput'

const near = (a: number, b: number): void => expect(a).toBeCloseTo(b, 9)

describe('parseCurveInput', () => {
  it('reads a bare number in the display unit', () => {
    const t = parseCurveInput('6', 'mm')
    expect(t).toMatchObject({ kind: 'length' })
    near((t as { meters: number }).meters, 0.006)
    near((parseCurveInput('6', 'cm') as { meters: number }).meters, 0.06)
    near((parseCurveInput('2', 'm') as { meters: number }).meters, 2)
  })

  it('reads explicit units and imperial marks', () => {
    near((parseCurveInput('6mm', 'm') as { meters: number }).meters, 0.006)
    near((parseCurveInput('1/4"', 'arch') as { meters: number }).meters, 0.00635)
    near((parseCurveInput('2-1/4"', 'arch') as { meters: number }).meters, 0.05715)
  })

  it('reads a pair of lengths', () => {
    const t = parseCurveInput('6,3', 'mm')
    expect(t).toMatchObject({ kind: 'pair' })
    near((t as { a: number }).a, 0.006)
    near((t as { b: number }).b, 0.003)
    expect(parseCurveInput('1,2,3', 'mm')).toHaveProperty('error')
    expect(parseCurveInput('6,', 'mm')).toHaveProperty('error')
  })

  it('reads a length and an angle', () => {
    for (const text of ['3,45°', '3,45deg', '3, 45 deg']) {
      const t = parseCurveInput(text, 'mm')
      expect(t).toMatchObject({ kind: 'lengthAngle' })
      near((t as { meters: number }).meters, 0.003)
      near((t as { radians: number }).radians, Math.PI / 4)
    }
  })

  it('reads an angle', () => {
    near((parseCurveInput('30°', 'mm') as { radians: number }).radians, Math.PI / 6)
    near((parseCurveInput('90deg', 'mm') as { radians: number }).radians, Math.PI / 2)
    near((parseCurveInput('-15°', 'mm') as { radians: number }).radians, -Math.PI / 12)
  })

  it('reads a percentage as a ratio', () => {
    near((parseCurveInput('60%', 'mm') as { ratio: number }).ratio, 0.6)
    near((parseCurveInput('250%', 'mm') as { ratio: number }).ratio, 2.5)
  })

  it('reads a ramp with either arrow', () => {
    for (const text of ['100%→40%', '100%->40%', '100% > 40%']) {
      const t = parseCurveInput(text, 'mm')
      expect(t).toMatchObject({ kind: 'ramp' })
      near((t as { from: number }).from, 1)
      near((t as { to: number }).to, 0.4)
    }
  })

  it('reads a segment count and a copy count', () => {
    expect(parseCurveInput('8s', 'mm')).toEqual({ kind: 'segments', n: 8 })
    expect(parseCurveInput('5x', 'mm')).toEqual({ kind: 'count', n: 5 })
    expect(parseCurveInput('x5', 'mm')).toEqual({ kind: 'count', n: 5 })
    expect(parseCurveInput('0s', 'mm')).toHaveProperty('error')
  })

  it('reports an error for empty or garbled text', () => {
    expect(parseCurveInput('', 'mm')).toHaveProperty('error')
    expect(parseCurveInput('abc', 'mm')).toHaveProperty('error')
  })
})

describe('curve input keys', () => {
  it('accepts the grammar keys and nothing else', () => {
    for (const k of ['5', '.', ',', '-', '/', "'", '"', '°', '%', 's', 'x', '>', 'm', 'c', 'Backspace']) {
      expect(isCurveInputKey(k)).toBe(true)
    }
    for (const k of ['Enter', 'Escape', 'a', 'z', 'ArrowUp', 'Tab']) {
      expect(isCurveInputKey(k)).toBe(false)
    }
  })

  it('edits a buffer', () => {
    let b = ''
    for (const k of ['6', ',', '3', 'Backspace', '4']) b = editCurveBuffer(b, k)
    expect(b).toBe('6,4')
    expect(editCurveBuffer('6', 'Enter')).toBe('6')
  })
})

describe('parseCurveInput (counts and ramps)', () => {
  it('reads counts in either order', () => {
    expect(parseCurveInput('5x', 'm')).toEqual({ kind: 'count', n: 5 })
    expect(parseCurveInput('x12', 'm')).toEqual({ kind: 'count', n: 12 })
    expect(parseCurveInput('12*', 'm')).toEqual({ kind: 'count', n: 12 })
  })

  it('reads a scale ramp as ratios with either arrow', () => {
    expect(parseCurveInput('100%→40%', 'm')).toEqual({ kind: 'ramp', from: 1, to: 0.4 })
    expect(parseCurveInput('100%->40%', 'm')).toEqual({ kind: 'ramp', from: 1, to: 0.4 })
    expect(parseCurveInput(' 50 % -> 150.5 % ', 'm')).toEqual({ kind: 'ramp', from: 0.5, to: 1.505 })
  })

  it('refuses zero scales, a zero count and junk', () => {
    expect(parseCurveInput('0%→40%', 'm')).toHaveProperty('error')
    expect(parseCurveInput('100%→0%', 'm')).toHaveProperty('error')
    expect(parseCurveInput('0x', 'm')).toHaveProperty('error')
    expect(parseCurveInput('', 'm')).toHaveProperty('error')
    // A bare number is a length in the full grammar; Array along Path reads it as a count (parseArrayInput).
    expect(parseCurveInput('12', 'm')).toEqual({ kind: 'length', meters: 12 })
    expect(parseCurveInput('abc', 'm')).toHaveProperty('error')
  })
})

describe('parseArrayInput', () => {
  it('takes a bare whole number as a count', () => {
    expect(parseArrayInput('12', 'm')).toEqual({ kind: 'count', n: 12 })
    expect(parseArrayInput('12x', 'm')).toEqual({ kind: 'count', n: 12 })
    expect(parseArrayInput('100%→40%', 'm')).toEqual({ kind: 'ramp', from: 1, to: 0.4 })
    expect(parseArrayInput('1.5', 'm')).toHaveProperty('error')
  })
})

describe('buffer editing (array keys)', () => {
  it('accepts the grammar keys and edits with Backspace', () => {
    let b = ''
    for (const k of ['1', '0', '0', '%', '-', '>', '4', '0', '%']) b = editCurveBuffer(b, k)
    expect(b).toBe('100%->40%')
    expect(editCurveBuffer(b, 'Backspace')).toBe('100%->40')
    expect(editCurveBuffer('5', 'X')).toBe('5x')
    expect(editCurveBuffer('5', 'Q')).toBe('5')
  })

  it('knows its keys', () => {
    for (const k of ['0', '9', '.', '%', '-', '>', 'x', '→', '°']) expect(isCurveInputKey(k)).toBe(true)
    for (const k of ['Enter', 'Escape', 'q', ' ']) expect(isCurveInputKey(k)).toBe(false)
  })
})
