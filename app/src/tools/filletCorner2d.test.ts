import { describe, expect, it } from 'vitest'
import {
  chamferedCornerPolyline,
  cornerAngle,
  cornerMaxSetback,
  cornerRadiusFromSetback,
  cornerSetbackFromPointer,
  roundedCornerPolyline,
  type Corner2d,
} from './filletCorner2d'

// A right-angle corner at the origin of a 2 x 1 rectangle: edges along +x
// (length 2) and +y (length 1).
const corner: Corner2d = { c: [0, 0, 0], p: [2, 0, 0], q: [0, 1, 0] }
const up: [number, number, number] = [0, 0, 1]
const near = (a: number, b: number): void => expect(a).toBeCloseTo(b, 9)

describe('2D corner geometry', () => {
  it('measures the angle and the room', () => {
    near(cornerAngle(corner) ?? NaN, Math.PI / 2)
    near(cornerMaxSetback(corner), 1)
    expect(cornerAngle({ c: [0, 0, 0], p: [1, 0, 0], q: [-1, 0, 0] })).toBeNull()
  })

  it('reads the setback from the pointer on the nearer edge', () => {
    near(cornerSetbackFromPointer(corner, [0.4, 0.1, 0]), 0.4)
    near(cornerSetbackFromPointer(corner, [0.05, 0.3, 0]), 0.3)
    near(cornerSetbackFromPointer(corner, [-1, -1, 0]), 0)
    near(cornerRadiusFromSetback(0.2, Math.PI / 2), 0.2)
  })

  it('rounds with tangent points on both edges and an arc between', () => {
    const line = roundedCornerPolyline(corner, 0.25, 4)
    expect(line).not.toBeNull()
    const pts = line ?? []
    expect(pts).toHaveLength(5)
    near(pts[0][0], 0.25)
    near(pts[0][1], 0)
    expect(pts[4][0]).toBeCloseTo(0, 9)
    expect(pts[4][1]).toBeCloseTo(0.25, 9)
    // Every interior point is the radius from the arc's center (0.25, 0.25).
    for (const p of pts.slice(1, 4)) near(Math.hypot(p[0] - 0.25, p[1] - 0.25), 0.25)
  })

  it('refuses a radius that does not fit the shorter edge', () => {
    expect(roundedCornerPolyline(corner, 1.5, 4)).toBeNull()
    expect(roundedCornerPolyline(corner, 0, 4)).toBeNull()
  })

  it('chamfers with setback a along +y and b along +x for this corner', () => {
    const seg = chamferedCornerPolyline(corner, 0.3, 0.1, up)
    expect(seg).not.toBeNull()
    expect(seg?.[0]).toEqual([0, 0.3, 0])
    expect(seg?.[1]).toEqual([0.1, 0, 0])
    expect(chamferedCornerPolyline(corner, 2, 0.1, up)).toBeNull()
  })
})
