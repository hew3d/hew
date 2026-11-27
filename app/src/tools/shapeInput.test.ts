import { describe, expect, it } from 'vitest'
import {
  editShapeBuffer,
  formatDegrees,
  formatPercent,
  isShapeInputKey,
  parseAngleBuffer,
  parseRatioBuffer,
  parseShapeToken,
  shapeReadout,
} from './shapeInput'

function type(keys: string): string {
  let buf = ''
  for (const k of keys === '<' ? ['Backspace'] : [...keys]) buf = editShapeBuffer(buf, k, 'm')
  return buf
}

describe('shapeInput', () => {
  it('builds angle, percent and count buffers', () => {
    expect(type('90°')).toBe('90°')
    expect(type('90deg')).toBe('90deg')
    expect(type('60%')).toBe('60%')
    expect(type('24s')).toBe('24s')
    expect(type('-12.5')).toBe('-12.5')
  })

  it('ignores a suffix that has no number in front of it', () => {
    expect(type('s')).toBe('')
    expect(type('d')).toBe('')
    expect(type('%')).toBe('')
    expect(type('°')).toBe('')
  })

  it('freezes after a terminal suffix and spells deg letter by letter', () => {
    expect(type('30°5')).toBe('30°')
    expect(type('30%7')).toBe('30%')
    expect(type('30de5')).toBe('30de')
    expect(type('30dg')).toBe('30d')
    expect(editShapeBuffer('30°', 'Backspace', 'm')).toBe('30')
  })

  it('recognizes its keys', () => {
    for (const k of ['5', '.', '-', '°', '%', 'd', 'e', 'g', 's', 'm', 'Backspace']) {
      expect(isShapeInputKey(k)).toBe(true)
    }
    expect(isShapeInputKey('x')).toBe(false)
    expect(isShapeInputKey('Enter')).toBe(false)
  })

  it('parses suffix tokens', () => {
    const a = parseShapeToken('90°')
    expect(a).toEqual({ kind: 'angle', radians: Math.PI / 2, degrees: 90 })
    expect(parseShapeToken('45deg')).toMatchObject({ kind: 'angle', degrees: 45 })
    expect(parseShapeToken('45d')).toMatchObject({ kind: 'angle', degrees: 45 })
    expect(parseShapeToken('45de')).toBeNull()
    expect(parseShapeToken('60%')).toEqual({ kind: 'percent', ratio: 0.6 })
    expect(parseShapeToken('8s')).toEqual({ kind: 'count', n: 8 })
    expect(parseShapeToken('90')).toBeNull()
    expect(parseShapeToken('0%')).toBeNull()
    expect(parseShapeToken('0s')).toBeNull()
  })

  it('reads a bare number as degrees or percent as the tool asks', () => {
    expect(parseAngleBuffer('90')?.radians).toBeCloseTo(Math.PI / 2, 12)
    expect(parseAngleBuffer('-30°')?.degrees).toBe(-30)
    expect(parseAngleBuffer('60%')).toBeNull()
    expect(parseAngleBuffer('')).toBeNull()
    expect(parseAngleBuffer('-')).toBeNull()
    expect(parseRatioBuffer('60')).toBeCloseTo(0.6, 12)
    expect(parseRatioBuffer('150%')).toBeCloseTo(1.5, 12)
    expect(parseRatioBuffer('30°')).toBeNull()
  })

  it('formats readouts', () => {
    expect(shapeReadout('90', 'angle')).toBe('90°')
    expect(shapeReadout('60', 'percent')).toBe('60%')
    expect(shapeReadout('90deg', 'angle')).toBe('90deg')
    expect(shapeReadout('', 'angle')).toBe('')
    expect(formatDegrees(Math.PI / 2)).toBe('90°')
    expect(formatDegrees(0.2617993878)).toBe('15°')
    expect(formatDegrees(0.1)).toBe('5.7°')
    expect(formatPercent(0.4)).toBe('40%')
    expect(formatPercent(0.4567)).toBe('45.7%')
  })
})
