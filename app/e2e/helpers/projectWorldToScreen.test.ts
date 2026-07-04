import { describe, it, expect } from 'vitest'
import {
  worldToNdc,
  ndcToPagePixel,
  worldToPagePixel,
  buildViewProjection,
  PINNED_CAMERA,
  type Mat4,
} from './projectWorldToScreen'

// Identity view-projection: world coords pass straight through as NDC.
const IDENTITY: Mat4 = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]

describe('worldToNdc', () => {
  it('passes a point through an identity matrix unchanged (w=1)', () => {
    const ndc = worldToNdc({ x: 0.25, y: -0.5, z: 0.75 }, IDENTITY)
    expect(ndc).not.toBeNull()
    expect(ndc!.x).toBeCloseTo(0.25, 12)
    expect(ndc!.y).toBeCloseTo(-0.5, 12)
    expect(ndc!.z).toBeCloseTo(0.75, 12)
    expect(ndc!.w).toBeCloseTo(1, 12)
  })

  it('applies the translation column (e[12..15]) to a (x,y,z,1) point', () => {
    // Identity rotation/scale + a translation of (1, 2, 3) in the last column.
    const translated: Mat4 = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 1, 2, 3, 1]
    const ndc = worldToNdc({ x: 10, y: 20, z: 30 }, translated)
    expect(ndc!.x).toBeCloseTo(11, 12)
    expect(ndc!.y).toBeCloseTo(22, 12)
    expect(ndc!.z).toBeCloseTo(33, 12)
  })

  it('divides by a non-unit clip-w (perspective divide)', () => {
    // Put 2 into the w-row of the z column so clipW = 2*z; with z=2 → w=4.
    const persp: Mat4 = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 2, 0, 0, 0, 0]
    const ndc = worldToNdc({ x: 8, y: 4, z: 2 }, persp)
    expect(ndc!.w).toBeCloseTo(4, 12)
    expect(ndc!.x).toBeCloseTo(8 / 4, 12)
    expect(ndc!.y).toBeCloseTo(4 / 4, 12)
  })

  it('returns null when clip-w collapses to ~0 (undefined projection)', () => {
    const degenerate: Mat4 = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0]
    expect(worldToNdc({ x: 1, y: 1, z: 1 }, degenerate)).toBeNull()
  })
})

describe('ndcToPagePixel', () => {
  const rect = { left: 100, top: 50, width: 800, height: 600 }

  it('maps NDC center to the canvas center in page coords', () => {
    expect(ndcToPagePixel({ x: 0, y: 0 }, rect)).toEqual({ x: 500, y: 350 })
  })

  it('flips Y: NDC top (+1) → smaller page-y, NDC bottom (-1) → larger', () => {
    const top = ndcToPagePixel({ x: 0, y: 1 }, rect)
    const bottom = ndcToPagePixel({ x: 0, y: -1 }, rect)
    expect(top).toEqual({ x: 500, y: 50 }) // top edge
    expect(bottom).toEqual({ x: 500, y: 650 }) // bottom edge
  })

  it('maps NDC corners to canvas corners', () => {
    expect(ndcToPagePixel({ x: -1, y: 1 }, rect)).toEqual({ x: 100, y: 50 })
    expect(ndcToPagePixel({ x: 1, y: -1 }, rect)).toEqual({ x: 900, y: 650 })
  })
})

describe('worldToPagePixel', () => {
  const rect = { left: 0, top: 0, width: 1000, height: 1000 }

  it('composes projection + pixel mapping for an in-front point', () => {
    // Identity → world (0.5, 0.5) is NDC (0.5, 0.5) → page (750, 250).
    expect(worldToPagePixel({ x: 0.5, y: 0.5, z: 0 }, IDENTITY, rect)).toEqual({
      x: 750,
      y: 250,
    })
  })

  it('returns null for a point behind the camera (w <= 0)', () => {
    // Negative clip-w via a -1 in the w-row constant term.
    const behind: Mat4 = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, -1]
    expect(worldToPagePixel({ x: 0, y: 0, z: 0 }, behind, rect)).toBeNull()
  })
})

describe('buildViewProjection', () => {
  const rect = { left: 0, top: 0, width: 800, height: 600 }
  const vp = buildViewProjection(PINNED_CAMERA, rect.width / rect.height)

  it('projects the camera target to the exact canvas center', () => {
    const px = worldToPagePixel(PINNED_CAMERA.target, vp, rect)
    expect(px).not.toBeNull()
    expect(px!.x).toBeCloseTo(400, 6)
    expect(px!.y).toBeCloseTo(300, 6)
  })

  it('respects world-up: a point above the target lands higher on screen', () => {
    // PINNED_CAMERA's up is +Z, so +Z from the target must reduce page-y.
    const above = worldToPagePixel({ x: 0, y: 0, z: 1 }, vp, rect)
    expect(above).not.toBeNull()
    expect(above!.y).toBeLessThan(300)
    expect(above!.x).toBeCloseTo(400, 3) // straight above the target: no x drift
  })

  it('returns null (behind camera) for a point past the eye', () => {
    // Double the eye offset from the target: squarely behind the camera.
    const p = PINNED_CAMERA.position
    const behind = { x: 2 * p.x, y: 2 * p.y, z: 2 * p.z }
    expect(worldToPagePixel(behind, vp, rect)).toBeNull()
  })

  it('is scale-consistent with the perspective divide: nearer points subtend more pixels', () => {
    // Two points 1m apart on the ground: one pair near the camera, one far.
    const d = (a: { x: number; y: number }, b: { x: number; y: number }): number =>
      Math.hypot(a.x - b.x, a.y - b.y)
    const near1 = worldToPagePixel({ x: 2, y: 2, z: 0 }, vp, rect)!
    const near2 = worldToPagePixel({ x: 3, y: 2, z: 0 }, vp, rect)!
    const far1 = worldToPagePixel({ x: -8, y: -8, z: 0 }, vp, rect)!
    const far2 = worldToPagePixel({ x: -7, y: -8, z: 0 }, vp, rect)!
    expect(d(near1, near2)).toBeGreaterThan(d(far1, far2))
  })

  it('fov widens the frustum: larger fov pulls off-center points toward the center', () => {
    const wide = buildViewProjection({ ...PINNED_CAMERA, fovDeg: 80 }, rect.width / rect.height)
    const p = { x: 2, y: 0, z: 0 }
    const narrowPx = worldToPagePixel(p, vp, rect)!
    const widePx = worldToPagePixel(p, wide, rect)!
    const center = { x: 400, y: 300 }
    const dist = (a: { x: number; y: number }): number => Math.hypot(a.x - center.x, a.y - center.y)
    expect(dist(widePx)).toBeLessThan(dist(narrowPx))
  })

  it('throws on a degenerate camera basis (up parallel to the view direction)', () => {
    expect(() =>
      buildViewProjection(
        {
          position: { x: 0, y: 0, z: 10 },
          target: { x: 0, y: 0, z: 0 },
          up: { x: 0, y: 0, z: 1 }, // parallel to the look axis
          fovDeg: 50,
          near: 0.1,
          far: 1000,
        },
        1,
      ),
    ).toThrow('degenerate')
  })
})
