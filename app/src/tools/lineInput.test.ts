import { describe, it, expect } from 'vitest'
import { segmentLength, directionBetween, crossV3, dotV3, rehomePlaneNormal, type V3 } from './lineInput'

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

describe('crossV3', () => {
  it('X × Y = Z (right-handed)', () => {
    expect(crossV3([1, 0, 0], [0, 1, 0])).toEqual([0, 0, 1])
  })

  it('Y × Z = X', () => {
    expect(crossV3([0, 1, 0], [0, 0, 1])).toEqual([1, 0, 0])
  })

  it('is anticommutative', () => {
    const a: V3 = [1, 2, 3]
    const b: V3 = [4, -1, 2]
    const ab = crossV3(a, b)
    const ba = crossV3(b, a)
    expect(ba).toEqual([-ab[0], -ab[1], -ab[2]])
  })

  it('is zero for parallel vectors', () => {
    expect(crossV3([2, 0, 0], [5, 0, 0])).toEqual([0, 0, 0])
  })
})

describe('dotV3', () => {
  it('computes the dot product', () => {
    expect(dotV3([1, 2, 3], [4, 5, 6])).toBe(1 * 4 + 2 * 5 + 3 * 6)
  })

  it('is zero for perpendicular vectors', () => {
    expect(dotV3([1, 0, 0], [0, 1, 0])).toBe(0)
  })
})

/** Unit-length check with a generous tolerance for the arithmetic below. */
function expectUnit(v: V3): void {
  expect(Math.hypot(v[0], v[1], v[2])).toBeCloseTo(1, 9)
}

describe('rehomePlaneNormal', () => {
  it('spans the previous and new segment directions when they are not parallel', () => {
    const segDir: V3 = [1, 0, 0]
    const prevDir: V3 = [0, 0, 1]
    const normal = rehomePlaneNormal(segDir, prevDir, [0, -1, 0])
    expectUnit(normal)
    // The plane must contain BOTH directions — normal is perpendicular to each.
    expect(dotV3(normal, segDir)).toBeCloseTo(0, 9)
    expect(dotV3(normal, prevDir)).toBeCloseTo(0, 9)
    // The exact cross-product order this implementation uses.
    expect(normal).toEqual(crossV3(prevDir, segDir))
  })

  it('falls back to the view-facing plane when there is no previous segment', () => {
    const segDir: V3 = [0, 1, 0]
    const viewDir: V3 = [0, 0, -1] // looking straight down −Z
    const normal = rehomePlaneNormal(segDir, null, viewDir)
    expectUnit(normal)
    expect(dotV3(normal, segDir)).toBeCloseTo(0, 9)
    // normal is the (normalized) rejection of viewDir from segDir — since
    // viewDir is already ⊥ segDir here, normal is exactly viewDir.
    expect(normal[0]).toBeCloseTo(viewDir[0], 9)
    expect(normal[1]).toBeCloseTo(viewDir[1], 9)
    expect(normal[2]).toBeCloseTo(viewDir[2], 9)
  })

  it('falls back to the view-facing plane when the previous segment is parallel to the new one', () => {
    const segDir: V3 = [1, 0, 0]
    const prevDir: V3 = [-1, 0, 0] // parallel (opposite sign) — no plane spanned
    const viewDir: V3 = [0, 1, 0]
    const normal = rehomePlaneNormal(segDir, prevDir, viewDir)
    expectUnit(normal)
    expect(dotV3(normal, segDir)).toBeCloseTo(0, 9)
    // Falls all the way through to the view-facing branch, not the (unusable) span.
    expect(normal[0]).toBeCloseTo(0, 9)
    expect(normal[1]).toBeCloseTo(1, 9)
    expect(normal[2]).toBeCloseTo(0, 9)
  })

  it('the view-facing rejection removes exactly the along-segment component of viewDir', () => {
    const segDir: V3 = [1, 0, 0]
    const viewDir: V3 = [0.5, 0.5, 0] // 45°, half along the locked line
    const normal = rehomePlaneNormal(segDir, null, viewDir)
    expectUnit(normal)
    expect(dotV3(normal, segDir)).toBeCloseTo(0, 9)
    // viewDir rejected from X leaves only its Y component, normalized.
    expect(normal[0]).toBeCloseTo(0, 9)
    expect(normal[1]).toBeCloseTo(1, 9)
    expect(normal[2]).toBeCloseTo(0, 9)
  })

  it('never degenerates even when BOTH the previous segment and the view direction are parallel to the new segment', () => {
    const segDir: V3 = [1, 0, 0]
    const prevDir: V3 = [1, 0, 0] // parallel
    const viewDir: V3 = [-1, 0, 0] // ALSO parallel (camera sighting straight down the line)
    const normal = rehomePlaneNormal(segDir, prevDir, viewDir)
    expectUnit(normal)
    expect(Number.isFinite(normal[0])).toBe(true)
    expect(Number.isFinite(normal[1])).toBe(true)
    expect(Number.isFinite(normal[2])).toBe(true)
    expect(dotV3(normal, segDir)).toBeCloseTo(0, 9)
  })

  it('is deterministic (same inputs, same output) — no hidden randomness in the degenerate fallback', () => {
    const segDir: V3 = [0, 0, 1]
    const normal1 = rehomePlaneNormal(segDir, segDir, segDir)
    const normal2 = rehomePlaneNormal(segDir, segDir, segDir)
    expect(normal1).toEqual(normal2)
  })

  it('near-parallel (but not EXACTLY parallel) previous/new directions route to the stable view-facing fallback, not the numerically ill-conditioned cross-product plane (playtest-2 review finding C)', () => {
    const prevDir: V3 = [1, 0, 0]
    // Two "continue nearly straight ahead" directions — about 0.006° off
    // parallel, an angle no user could deliberately aim for — that differ
    // only in WHICH axis the tiny deviation happens to land on (Y vs Z).
    // That axis is not a meaningful design choice; it is exactly the kind
    // of imperceptible difference ordinary pixel/inference imprecision
    // produces.
    const segDirA = directionBetween([0, 0, 0], [1, 1e-4, 0]) as V3
    const segDirB = directionBetween([0, 0, 0], [1, 0, 1e-4]) as V3
    const viewDir: V3 = [0, 0, -1]

    const normalA = rehomePlaneNormal(segDirA, prevDir, viewDir)
    const normalB = rehomePlaneNormal(segDirB, prevDir, viewDir)

    // The re-homed plane must not swing wildly (here: all the way to
    // ORTHOGONAL, dot ~ 0) between two inputs that are, for any practical
    // purpose, the same "continue straight" gesture.
    expect(dotV3(normalA, normalB)).toBeGreaterThan(0.9)
  })
})
