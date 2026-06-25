import { describe, it, expect } from 'vitest'
import {
  rectangleCorners,
  faceRectangleCorners,
  projectRayOntoAxis,
  rayPlaneIntersect,
  parseKernelErrorCode,
  kernelErrorMessage,
  pointInPolygonXY,
  polygonAreaXY,
  circlePolygonGround,
  circlePolygonFace,
} from './geoHelpers'
import type { V3 } from './geoHelpers'

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

describe('rayPlaneIntersect', () => {
  it('intersects a ray straight down onto the ground plane (Z=0)', () => {
    const p = rayPlaneIntersect([0, 0, 5], [0, 0, -1], [0, 0, 0], [0, 0, 1])
    expect(p).not.toBeNull()
    expect(p?.[0]).toBeCloseTo(0)
    expect(p?.[1]).toBeCloseTo(0)
    expect(p?.[2]).toBeCloseTo(0)
  })

  it('intersects an angled ray onto an arbitrary plane', () => {
    // Plane through (1,0,0) with normal +X (a vertical YZ plane at x=1)
    const p = rayPlaneIntersect([0, 0, 0], [1, 1, 0], [1, 0, 0], [1, 0, 0])
    expect(p).not.toBeNull()
    expect(p?.[0]).toBeCloseTo(1)
    expect(p?.[1]).toBeCloseTo(1)
    expect(p?.[2]).toBeCloseTo(0)
  })

  it('returns null when the ray is parallel to the plane', () => {
    const p = rayPlaneIntersect([0, 0, 5], [1, 0, 0], [0, 0, 0], [0, 0, 1])
    expect(p).toBeNull()
  })

  it('returns null when the intersection is behind the ray origin', () => {
    const p = rayPlaneIntersect([0, 0, -5], [0, 0, -1], [0, 0, 0], [0, 0, 1])
    expect(p).toBeNull()
  })

  it('does not require a normalized ray direction', () => {
    const p = rayPlaneIntersect([0, 0, 5], [0, 0, -10], [0, 0, 0], [0, 0, 1])
    expect(p).not.toBeNull()
    expect(p?.[0]).toBeCloseTo(0)
    expect(p?.[1]).toBeCloseTo(0)
    expect(p?.[2]).toBeCloseTo(0)
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

describe('faceRectangleCorners', () => {
  /** Dot product of two V3s */
  function dot(a: V3, b: V3): number {
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
  }

  /** Signed area of triangle (a,b,c) projected onto the plane with given normal.
   *  Positive means CCW from the +normal side. */
  function signedTriArea(a: V3, b: V3, c: V3, n: V3): number {
    const ab: V3 = [b[0]-a[0], b[1]-a[1], b[2]-a[2]]
    const ac: V3 = [c[0]-a[0], c[1]-a[1], c[2]-a[2]]
    // cross(ab, ac) · n gives signed area * 2
    const cx = ab[1]*ac[2] - ab[2]*ac[1]
    const cy = ab[2]*ac[0] - ab[0]*ac[2]
    const cz = ab[0]*ac[1] - ab[1]*ac[0]
    return cx*n[0] + cy*n[1] + cz*n[2]
  }

  it('(a) +Z normal matches ground rectangleCorners layout and is CCW from above', () => {
    const anchor: V3 = [0, 0, 0]
    const cursor: V3 = [2, 3, 0]
    const normal: V3 = [0, 0, 1]
    const corners = faceRectangleCorners(anchor, cursor, normal)
    expect(corners).not.toBeNull()
    const [a, b, c, d] = corners!

    // Should cover the expected bounding extents like rectangleCorners([0,0],[2,3])
    const groundCorners = rectangleCorners([0, 0], [2, 3])
    const groundXYs = groundCorners.map(([x, y]) => [x, y]).sort().toString()
    const faceXYs = [a, b, c, d].map(([x, y]) => [x, y]).sort().toString()
    expect(faceXYs).toEqual(groundXYs)

    // CCW from +Z: the shoelace sign should be positive for all consecutive triples
    expect(signedTriArea(a, b, c, normal)).toBeGreaterThan(0)
    expect(signedTriArea(a, b, d, normal)).toBeGreaterThan(0)
  })

  it('(b) +X normal: all 4 corners are coplanar with the face plane', () => {
    const normal: V3 = [1, 0, 0]
    const anchor: V3 = [5, 0, 0]   // on the x=5 plane
    const cursor: V3 = [5, 2, 3]   // also on x=5 plane
    const corners = faceRectangleCorners(anchor, cursor, normal)
    expect(corners).not.toBeNull()
    for (const corner of corners!) {
      // dot(corner - anchor, normal) should be ≈ 0
      const diff: V3 = [corner[0]-anchor[0], corner[1]-anchor[1], corner[2]-anchor[2]]
      expect(Math.abs(dot(diff, normal))).toBeLessThan(1e-10)
    }
    // Also check CCW
    const [a, b, c] = corners!
    expect(signedTriArea(a, b, c, normal)).toBeGreaterThan(0)
  })

  it('(c) degenerate: cursor === anchor returns null', () => {
    const anchor: V3 = [1, 2, 3]
    const cursor: V3 = [1, 2, 3]
    const normal: V3 = [0, 0, 1]
    expect(faceRectangleCorners(anchor, cursor, normal)).toBeNull()
  })

  it('(c) degenerate: zero extent on u axis returns null', () => {
    // normal = [0,0,1], u = [1,0,0], v = [0,1,0]
    // cursor directly above anchor (only v-extent, no u-extent)
    const anchor: V3 = [0, 0, 0]
    const cursor: V3 = [0, 3, 0]   // du=0, dv=3
    const normal: V3 = [0, 0, 1]
    expect(faceRectangleCorners(anchor, cursor, normal)).toBeNull()
  })

  it('(c) degenerate: zero extent on v axis returns null', () => {
    // cursor only moves in u direction
    const anchor: V3 = [0, 0, 0]
    const cursor: V3 = [2, 0, 0]   // du=2, dv=0
    const normal: V3 = [0, 0, 1]
    expect(faceRectangleCorners(anchor, cursor, normal)).toBeNull()
  })

  it('oblique normal: 4 corners are coplanar with the face plane', () => {
    // normal = normalize([1,1,1])
    const s = 1 / Math.sqrt(3)
    const normal: V3 = [s, s, s]
    const anchor: V3 = [0, 0, 0]
    // cursor: move along some direction in the plane
    // In-plane direction: any vector with dot(v, normal)=0, e.g. [1,-1,0] (normalized)
    const t = 1 / Math.sqrt(2)
    const cursor: V3 = [anchor[0] + t + t, anchor[1] - t + t, anchor[2] + 0]
    const corners = faceRectangleCorners(anchor, cursor, normal)
    if (corners === null) return // degenerate ok for this generic test
    for (const corner of corners) {
      const diff: V3 = [corner[0]-anchor[0], corner[1]-anchor[1], corner[2]-anchor[2]]
      expect(Math.abs(diff[0]*normal[0] + diff[1]*normal[1] + diff[2]*normal[2])).toBeLessThan(1e-9)
    }
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

describe('circlePolygonGround', () => {
  it('returns N vertices, all at the given radius from center, all z=0', () => {
    const verts = circlePolygonGround([1, 2], [4, 2], 24) // radius 3
    expect(verts).toHaveLength(24)
    for (const [x, y, z] of verts) {
      expect(Math.hypot(x - 1, y - 2)).toBeCloseTo(3)
      expect(z).toBe(0)
    }
  })

  it('vertex 0 is exactly the rim point', () => {
    const verts = circlePolygonGround([0, 0], [5, 0], 24)
    expect(verts[0]).toEqual([5, 0, 0])
  })

  it('forms a closed loop (last vertex connects back near vertex 0 via equal angular spacing)', () => {
    const verts = circlePolygonGround([0, 0], [1, 0], 8)
    expect(verts).toHaveLength(8)
    // 8-gon: vertex 4 should be diametrically opposite vertex 0.
    expect(verts[4][0]).toBeCloseTo(-1)
    expect(verts[4][1]).toBeCloseTo(0)
  })

  it('winding is counter-clockwise viewed from +Z', () => {
    const verts = circlePolygonGround([0, 0], [1, 0], 4)
    // Square-ish 4-gon starting at +X: should proceed toward +Y next (CCW).
    expect(verts[1][0]).toBeCloseTo(0)
    expect(verts[1][1]).toBeCloseTo(1)
  })

  it('returns [] for a degenerate (near-zero) radius', () => {
    expect(circlePolygonGround([1, 1], [1, 1], 24)).toEqual([])
    expect(circlePolygonGround([1, 1], [1 + 1e-9, 1], 24)).toEqual([])
  })
})

describe('circlePolygonFace', () => {
  const normalZ: V3 = [0, 0, 1]

  it('returns N vertices, all at the given radius from center, all on the plane', () => {
    const center: V3 = [1, 1, 5]
    const rim: V3 = [4, 1, 5]
    const verts = circlePolygonFace(center, rim, normalZ, 24)
    expect(verts).not.toBeNull()
    expect(verts).toHaveLength(24)
    for (const [x, y, z] of verts!) {
      expect(Math.hypot(x - 1, y - 1)).toBeCloseTo(3)
      expect(z).toBeCloseTo(5) // on-plane (z=5 plane, normal +Z)
    }
  })

  it('vertex 0 is the rim point when rim already lies on the plane', () => {
    const verts = circlePolygonFace([0, 0, 2], [5, 0, 2], normalZ, 24)
    expect(verts).not.toBeNull()
    expect(verts![0][0]).toBeCloseTo(5)
    expect(verts![0][1]).toBeCloseTo(0)
    expect(verts![0][2]).toBeCloseTo(2)
  })

  it('matches circlePolygonGround for a Z=0 plane with +Z normal', () => {
    const ground = circlePolygonGround([2, 3], [6, 3], 24)
    const face = circlePolygonFace([2, 3, 0], [6, 3, 0], normalZ, 24)
    expect(face).not.toBeNull()
    for (let i = 0; i < 24; i++) {
      expect(face![i][0]).toBeCloseTo(ground[i][0])
      expect(face![i][1]).toBeCloseTo(ground[i][1])
      expect(face![i][2]).toBeCloseTo(ground[i][2])
    }
  })

  it('works on an arbitrary tilted plane (vertices stay coplanar, on the normal)', () => {
    // Plane through origin with normal (1,1,1)/sqrt(3)
    const n = 1 / Math.sqrt(3)
    const normal: V3 = [n, n, n]
    const center: V3 = [0, 0, 0]
    // Pick a rim point that is actually on the plane: must satisfy dot(p, normal) = 0
    const rim: V3 = [1, -1, 0] // dot = n - n + 0 = 0, on-plane
    const verts = circlePolygonFace(center, rim, normal, 12)
    expect(verts).not.toBeNull()
    for (const v of verts!) {
      const dot = v[0] * normal[0] + v[1] * normal[1] + v[2] * normal[2]
      expect(dot).toBeCloseTo(0, 6) // coplanar with the plane through origin
      expect(Math.hypot(v[0], v[1], v[2])).toBeCloseTo(Math.hypot(1, -1, 0))
    }
  })

  it('returns null for a degenerate (near-zero) radius', () => {
    expect(circlePolygonFace([1, 1, 1], [1, 1, 1], normalZ, 24)).toBeNull()
  })
})
