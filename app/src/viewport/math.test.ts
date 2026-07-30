import { describe, it, expect } from 'vitest'
import {
  pixelRadiusToAperture,
  intersectGroundPlane,
  tanHalfFovRad,
  screenConstantWorldHalf,
  legacyScreenConstantToPixels,
  axisDashGapWorld,
  scaleCameraAboutOrigin,
  scaleViewLimits,
  zoomExtentsViewLimits,
  MOUNT_FIT_DISTANCE,
  MOUNT_LIMITS,
  LEGACY_REFERENCE_FOV_DEG,
  LEGACY_REFERENCE_VIEWPORT_HEIGHT_PX,
  worldPerPixelPerspective,
  worldPerPixelOrtho,
  orthoZoomBounds,
  screenConstantWorldHalfFromWorldPerPixel,
  axisDashGapWorldFromWorldPerPixel,
  apertureForPixelRadius,
  apertureModeFor,
  type ApertureBasis,
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

describe('axisDashGapWorld', () => {
  const tanHalf45 = tanHalfFovRad(45)

  it('is exactly screenConstantWorldHalf applied separately to dash and gap pixel targets', () => {
    const { dashSize, gapSize } = axisDashGapWorld(9, 7, 20, tanHalf45, 800)
    expect(dashSize).toBeCloseTo(screenConstantWorldHalf(9, 20, tanHalf45, 800), 12)
    expect(gapSize).toBeCloseTo(screenConstantWorldHalf(7, 20, tanHalf45, 800), 12)
  })

  it('the fix: dash/gap grow linearly with camera-to-origin distance (screen-constant), unlike a flat world constant', () => {
    const near = axisDashGapWorld(9, 7, 1, tanHalf45, 800)
    const far = axisDashGapWorld(9, 7, 100, tanHalf45, 800)
    // 100x the distance -> 100x the world-space dash/gap, so the apparent
    // on-screen (pixel) length stays the same at both distances.
    expect(far.dashSize / near.dashSize).toBeCloseTo(100, 9)
    expect(far.gapSize / near.gapSize).toBeCloseTo(100, 9)
    // At a cm-scale distance the world dash length is correspondingly tiny
    // (many periods fit in view, unlike the old fixed 0.28 m which read
    // solid at this scale) — sanity bound, not a precise expectation.
    expect(near.dashSize).toBeLessThan(0.05)
  })

  it('preserves the dash:gap ratio at every distance (both scale by the same factor)', () => {
    for (const dist of [0.5, 3, 15, 80]) {
      const { dashSize, gapSize } = axisDashGapWorld(9, 7, dist, tanHalf45, 800)
      expect(dashSize / gapSize).toBeCloseTo(9 / 7, 9)
    }
  })

  it('is stable under fov and viewport-height changes for the same desired pixel sizes', () => {
    const dist = 12
    for (const fov of [20, 45, 90]) {
      for (const viewportHeight of [480, 900]) {
        const { dashSize, gapSize } = axisDashGapWorld(9, 7, dist, tanHalfFovRad(fov), viewportHeight)
        // Invert back to pixels the same way math.test.ts's screenConstantWorldHalf block does.
        const tanHalf = tanHalfFovRad(fov)
        expect((dashSize * viewportHeight) / (dist * tanHalf)).toBeCloseTo(9, 9)
        expect((gapSize * viewportHeight) / (dist * tanHalf)).toBeCloseTo(7, 9)
      }
    }
  })

  it('floors both dash and gap at minWorld for a degenerate (near-zero) distance', () => {
    const { dashSize, gapSize } = axisDashGapWorld(9, 7, 0, tanHalf45, 800, 1e-5)
    expect(dashSize).toBe(1e-5)
    expect(gapSize).toBe(1e-5)
  })

  it('defaults minWorld to 0 (no floor) when omitted', () => {
    const { dashSize, gapSize } = axisDashGapWorld(9, 7, 0, tanHalf45, 800)
    expect(dashSize).toBe(0)
    expect(gapSize).toBe(0)
  })
})

describe('worldPerPixelPerspective — parity with the legacy fov math', () => {
  it('matches 2 · dist · tanHalfFov / viewportHeight directly', () => {
    const dist = 12, fov = 50, vpH = 640
    expect(worldPerPixelPerspective(dist, fov, vpH)).toBeCloseTo(
      (2 * dist * tanHalfFovRad(fov)) / vpH, 12,
    )
  })

  it('feeding it through screenConstantWorldHalfFromWorldPerPixel reproduces screenConstantWorldHalf exactly, at every dist/fov/viewport combo', () => {
    for (const dist of [0.5, 4, 12.5, 100]) {
      for (const fov of [20, 45, 70, 100]) {
        for (const vpH of [400, 720, 1440]) {
          const legacy = screenConstantWorldHalf(10, dist, tanHalfFovRad(fov), vpH)
          const rigAware = screenConstantWorldHalfFromWorldPerPixel(
            10, worldPerPixelPerspective(dist, fov, vpH),
          )
          expect(rigAware).toBeCloseTo(legacy, 9)
        }
      }
    }
  })

  it('is zero for a degenerate (<=0) viewport height', () => {
    expect(worldPerPixelPerspective(10, 45, 0)).toBe(0)
    expect(worldPerPixelPerspective(10, 45, -5)).toBe(0)
  })
})

describe('worldPerPixelOrtho', () => {
  it('is frustumHeight / zoom / viewportHeight', () => {
    expect(worldPerPixelOrtho(20, 2, 800)).toBeCloseTo(20 / 2 / 800, 12)
  })

  it('is INDEPENDENT of camera distance — the defining ortho behavior worldPerPixelPerspective cannot express', () => {
    // A perspective worldPerPixel grows linearly with dist; an ortho one has
    // no dist parameter at all — the same frustumHeight/zoom/viewportHeight
    // applies to a point 1 m away or 1000 m away.
    const near = worldPerPixelOrtho(20, 1, 800)
    const far = worldPerPixelOrtho(20, 1, 800) // same call — no dist to vary
    expect(near).toBe(far)
  })

  it('halving zoom doubles the world-per-pixel (zooming out)', () => {
    expect(worldPerPixelOrtho(20, 0.5, 800)).toBeCloseTo(worldPerPixelOrtho(20, 1, 800) * 2, 12)
  })

  it('is zero for a degenerate (<=0) viewport height or zoom', () => {
    expect(worldPerPixelOrtho(20, 1, 0)).toBe(0)
    expect(worldPerPixelOrtho(20, 0, 800)).toBe(0)
    expect(worldPerPixelOrtho(20, -1, 800)).toBe(0)
  })
})

describe('orthoZoomBounds', () => {
  it('spans the same total ratio as maxDistance/minDistance, symmetric in log-space around zoom = 1', () => {
    const { minZoom, maxZoom } = orthoZoomBounds(0.1, 50) // ratio 500, matching configureControls
    expect(maxZoom / minZoom).toBeCloseTo(500, 9)
    expect(minZoom).toBeLessThan(1)
    expect(maxZoom).toBeGreaterThan(1)
    // Symmetric in log space: log(minZoom) == -log(maxZoom).
    expect(Math.log(minZoom)).toBeCloseTo(-Math.log(maxZoom), 9)
  })

  it('equal min/max distance gives a zoom range of exactly [1, 1] (no headroom either way)', () => {
    const { minZoom, maxZoom } = orthoZoomBounds(5, 5)
    expect(minZoom).toBeCloseTo(1, 9)
    expect(maxZoom).toBeCloseTo(1, 9)
  })

  it('never returns a zero/negative bound even for a degenerate (<=0) input', () => {
    const { minZoom, maxZoom } = orthoZoomBounds(0, 0)
    expect(minZoom).toBeGreaterThan(0)
    expect(maxZoom).toBeGreaterThan(0)
    expect(Number.isFinite(minZoom)).toBe(true)
    expect(Number.isFinite(maxZoom)).toBe(true)
  })
})

describe('screenConstantWorldHalfFromWorldPerPixel', () => {
  it('is desiredPixels · worldPerPixel / 2, clamped to minWorldHalf', () => {
    expect(screenConstantWorldHalfFromWorldPerPixel(10, 4)).toBeCloseTo(20, 12)
    expect(screenConstantWorldHalfFromWorldPerPixel(10, 0, 0.5)).toBe(0.5)
  })
})

describe('axisDashGapWorldFromWorldPerPixel', () => {
  it('applies screenConstantWorldHalfFromWorldPerPixel to dash/gap separately', () => {
    const { dashSize, gapSize } = axisDashGapWorldFromWorldPerPixel(9, 7, 4)
    expect(dashSize).toBeCloseTo(screenConstantWorldHalfFromWorldPerPixel(9, 4), 12)
    expect(gapSize).toBeCloseTo(screenConstantWorldHalfFromWorldPerPixel(7, 4), 12)
  })
})

describe('apertureForPixelRadius', () => {
  it('perspective basis matches pixelRadiusToAperture exactly (radians)', () => {
    const basis: ApertureBasis = { kind: 'perspective', fovYDeg: 45 }
    expect(apertureForPixelRadius(basis, 8, 800)).toBeCloseTo(pixelRadiusToAperture(8, 800, 45), 12)
  })

  it('parallel basis returns the exact world radius — a true cylindrical tolerance, no depth involved', () => {
    const worldPerPixel = 0.01 // 1 cm per pixel
    const basis: ApertureBasis = { kind: 'parallel', worldPerPixel }
    expect(apertureForPixelRadius(basis, 8, 800)).toBeCloseTo(8 * worldPerPixel, 12)
  })

  it('parallel basis scales monotonically (and linearly) with pixel radius', () => {
    const basis: ApertureBasis = { kind: 'parallel', worldPerPixel: 0.02 }
    const small = apertureForPixelRadius(basis, 4, 800)
    const large = apertureForPixelRadius(basis, 16, 800)
    expect(large).toBeGreaterThan(small)
    expect(large).toBeCloseTo(4 * small, 12)
  })
})

describe('apertureModeFor', () => {
  it('perspective basis selects cone', () => {
    expect(apertureModeFor({ kind: 'perspective', fovYDeg: 45 })).toBe('cone')
  })

  it('parallel basis selects cylinder', () => {
    expect(apertureModeFor({ kind: 'parallel', worldPerPixel: 0.02 })).toBe('cylinder')
  })
})

// Tape Measure's "resize the model?" camera-follow fix (design tool-parity
// §3): rescale_document(factor) scales the whole model about the WORLD
// ORIGIN, so the camera must scale about that SAME pivot by the SAME factor
// to keep the view visually identical.
describe('scaleCameraAboutOrigin', () => {
  it('scales both eye and target by the factor about the origin', () => {
    const { eye, target } = scaleCameraAboutOrigin([4, -6, 8], [1, 2, 3], 2)
    expect(eye).toEqual([8, -12, 16])
    expect(target).toEqual([2, 4, 6])
  })

  it('preserves the eye→target direction and scales the distance by the factor', () => {
    const eye0: [number, number, number] = [10, 0, 5]
    const target0: [number, number, number] = [0, 0, 0]
    const factor = 3
    const { eye, target } = scaleCameraAboutOrigin(eye0, target0, factor)

    const distBefore = Math.hypot(eye0[0] - target0[0], eye0[1] - target0[1], eye0[2] - target0[2])
    const distAfter = Math.hypot(eye[0] - target[0], eye[1] - target[1], eye[2] - target[2])
    expect(distAfter).toBeCloseTo(distBefore * factor, 9)

    // Direction unchanged (same angular size at the new scale — the whole
    // point of scaling the camera in lockstep with the model).
    const dirBefore = [(eye0[0] - target0[0]) / distBefore, (eye0[1] - target0[1]) / distBefore, (eye0[2] - target0[2]) / distBefore]
    const dirAfter = [(eye[0] - target[0]) / distAfter, (eye[1] - target[1]) / distAfter, (eye[2] - target[2]) / distAfter]
    expect(dirAfter[0]).toBeCloseTo(dirBefore[0], 9)
    expect(dirAfter[1]).toBeCloseTo(dirBefore[1], 9)
    expect(dirAfter[2]).toBeCloseTo(dirBefore[2], 9)
  })

  it('a factor of 1 is a no-op', () => {
    const { eye, target } = scaleCameraAboutOrigin([3, -2, 7], [1, 1, 1], 1)
    expect(eye).toEqual([3, -2, 7])
    expect(target).toEqual([1, 1, 1])
  })

  it('a target already at the origin stays at the origin (the origin is a fixed point of the scale)', () => {
    const { eye, target } = scaleCameraAboutOrigin([5, 5, 5], [0, 0, 0], 4)
    expect(eye).toEqual([20, 20, 20])
    expect(target).toEqual([0, 0, 0])
  })
})

// Companion to scaleCameraAboutOrigin: the OTHER world-length view state
// (OrbitControls' orbit-distance clamp, the camera's clip planes) must scale
// by the same factor or the pose re-computed above gets clamped/clipped
// right back to the pre-rescale scale (delta-review Findings 1 & 2 — see
// `applyRescaleToView`'s doc comment in Viewport.tsx).
describe('scaleViewLimits', () => {
  it('scales near, far, minDistance, and maxDistance all by the same factor', () => {
    const limits = scaleViewLimits({ near: 0.01, far: 100, minDistance: 0.1, maxDistance: 50 }, 100)
    expect(limits).toEqual({ near: 1, far: 10000, minDistance: 10, maxDistance: 5000 })
  })

  it('a factor of 1 is a no-op', () => {
    const original = { near: 0.01, far: 100, minDistance: 0.1, maxDistance: 50 }
    expect(scaleViewLimits(original, 1)).toEqual(original)
  })

  it('a shrink factor (< 1) scales every bound DOWN, including near — a shrunk model must not stay near-clipped by the old (now comparatively huge) near plane', () => {
    const limits = scaleViewLimits({ near: 0.01, far: 100, minDistance: 0.1, maxDistance: 50 }, 0.01)
    expect(limits.near).toBeCloseTo(0.0001, 10)
    expect(limits.far).toBeCloseTo(1, 10)
    expect(limits.minDistance).toBeCloseTo(0.001, 10)
    expect(limits.maxDistance).toBeCloseTo(0.5, 10)
  })

  it('preserves the near < far and minDistance < maxDistance orderings (scaling by a positive factor cannot invert them)', () => {
    const before = { near: 0.01, far: 100, minDistance: 0.1, maxDistance: 50 }
    for (const factor of [100, 0.01, 1, 3.7]) {
      const after = scaleViewLimits(before, factor)
      expect(after.near).toBeLessThan(after.far)
      expect(after.minDistance).toBeLessThan(after.maxDistance)
    }
  })

  it('does not mutate the input', () => {
    const original = { near: 0.01, far: 100, minDistance: 0.1, maxDistance: 50 }
    const copy = { ...original }
    scaleViewLimits(original, 100)
    expect(original).toEqual(copy)
  })
})

// Zoom Extents' own view-limit resync (delta-review Finding 1): re-derives
// near/far/minDistance/maxDistance as RATIOS of the fit distance it just
// computed, rather than reusing whatever `scaleViewLimits` last left in
// place — a rescale scales those bounds, but undoing the rescale never
// restores them (camera/view state is outside undo by design), so they can
// be stuck at a stale scale no matter what Zoom Extents' OWN fit distance
// says the scene actually needs right now.
describe('zoomExtentsViewLimits', () => {
  it('a fit distance of exactly MOUNT_FIT_DISTANCE reproduces MOUNT_LIMITS exactly (the mount-time tuning is the ratio-1 fixed point)', () => {
    const limits = zoomExtentsViewLimits(MOUNT_FIT_DISTANCE)
    expect(limits.near).toBeCloseTo(MOUNT_LIMITS.near, 12)
    expect(limits.far).toBeCloseTo(MOUNT_LIMITS.far, 12)
    expect(limits.minDistance).toBeCloseTo(MOUNT_LIMITS.minDistance, 12)
    expect(limits.maxDistance).toBeCloseTo(MOUNT_LIMITS.maxDistance, 12)
  })

  it('scales all four bounds proportionally to the fit distance, exactly like scaleViewLimits', () => {
    const factor = 17.3
    const limits = zoomExtentsViewLimits(MOUNT_FIT_DISTANCE * factor)
    expect(limits).toEqual(scaleViewLimits(MOUNT_LIMITS, factor))
  })

  it('the refuters repro: a stale minDistance of 10 (0.1 mount value scaled 100x by a prior rescale) does not survive a resync at the true (small) post-undo fit distance', () => {
    // Mirrors the exact numbers from the delta-review repro: a 100x rescale
    // scales minDistance to 0.1 * 100 = 10; undoing it reverts the model to
    // a small bounding sphere whose fit distance is well under that stale
    // floor (the refuters measured ~2.51 for their scene).
    const staleMinDistance = MOUNT_LIMITS.minDistance * 100
    const trueFitDistance = 2.51
    expect(trueFitDistance).toBeLessThan(staleMinDistance)
    const resynced = zoomExtentsViewLimits(trueFitDistance)
    expect(resynced.minDistance).toBeLessThan(trueFitDistance)
    expect(resynced.maxDistance).toBeGreaterThan(trueFitDistance)
    expect(resynced.minDistance).not.toBeCloseTo(staleMinDistance, 0)
  })

  it('minDistance/maxDistance always bracket the fit distance itself, at any scale — the pose Zoom Extents is about to set can never be clamped by the limits it just derived', () => {
    for (const fitDistance of [0.001, 0.05, MOUNT_FIT_DISTANCE, 12, 500, 1e6]) {
      const limits = zoomExtentsViewLimits(fitDistance)
      expect(limits.minDistance).toBeLessThan(fitDistance)
      expect(limits.maxDistance).toBeGreaterThan(fitDistance)
      expect(limits.near).toBeLessThan(fitDistance)
      expect(limits.far).toBeGreaterThan(fitDistance)
    }
  })

  it('preserves the near < far and minDistance < maxDistance orderings at any fit distance', () => {
    for (const fitDistance of [0.001, 1, MOUNT_FIT_DISTANCE, 1000]) {
      const limits = zoomExtentsViewLimits(fitDistance)
      expect(limits.near).toBeLessThan(limits.far)
      expect(limits.minDistance).toBeLessThan(limits.maxDistance)
    }
  })
})
