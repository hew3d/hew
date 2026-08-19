import { describe, expect, it } from 'vitest'
import { fitScale, parseCustomScale, ratioText, scaleDisplay, scaleHint, scalePresets, scaleRatio, sameScale } from './scale'

describe('scale presets', () => {
  it('ladders are ordered largest scale first and carry ratio labels', () => {
    for (const sys of ['metric', 'imperial'] as const) {
      const ladder = scalePresets(sys)
      for (let i = 1; i < ladder.length; i++) expect(scaleRatio(ladder[i])).toBeLessThan(scaleRatio(ladder[i - 1]))
    }
    const imp = scalePresets('imperial')
    const quarter = imp.find((s) => s.label === '¼" = 1\'')!
    expect(ratioText(quarter)).toBe('1:48')
    expect(scaleDisplay(quarter)).toBe('¼" = 1\' (1:48)')
    const one = scalePresets('metric').find((s) => s.label === '1:10')!
    expect(scaleDisplay(one)).toBe('1:10')
    expect(scaleHint(one, 'cm')).toBe('1 cm = 10 cm')
    expect(scaleHint(one, 'm')).toBe('1 cm = 10 cm')
    expect(scaleHint(one, 'mm')).toBe('1 mm = 10 mm')
    const hundred = scalePresets('metric').find((s) => s.label === '1:100')!
    expect(scaleHint(hundred, 'm')).toBe('1 cm = 1 m')
    expect(scaleHint(one, 'arch')).toBeNull()
  })
})

describe('custom scale parsing', () => {
  it("Kurt's phrases: 1 cm = 1 m, 1 in = 2 ft, 1 in = 1 in", () => {
    const a = parseCustomScale('1cm', '1m', 'm')!
    expect(scaleRatio(a)).toBeCloseTo(0.01, 12)
    expect(ratioText(a)).toBe('1:100')
    const b = parseCustomScale('1in', '2 ft', 'arch')!
    expect(ratioText(b)).toBe('1:24')
    expect(b.label).toBe('1" = 2\'')
    const c = parseCustomScale("1'", "1'", 'arch')!
    expect(ratioText(c)).toBe('1:1')
  })
  it('bare numbers use the current format; bad input is null', () => {
    const s = parseCustomScale('1', '10', 'cm')!
    expect(ratioText(s)).toBe('1:10')
    expect(parseCustomScale('', '10', 'cm')).toBeNull()
    expect(parseCustomScale('0', '10', 'cm')).toBeNull()
    expect(parseCustomScale('1', '100000', 'cm')).toBeNull()
    expect(parseCustomScale('100', '1', 'cm')).toBeNull()
  })
})

describe('fit', () => {
  it('is the exact ratio that fills the page, as a custom scale labelled by its ratio', () => {
    expect(fitScale(0.11, 'metric').label).toBe('1:9.09')
    expect(fitScale(0.1, 'metric').label).toBe('1:10')
    expect(fitScale(1 / 40, 'imperial').label).toBe('1:40')
    const tiny = fitScale(1 / 317, 'metric')
    expect(tiny.label).toBe('1:317')
    expect(tiny.kind).toBe('fit')
    // The label IS the ratio, so the full display never doubles it up.
    expect(scaleDisplay(tiny)).toBe('1:317')
    expect(scaleRatio(fitScale(0.11, 'metric'))).toBeCloseTo(0.11, 12)
    expect(sameScale(fitScale(0.5, 'metric'), { paperMeters: 1, modelMeters: 2, label: '', kind: 'custom' })).toBe(true)
    // Clamped to the custom-scale range.
    expect(scaleRatio(fitScale(1e-6, 'metric'))).toBeCloseTo(1 / 1000, 12)
  })
})
