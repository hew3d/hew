import { describe, it, expect } from 'vitest'
import {
  worldToNdc,
  ndcToPagePixel,
  worldToPagePixel,
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
