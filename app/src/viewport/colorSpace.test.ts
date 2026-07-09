import { describe, expect, it } from 'vitest'
import { srgbColorsToLinear, srgbToLinear } from './colorSpace'

describe('srgbToLinear', () => {
  it('keeps the endpoints fixed', () => {
    expect(srgbToLinear(0)).toBe(0)
    expect(srgbToLinear(1)).toBeCloseTo(1, 10)
  })

  it('uses the linear segment below the sRGB knee', () => {
    expect(srgbToLinear(0.04)).toBeCloseTo(0.04 / 12.92, 10)
  })

  it('matches the reference value for the default face grey (0xcc)', () => {
    // sRGB 204/255 ≈ 0.8 → linear ≈ 0.60383
    expect(srgbToLinear(204 / 255)).toBeCloseTo(0.60383, 4)
  })

  it('is monotonic and darkens every mid-tone', () => {
    let prev = 0
    for (let i = 1; i <= 100; i++) {
      const c = i / 100
      const lin = srgbToLinear(c)
      expect(lin).toBeGreaterThan(prev)
      if (c > 0.04045 && c < 1) expect(lin).toBeLessThan(c)
      prev = lin
    }
  })
})

describe('srgbColorsToLinear', () => {
  it('converts in place and returns the same array', () => {
    const arr = new Float32Array([0, 0.5, 1, 204 / 255])
    const out = srgbColorsToLinear(arr)
    expect(out).toBe(arr)
    expect(arr[0]).toBe(0)
    expect(arr[1]).toBeCloseTo(srgbToLinear(0.5), 6)
    expect(arr[2]).toBeCloseTo(1, 6)
    expect(arr[3]).toBeCloseTo(0.60383, 4)
  })
})
