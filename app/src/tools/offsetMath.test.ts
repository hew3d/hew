/**
 * offsetMath unit tests — signed cursor→boundary distance and the preview
 * wire-format decoder.
 */
import { describe, it, expect } from 'vitest'
import { signedOffsetDistance, decodeOffsetLoops, loopToSegmentPairs } from './offsetMath'

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
