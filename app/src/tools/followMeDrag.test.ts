/**
 * followMeDrag specs — the drag-to-partial-sweep arc-length mapping.
 *
 * These pin shape (seam walk direction/order, arc-length totals) and the
 * ray→arc-length query, not kernel-legality — see the module doc comment for
 * why this is a best-effort preview aid, not a mirrored legality claim.
 */
import { describe, it, expect } from 'vitest'
import {
  seamWalk,
  nearestOnWalk,
  subWalkTo,
  pointAtArcLength,
  type PathPolyline,
  type PathSegment,
  type PlaneDef,
  type Vec3,
} from './followMeDrag'

function plane(point: Vec3, normal: Vec3): PlaneDef {
  return { point, normal }
}

/** The open L: (0,0,0) → (2,0,0) → (2,3,0). */
function lPath(): PathPolyline {
  const segments: PathSegment[] = [
    { a: [0, 0, 0], b: [2, 0, 0], curve: null },
    { a: [2, 0, 0], b: [2, 3, 0], curve: null },
  ]
  return { segments, closed: false }
}

/** The closed 2x2 square from the corner-seam kernel specs. */
function squarePath(): PathPolyline {
  const pts: Vec3[] = [
    [0, 0, 0],
    [2, 0, 0],
    [2, 2, 0],
    [0, 2, 0],
  ]
  const segments: PathSegment[] = pts.map((p, i) => ({
    a: p,
    b: pts[(i + 1) % pts.length],
    curve: null,
  }))
  return { segments, closed: true }
}

describe('followMeDrag — open path seam walk', () => {
  it('walks from the end square to the plane, total length = the path length', () => {
    const walk = seamWalk(lPath(), plane([0, 0, 0], [0, 1, 0]), [1, 1, 0])
    expect(walk).not.toBeNull()
    expect(walk!.points[0]).toEqual([0, 0, 0])
    expect(walk!.total).toBeCloseTo(5, 9) // 2 + 3
    expect(walk!.cumulative[walk!.cumulative.length - 1]).toBeCloseTo(5, 9)
  })

  it('walks from the OTHER end when the plane is square there instead', () => {
    const walk = seamWalk(lPath(), plane([2, 3, 0], [1, 0, 0]), [1, 1, 0])
    expect(walk!.points[0]).toEqual([2, 3, 0])
    expect(walk!.points[walk!.points.length - 1]).toEqual([0, 0, 0])
  })
})

describe('followMeDrag — closed path seam walk', () => {
  it('starts the lap at the point nearest the profile centroid and returns to it', () => {
    // A profile centroid near (0,0,0) (the plane touches that corner too, but
    // the seam approximation only consults the centroid, not the plane fold).
    const walk = seamWalk(squarePath(), plane([0, 0, 0], [1, 0, 0]), [0.01, 0.01, 0])
    expect(walk).not.toBeNull()
    expect(walk!.points[0]).toEqual(walk!.points[walk!.points.length - 1])
    // Nearest point to (0.01, 0.01, 0) on the square's boundary is (0.01, 0, 0)
    // — the projection onto the bottom edge, not the (0,0,0) corner itself.
    expect(walk!.points[0][0]).toBeCloseTo(0.01, 6)
    expect(walk!.points[0][1]).toBeCloseTo(0, 6)
    expect(walk!.total).toBeCloseTo(8, 9) // the whole 2x2 square's perimeter
  })

  it('leaves the seam ALONG +n, walking backward when the natural chord runs against it', () => {
    // Centroid near (2, 0, 0): the natural segment there runs (0,0,0)->(2,0,0),
    // i.e. +x. A plane normal of -x forces the walk backward from that point.
    const walk = seamWalk(squarePath(), plane([2, 0, 0], [-1, 0, 0]), [1.99, 0.01, 0])
    // Walking backward from near (2,0,0) means the very next point is (0,0,0).
    expect(walk!.points[1]).toEqual([0, 0, 0])
  })
})

describe('followMeDrag — nearestOnWalk', () => {
  it('maps a ray straight down onto a point on the path to that arc length', () => {
    const walk = seamWalk(lPath(), plane([0, 0, 0], [0, 1, 0]), [1, 1, 0])!
    // A ray from above, straight down, aimed at (1, 0, 0) — 1m along the first leg.
    const { point, arcLen } = nearestOnWalk(walk, [1, 0, 5], [0, 0, -1])
    expect(point[0]).toBeCloseTo(1, 6)
    expect(point[1]).toBeCloseTo(0, 6)
    expect(arcLen).toBeCloseTo(1, 6)
  })

  it('clamps to the nearer end when the ray aims past it', () => {
    const walk = seamWalk(lPath(), plane([0, 0, 0], [0, 1, 0]), [1, 1, 0])!
    const { arcLen } = nearestOnWalk(walk, [2, 10, 5], [0, 0, -1])
    expect(arcLen).toBeCloseTo(5, 6) // the far end (2,3,0)
  })

  it('finds the mid-run point on the SECOND leg, past the elbow', () => {
    const walk = seamWalk(lPath(), plane([0, 0, 0], [0, 1, 0]), [1, 1, 0])!
    const { point, arcLen } = nearestOnWalk(walk, [2, 1.5, 5], [0, 0, -1])
    expect(point).toEqual([2, 1.5, 0])
    expect(arcLen).toBeCloseTo(2 + 1.5, 6)
  })
})

describe('followMeDrag — subWalkTo / pointAtArcLength', () => {
  it('returns the seam alone at arc length 0', () => {
    const walk = seamWalk(lPath(), plane([0, 0, 0], [0, 1, 0]), [1, 1, 0])!
    expect(subWalkTo(walk, 0)).toEqual([[0, 0, 0]])
  })

  it('interpolates mid-segment and clamps beyond the total', () => {
    const walk = seamWalk(lPath(), plane([0, 0, 0], [0, 1, 0]), [1, 1, 0])!
    expect(pointAtArcLength(walk, 1)).toEqual([1, 0, 0])
    expect(pointAtArcLength(walk, 99)).toEqual([2, 3, 0])
    expect(pointAtArcLength(walk, -5)).toEqual([0, 0, 0])
  })

  it('a sub-walk to the full total reproduces every vertex', () => {
    const walk = seamWalk(lPath(), plane([0, 0, 0], [0, 1, 0]), [1, 1, 0])!
    expect(subWalkTo(walk, walk.total)).toEqual(walk.points)
  })
})
