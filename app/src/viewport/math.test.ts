import { describe, it, expect } from 'vitest'
import { pixelRadiusToAperture, intersectGroundPlane } from './math'

describe('pixelRadiusToAperture', () => {
  it('returns a positive aperture for typical inputs', () => {
    const aperture = pixelRadiusToAperture(8, 480, 45)
    expect(aperture).toBeGreaterThan(0)
  })

  it('scales linearly with snap radius for small angles', () => {
    // For small FOV angles and small snap radii, aperture ≈ proportional to snapRadiusPx
    const a1 = pixelRadiusToAperture(4, 480, 45)
    const a2 = pixelRadiusToAperture(8, 480, 45)
    // a2 should be approximately twice a1 (within 5% due to atan non-linearity)
    expect(a2 / a1).toBeCloseTo(2, 1)
  })

  it('returns zero when snap radius is zero', () => {
    expect(pixelRadiusToAperture(0, 480, 45)).toBe(0)
  })

  it('increases with FOV', () => {
    const narrow = pixelRadiusToAperture(8, 480, 30)
    const wide = pixelRadiusToAperture(8, 480, 60)
    expect(wide).toBeGreaterThan(narrow)
  })

  it('decreases with larger viewport height (same pixel radius = smaller fraction)', () => {
    const small = pixelRadiusToAperture(8, 480, 45)
    const large = pixelRadiusToAperture(8, 960, 45)
    expect(large).toBeLessThan(small)
  })
})

describe('intersectGroundPlane', () => {
  it('intersects a ray pointing straight down', () => {
    const result = intersectGroundPlane({
      origin: [0, 0, 5],
      direction: [0, 0, -1],
    })
    expect(result).not.toBeNull()
    expect(result!.x).toBeCloseTo(0)
    expect(result!.y).toBeCloseTo(0)
    expect(result!.z).toBeCloseTo(0)
  })

  it('intersects a diagonal ray correctly', () => {
    // Origin at (0, 0, 10), direction (1, 0, -1) normalized
    const len = Math.sqrt(2)
    const result = intersectGroundPlane({
      origin: [0, 0, 10],
      direction: [1 / len, 0, -1 / len],
    })
    expect(result).not.toBeNull()
    // t = -10 / (-1/len) = 10 * len; x = 0 + t * (1/len) = 10
    expect(result!.x).toBeCloseTo(10)
    expect(result!.y).toBeCloseTo(0)
    expect(result!.z).toBeCloseTo(0)
  })

  it('returns null for a ray parallel to the plane', () => {
    const result = intersectGroundPlane({
      origin: [0, 0, 1],
      direction: [1, 0, 0],
    })
    expect(result).toBeNull()
  })

  it('returns null for a ray pointing away from the plane', () => {
    // Origin below the plane, pointing further away (negative z, negative dz)
    const result = intersectGroundPlane({
      origin: [0, 0, -1],
      direction: [0, 0, -1],
    })
    expect(result).toBeNull()
  })

  it('returns null for a ray origin above plane pointing up', () => {
    const result = intersectGroundPlane({
      origin: [0, 0, 1],
      direction: [0, 0, 1],
    })
    expect(result).toBeNull()
  })
})
