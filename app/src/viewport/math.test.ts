import { describe, it, expect } from 'vitest'
import {
  pixelRadiusToAperture,
  intersectGroundPlane,
  tanHalfFovRad,
  screenConstantWorldHalf,
  legacyScreenConstantToPixels,
  LEGACY_REFERENCE_FOV_DEG,
  LEGACY_REFERENCE_VIEWPORT_HEIGHT_PX,
} from './math'

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

describe('tanHalfFovRad', () => {
  it('matches Math.tan(fov/2 in radians) directly', () => {
    for (const fov of [10, 45, 60, 90, 120]) {
      expect(tanHalfFovRad(fov)).toBeCloseTo(Math.tan((fov * Math.PI) / 360), 12)
    }
  })
})

describe('screenConstantWorldHalf', () => {
  const tanHalf45 = tanHalfFovRad(45)

  it('implements the perspective-projection inverse: worldHalf = px · dist · tanHalfFov / viewportHeight', () => {
    const worldHalf = screenConstantWorldHalf(10, 20, tanHalf45, 800)
    expect(worldHalf).toBeCloseTo((10 * 20 * tanHalf45) / 800, 12)
  })

  it('is stable under fov change: the SAME world point stays the SAME screen size as fov varies, given the derived desiredPixels', () => {
    // Pick an arbitrary widget size at a reference fov/viewport, then confirm
    // a DIFFERENT fov, fed through the real formula, reproduces the same
    // desiredPixels back out — i.e. no drift, unlike a baked `k · dist`.
    const dist = 12
    const viewportHeight = 600
    const desiredPixels = 50
    for (const fov of [20, 45, 70, 100]) {
      const tanHalf = tanHalfFovRad(fov)
      const worldHalf = screenConstantWorldHalf(desiredPixels, dist, tanHalf, viewportHeight)
      // Invert: pixels = worldHalf * viewportHeight / (dist * tanHalf)
      const recoveredPixels = (worldHalf * viewportHeight) / (dist * tanHalf)
      expect(recoveredPixels).toBeCloseTo(desiredPixels, 9)
    }
  })

  it('is stable under viewport resize: the SAME desiredPixels recovers cleanly at any viewport height', () => {
    const dist = 8
    const desiredPixels = 30
    for (const viewportHeight of [400, 600, 900, 1440]) {
      const worldHalf = screenConstantWorldHalf(desiredPixels, dist, tanHalf45, viewportHeight)
      const recoveredPixels = (worldHalf * viewportHeight) / (dist * tanHalf45)
      expect(recoveredPixels).toBeCloseTo(desiredPixels, 9)
    }
  })

  it('scales linearly with camera distance for a fixed fov/viewport', () => {
    const near = screenConstantWorldHalf(10, 5, tanHalf45, 800)
    const far = screenConstantWorldHalf(10, 15, tanHalf45, 800)
    expect(far / near).toBeCloseTo(3, 9)
  })

  it('clamps to minWorldHalf when the raw result would be smaller', () => {
    const worldHalf = screenConstantWorldHalf(1, 0.001, tanHalf45, 800, 0.5)
    expect(worldHalf).toBe(0.5)
  })

  it('does not clamp when the raw result already exceeds minWorldHalf', () => {
    const raw = (10 * 20 * tanHalf45) / 800
    const worldHalf = screenConstantWorldHalf(10, 20, tanHalf45, 800, 1e-9)
    expect(worldHalf).toBeCloseTo(raw, 12)
  })

  it('falls back to minWorldHalf for a degenerate (zero or negative) viewport height', () => {
    expect(screenConstantWorldHalf(10, 20, tanHalf45, 0, 0.25)).toBe(0.25)
    expect(screenConstantWorldHalf(10, 20, tanHalf45, -5, 0.25)).toBe(0.25)
  })

  it('defaults minWorldHalf to 0', () => {
    expect(screenConstantWorldHalf(10, 20, tanHalf45, 0)).toBe(0)
  })
})

describe('legacyScreenConstantToPixels', () => {
  it('round-trips: feeding the derived desiredPixels back through screenConstantWorldHalf at the SAME reference reproduces the old k · dist value', () => {
    const k = 0.06
    const refFov = 45
    const refHeight = 720
    const desiredPixels = legacyScreenConstantToPixels(k, refFov, refHeight)
    for (const dist of [1, 4, 12.5, 100]) {
      const worldHalf = screenConstantWorldHalf(desiredPixels, dist, tanHalfFovRad(refFov), refHeight)
      expect(worldHalf).toBeCloseTo(k * dist, 9)
    }
  })

  it('is linear in k (so ratios between widgets are preserved regardless of the chosen reference)', () => {
    const px1 = legacyScreenConstantToPixels(0.03, 45, 720)
    const px2 = legacyScreenConstantToPixels(0.06, 45, 720)
    expect(px2 / px1).toBeCloseTo(2, 9)
  })

  it('the app-wide default reference (45°, 720px) is exported and usable directly', () => {
    const desiredPixels = legacyScreenConstantToPixels(0.06, LEGACY_REFERENCE_FOV_DEG, LEGACY_REFERENCE_VIEWPORT_HEIGHT_PX)
    expect(desiredPixels).toBeGreaterThan(0)
    const worldHalf = screenConstantWorldHalf(
      desiredPixels,
      4,
      tanHalfFovRad(LEGACY_REFERENCE_FOV_DEG),
      LEGACY_REFERENCE_VIEWPORT_HEIGHT_PX,
    )
    expect(worldHalf).toBeCloseTo(0.06 * 4, 9)
  })
})
