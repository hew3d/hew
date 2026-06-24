import { describe, it, expect } from 'vitest'
import { segmentLength, directionBetween } from './lineInput'

describe('segmentLength', () => {
  it('computes Euclidean distance in 3D', () => {
    expect(segmentLength([0, 0, 0], [3, 4, 0])).toBeCloseTo(5, 9)
    expect(segmentLength([1, 1, 1], [1, 1, 1])).toBe(0)
    expect(segmentLength([0, 0, 0], [1, 2, 2])).toBeCloseTo(3, 9)
  })

  it('is symmetric', () => {
    const a: [number, number, number] = [1, 2, 3]
    const b: [number, number, number] = [4, 0, -1]
    expect(segmentLength(a, b)).toBeCloseTo(segmentLength(b, a), 9)
  })
})

describe('directionBetween', () => {
  it('returns the unit vector from a to b', () => {
    const dir = directionBetween([0, 0, 0], [5, 0, 0])
    expect(dir).not.toBeNull()
    expect(dir![0]).toBeCloseTo(1, 9)
    expect(dir![1]).toBeCloseTo(0, 9)
    expect(dir![2]).toBeCloseTo(0, 9)
  })

  it('normalizes a diagonal vector', () => {
    const dir = directionBetween([0, 0, 0], [1, 1, 0])
    expect(dir).not.toBeNull()
    const len = Math.hypot(dir![0], dir![1], dir![2])
    expect(len).toBeCloseTo(1, 9)
    expect(dir![0]).toBeCloseTo(Math.SQRT1_2, 9)
    expect(dir![1]).toBeCloseTo(Math.SQRT1_2, 9)
  })

  it('returns null for coincident points', () => {
    expect(directionBetween([1, 2, 3], [1, 2, 3])).toBeNull()
  })

  it('returns null when points are within epsilon', () => {
    expect(directionBetween([0, 0, 0], [1e-10, 0, 0])).toBeNull()
  })

  it('respects a custom epsilon', () => {
    expect(directionBetween([0, 0, 0], [0.5, 0, 0], 1)).toBeNull()
    expect(directionBetween([0, 0, 0], [2, 0, 0], 1)).not.toBeNull()
  })
})
