import { describe, it, expect } from 'vitest'
import {
  rectangleCorners,
  projectRayOntoAxis,
  parseKernelErrorCode,
  kernelErrorMessage,
  pointInPolygonXY,
  polygonAreaXY,
} from './geoHelpers'

describe('rectangleCorners', () => {
  it('returns four corners from two diagonal corners', () => {
    const corners = rectangleCorners([0, 0], [2, 3])
    expect(corners).toHaveLength(4)
    expect(corners[0]).toEqual([0, 0, 0])
    expect(corners[1]).toEqual([2, 0, 0])
    expect(corners[2]).toEqual([2, 3, 0])
    expect(corners[3]).toEqual([0, 3, 0])
  })

  it('all corners have z=0 (ground plane)', () => {
    const corners = rectangleCorners([-1, -2], [3, 4])
    for (const c of corners) {
      expect(c[2]).toBe(0)
    }
  })

  it('handles negative coordinates', () => {
    const corners = rectangleCorners([-3, -4], [-1, -1])
    expect(corners[0]).toEqual([-3, -4, 0])
    expect(corners[2]).toEqual([-1, -1, 0])
  })

  it('winding: corner sequence covers all four corners of the bounding rect', () => {
    const [[x0, y0], [x1, y1], [x2, y2], [x3, y3]] =
      rectangleCorners([1, 2], [4, 5]).map(([x, y]) => [x, y])
    // The four (x,y) coords should cover both input x values and both input y values
    const xs = new Set([x0, x1, x2, x3])
    const ys = new Set([y0, y1, y2, y3])
    expect(xs).toEqual(new Set([1, 4]))
    expect(ys).toEqual(new Set([2, 5]))
  })
})

describe('projectRayOntoAxis', () => {
  it('returns positive distance when ray points toward axis ahead of anchor', () => {
    // Axis: vertical (+Z), anchor at origin
    // Ray from (1, 0, 3), pointing in -X direction (perpendicular to axis)
    // Closest point on axis to ray: (0, 0, 3), so t = 3 (3 units up the +Z axis)
    const t = projectRayOntoAxis(
      [1, 0, 3],   // rayOrigin
      [-1, 0, 0],  // rayDir (unit, pointing -X)
      [0, 0, 0],   // anchor
      [0, 0, 1],   // direction (up)
    )
    expect(t).toBeCloseTo(3)
  })

  it('returns negative distance when ray is behind anchor along axis', () => {
    // Axis: +Z from anchor at (0,0,0)
    // Ray from (1, 0, -5), pointing in -X direction
    // Closest point on axis: (0, 0, -5), t = -5
    const t = projectRayOntoAxis(
      [1, 0, -5],
      [-1, 0, 0],
      [0, 0, 0],
      [0, 0, 1],
    )
    expect(t).toBeCloseTo(-5)
  })

  it('returns 0 when ray is parallel to axis (degenerate)', () => {
    // Both ray and axis point in +Z — d^2 ≈ 1, denom ≈ 0
    const t = projectRayOntoAxis(
      [1, 0, 0],
      [0, 0, 1],
      [0, 0, 0],
      [0, 0, 1],
    )
    expect(t).toBe(0)
  })

  it('handles axis along X and ray perpendicular', () => {
    // Axis: +X from anchor (0,0,0)
    // Ray from (3, 1, 0), pointing in -Y direction → closest on axis at (3,0,0)
    const t = projectRayOntoAxis(
      [3, 1, 0],
      [0, -1, 0],
      [0, 0, 0],
      [1, 0, 0],
    )
    expect(t).toBeCloseTo(3)
  })
})

describe('parseKernelErrorCode', () => {
  it('parses a CODE: message format', () => {
    const code = parseKernelErrorCode(new Error('WouldVanish: face would be removed'))
    expect(code).toBe('WouldVanish')
  })

  it('parses multi-word codes', () => {
    expect(parseKernelErrorCode(new Error('NonManifoldResult: edge shared by 3+ faces'))).toBe(
      'NonManifoldResult',
    )
  })

  it('returns null when format does not match', () => {
    expect(parseKernelErrorCode(new Error('something went wrong'))).toBeNull()
    expect(parseKernelErrorCode(new Error(''))).toBeNull()
    expect(parseKernelErrorCode('plain string')).toBeNull()
  })

  it('handles non-Error objects', () => {
    expect(parseKernelErrorCode('WouldVanish: bad things')).toBe('WouldVanish')
  })
})

describe('kernelErrorMessage', () => {
  it('returns a human-readable description for known codes', () => {
    const msg = kernelErrorMessage('WouldVanish', 'face 42 would be removed')
    expect(msg).toContain("Can't remove all material")
  })

  it('falls back to raw code+message for unknown codes', () => {
    const msg = kernelErrorMessage('SomeUnknownCode', 'raw detail')
    expect(msg).toBe('SomeUnknownCode: raw detail')
  })
})

// Helper to build a Float32Array from flat [x,y,0] triples
function makePolygon(coords: [number, number][]): Float32Array {
  const arr = new Float32Array(coords.length * 3)
  for (let i = 0; i < coords.length; i++) {
    arr[i * 3] = coords[i][0]
    arr[i * 3 + 1] = coords[i][1]
    arr[i * 3 + 2] = 0
  }
  return arr
}

describe('polygonAreaXY', () => {
  it('computes area of a unit square', () => {
    const square = makePolygon([[0, 0], [1, 0], [1, 1], [0, 1]])
    expect(polygonAreaXY(square)).toBeCloseTo(1.0)
  })

  it('computes area of a 4x4 square', () => {
    const square = makePolygon([[0, 0], [4, 0], [4, 4], [0, 4]])
    expect(polygonAreaXY(square)).toBeCloseTo(16.0)
  })

  it('computes area of a 2x2 square (inner region)', () => {
    const square = makePolygon([[1, 1], [3, 1], [3, 3], [1, 3]])
    expect(polygonAreaXY(square)).toBeCloseTo(4.0)
  })

  it('returns unsigned area regardless of winding order', () => {
    // CCW winding
    const ccw = makePolygon([[0, 0], [1, 0], [1, 1], [0, 1]])
    // CW winding (reversed)
    const cw = makePolygon([[0, 1], [1, 1], [1, 0], [0, 0]])
    expect(polygonAreaXY(ccw)).toBeCloseTo(polygonAreaXY(cw))
  })

  it('returns 0 for fewer than 3 vertices', () => {
    const tooFew = makePolygon([[0, 0], [1, 0]])
    expect(polygonAreaXY(tooFew)).toBe(0)
  })

  it('computes area of a right triangle', () => {
    const triangle = makePolygon([[0, 0], [2, 0], [0, 2]])
    // Area = 0.5 * base * height = 0.5 * 2 * 2 = 2
    expect(polygonAreaXY(triangle)).toBeCloseTo(2.0)
  })

  it('inner region area is smaller than outer region area (nested rectangles)', () => {
    const outer = makePolygon([[0, 0], [4, 0], [4, 4], [0, 4]])
    const inner = makePolygon([[1, 1], [3, 1], [3, 3], [1, 3]])
    expect(polygonAreaXY(inner)).toBeLessThan(polygonAreaXY(outer))
  })
})

describe('pointInPolygonXY', () => {
  // Unit square: (0,0), (1,0), (1,1), (0,1)
  const unitSquare = makePolygon([[0, 0], [1, 0], [1, 1], [0, 1]])

  it('returns true for a point clearly inside', () => {
    expect(pointInPolygonXY(0.5, 0.5, unitSquare)).toBe(true)
  })

  it('returns false for a point clearly outside', () => {
    expect(pointInPolygonXY(2, 2, unitSquare)).toBe(false)
    expect(pointInPolygonXY(-1, 0.5, unitSquare)).toBe(false)
  })

  it('handles a point at the centroid of a larger rectangle', () => {
    const rect = makePolygon([[-2, -3], [2, -3], [2, 3], [-2, 3]])
    expect(pointInPolygonXY(0, 0, rect)).toBe(true)
    expect(pointInPolygonXY(3, 0, rect)).toBe(false)
  })

  it('returns false for fewer than 3 vertices', () => {
    const tooFew = makePolygon([[0, 0], [1, 0]])
    expect(pointInPolygonXY(0.5, 0, tooFew)).toBe(false)
  })

  it('works for a triangle', () => {
    const triangle = makePolygon([[0, 0], [4, 0], [2, 4]])
    expect(pointInPolygonXY(2, 1, triangle)).toBe(true)
    expect(pointInPolygonXY(2, 5, triangle)).toBe(false)
    expect(pointInPolygonXY(-1, 1, triangle)).toBe(false)
  })

  it('handles negative coordinate polygons', () => {
    const negRect = makePolygon([[-3, -3], [-1, -3], [-1, -1], [-3, -1]])
    expect(pointInPolygonXY(-2, -2, negRect)).toBe(true)
    expect(pointInPolygonXY(0, 0, negRect)).toBe(false)
  })
})
