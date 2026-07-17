/**
 * offsetMath unit tests — signed cursor→boundary distance and the preview
 * wire-format decoder.
 */
import { describe, it, expect } from 'vitest'
import { signedOffsetDistance, decodeOffsetLoops, loopToSegmentPairs, boundaryContainsEdge } from './offsetMath'

/** A unit square on the ground plane, flat xyz. */
const SQUARE = [0, 0, 0, 2, 0, 0, 2, 2, 0, 0, 2, 0]
const UP: [number, number, number] = [0, 0, 1]

describe('signedOffsetDistance', () => {
  it('is negative inside the loop, measuring to the nearest edge', () => {
    expect(signedOffsetDistance([1, 0.5, 0], SQUARE, UP)).toBeCloseTo(-0.5)
    expect(signedOffsetDistance([1, 1, 0], SQUARE, UP)).toBeCloseTo(-1)
  })

  it('is positive outside the loop', () => {
    expect(signedOffsetDistance([3, 1, 0], SQUARE, UP)).toBeCloseTo(1)
    // Beyond a corner: distance to the corner vertex.
    expect(signedOffsetDistance([3, 3, 0], SQUARE, UP)).toBeCloseTo(Math.SQRT2)
  })

  it('rejects degenerate boundaries', () => {
    expect(signedOffsetDistance([0, 0, 0], [0, 0, 0, 1, 0, 0], UP)).toBeNull()
  })

  it('works on a non-ground plane', () => {
    // The same square stood up in the XZ plane (normal +Y).
    const wall = [0, 0, 0, 2, 0, 0, 2, 0, 2, 0, 0, 2]
    expect(signedOffsetDistance([1, 0, 0.5], wall, [0, 1, 0])).toBeCloseTo(-0.5)
    expect(signedOffsetDistance([1, 0, 3], wall, [0, 1, 0])).toBeCloseTo(1)
  })
})

describe('decodeOffsetLoops', () => {
  it('decodes the [count, n, xyz…] wire format', () => {
    const wire = [2, 3, /* loop 0 */ 0, 0, 0, 1, 0, 0, 0, 1, 0, 4, /* loop 1 */ 5, 5, 0, 6, 5, 0, 6, 6, 0, 5, 6, 0]
    const loops = decodeOffsetLoops(wire)
    expect(loops).toHaveLength(2)
    expect(loops[0]).toEqual([0, 0, 0, 1, 0, 0, 0, 1, 0])
    expect(loops[1]).toHaveLength(12)
  })

  it('tolerates truncated input by returning the clean prefix', () => {
    expect(decodeOffsetLoops([2, 3, 0, 0, 0, 1, 0, 0, 0, 1, 0, 4, 5, 5])).toHaveLength(1)
    expect(decodeOffsetLoops([])).toHaveLength(0)
  })
})

describe('loopToSegmentPairs', () => {
  it('closes the loop with segment pairs', () => {
    const pairs = loopToSegmentPairs([0, 0, 0, 1, 0, 0, 0, 1, 0])
    expect(pairs).toHaveLength(18)
    // Last segment closes back to the first vertex.
    expect(Array.from(pairs.slice(12))).toEqual([0, 1, 0, 0, 0, 0])
  })
})

describe('boundaryContainsEdge', () => {
  // A 2×2 ground square, f32 like region_boundary returns.
  const square = new Float32Array([0, 0, 0, 2, 0, 0, 2, 2, 0, 0, 2, 0])

  it('matches an interior consecutive pair, in either direction', () => {
    expect(boundaryContainsEdge(square, new Float64Array([0, 0, 0, 2, 0, 0]))).toBe(true)
    expect(boundaryContainsEdge(square, new Float64Array([2, 0, 0, 0, 0, 0]))).toBe(true)
    expect(boundaryContainsEdge(square, new Float64Array([2, 0, 0, 2, 2, 0]))).toBe(true)
  })

  it('matches the implicit closing segment (last vertex back to first)', () => {
    expect(boundaryContainsEdge(square, new Float64Array([0, 2, 0, 0, 0, 0]))).toBe(true)
    expect(boundaryContainsEdge(square, new Float64Array([0, 0, 0, 0, 2, 0]))).toBe(true)
  })

  it('rejects a diagonal (both endpoints on the loop but not consecutive)', () => {
    expect(boundaryContainsEdge(square, new Float64Array([0, 0, 0, 2, 2, 0]))).toBe(false)
  })

  it('rejects a segment off the loop entirely', () => {
    expect(boundaryContainsEdge(square, new Float64Array([5, 5, 0, 6, 5, 0]))).toBe(false)
  })

  it('tolerates f32 rounding of the boundary against f64 endpoints', () => {
    const v = 0.05000000074505806 // Math.fround(0.05)
    const tiny = new Float32Array([0, 0, 0, 0.05, 0, 0, 0.05, 0.05, 0, 0, 0.05, 0])
    expect(boundaryContainsEdge(tiny, new Float64Array([0, 0, 0, 0.05, 0, 0]))).toBe(true)
    expect(boundaryContainsEdge(tiny, new Float64Array([v, 0, 0, v, 0.05, 0]))).toBe(true)
  })

  it('tolerates f32 rounding far from the origin (magnitude-aware tolerance)', () => {
    // At x ≈ 500 m the f32 round-trip error (~0.5 ulp ≈ 6e-8·|coord|) is
    // ~1.2e-5 — past any fixed 1e-5 absolute tolerance, which would regress
    // edge clicks on far-from-origin sketches to the very miss this
    // matcher exists to fix.
    const x0 = 500.05
    const x1 = 502.05
    const far = new Float32Array([x0, 0, 0, x1, 0, 0, x1, 2, 0, x0, 2, 0])
    expect(boundaryContainsEdge(far, new Float64Array([x0, 0, 0, x1, 0, 0]))).toBe(true)
    expect(boundaryContainsEdge(far, new Float64Array([x1, 0, 0, x0, 0, 0]))).toBe(true)
    // …while a genuinely off-boundary segment at the same range still refuses.
    expect(boundaryContainsEdge(far, new Float64Array([x0 + 0.01, 0, 0, x1 + 0.01, 0, 0]))).toBe(false)
    expect(boundaryContainsEdge(far, new Float64Array([x0, 0.01, 0, x1, 0.01, 0]))).toBe(false)
  })

  it('degenerate inputs are misses, not throws', () => {
    expect(boundaryContainsEdge(new Float32Array([]), new Float64Array([0, 0, 0, 1, 0, 0]))).toBe(false)
    expect(boundaryContainsEdge(square, new Float64Array([0, 0, 0]))).toBe(false)
  })
})
