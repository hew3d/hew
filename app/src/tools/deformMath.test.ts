import { describe, expect, it } from 'vitest'
import {
  TwistTracker,
  FOUR_PI,
  TWO_PI,
  MIN_TAPER,
  angleAround,
  bendAngle,
  bendCrossAxis,
  boxSpecJson,
  crossAxes,
  clampToLimit,
  frameForFace,
  hitMovingPlane,
  movingCenter,
  pathSpecJson,
  radiusFromAxis,
  rayBoxFace,
  taperRatio,
  type V3,
} from './deformMath'

const MIN: V3 = [-0.05, -0.05, 0]
const MAX: V3 = [0.05, 0.05, 1]
const ray = (o: V3, d: V3) => ({ origin: o, direction: d })

describe('deformMath', () => {
  it('finds the box face a ray enters through', () => {
    const top = rayBoxFace(ray([0, 0, 5], [0, 0, -1]), MIN, MAX)!
    expect(top.axisIndex).toBe(2)
    expect(top.side).toBe(1)
    expect(top.normal).toEqual([0, 0, 1])
    const front = rayBoxFace(ray([0, -5, 0.5], [0, 1, 0]), MIN, MAX)!
    expect(front.axisIndex).toBe(1)
    expect(front.side).toBe(0)
    expect(front.normal).toEqual([0, -1, 0])
    const diagonal = rayBoxFace(ray([3, 3, 3], [-1, -1, -1]), MIN, MAX)!
    expect([0, 1]).toContain(diagonal.axisIndex) // the ray meets the x and y faces last, at the tie
    expect(rayBoxFace(ray([1, 1, 5], [0, 0, -1]), MIN, MAX)).toBeNull() // miss
    expect(rayBoxFace(ray([0, 0, 0.5], [0, 0, -1]), MIN, MAX)).toBeNull() // starts inside
  })

  it('builds the frame from the picked moving face', () => {
    const top = rayBoxFace(ray([0, 0, 5], [0, 0, -1]), MIN, MAX)!
    const f = frameForFace(MIN, MAX, top)
    expect(f.origin).toEqual([0, 0, 0]) // the bottom face center is the fixed end
    expect(f.axis).toEqual([0, 0, 1])
    expect(f.length).toBe(1)
    expect(Math.abs(f.u[0] * f.axis[0] + f.u[1] * f.axis[1] + f.u[2] * f.axis[2])).toBe(0)
    expect(movingCenter(f)).toEqual([0, 0, 1])
    // Picking the bottom instead flips the axis.
    const bottom = rayBoxFace(ray([0, 0, -5], [0, 0, 1]), MIN, MAX)!
    const g = frameForFace(MIN, MAX, bottom)
    expect(g.origin).toEqual([0, 0, 1])
    expect(g.axis).toEqual([0, 0, -1])
  })

  it('tracks a twist past a half turn without jumping', () => {
    const top = rayBoxFace(ray([0, 0, 5], [0, 0, -1]), MIN, MAX)!
    const f = frameForFace(MIN, MAX, top)
    const c = movingCenter(f)
    const tracker = new TwistTracker()
    // Sweep the pointer counterclockwise (about +Z) in 30 degree steps.
    let last = 0
    for (let k = 0; k <= 9; k++) {
      const a = (k * Math.PI) / 6
      const p: V3 = [c[0] + Math.cos(a), c[1] + Math.sin(a), c[2]]
      last = tracker.update(angleAround(f, c, p))
    }
    expect(last).toBeCloseTo((9 * Math.PI) / 6, 12) // 270 degrees, not -90
    expect(tracker.update(null)).toBeCloseTo(last, 12)
    // The total is bounded by the kernel's 4π.
    for (let k = 0; k < 100; k++) {
      const a = ((10 + k) * Math.PI) / 6
      tracker.update(angleAround(f, c, [c[0] + Math.cos(a), c[1] + Math.sin(a), c[2]]))
    }
    expect(tracker.total).toBeCloseTo(FOUR_PI, 12)
  })

  it('has no angle at the axis itself', () => {
    const top = rayBoxFace(ray([0, 0, 5], [0, 0, -1]), MIN, MAX)!
    const f = frameForFace(MIN, MAX, top)
    const c = movingCenter(f)
    expect(angleAround(f, c, c)).toBeNull()
  })

  it('reads the taper ratio against the grab radius, bounded', () => {
    const top = rayBoxFace(ray([0, 0, 5], [0, 0, -1]), MIN, MAX)!
    const f = frameForFace(MIN, MAX, top)
    const c = movingCenter(f)
    expect(radiusFromAxis(f, c, [c[0] + 0.03, c[1] + 0.04, c[2]])).toBeCloseTo(0.05, 12)
    expect(taperRatio(0.025, 0.05)).toBeCloseTo(0.5, 12)
    expect(taperRatio(0, 0.05)).toBe(MIN_TAPER)
    expect(taperRatio(1, 0.05)).toBe(20)
    expect(taperRatio(1, 0)).toBe(1)
  })

  it('bends by twice the drag along u over the length, bounded', () => {
    expect(bendAngle([0.25, 0, 0], [1, 0, 0], 1)).toBeCloseTo(0.5, 12)
    expect(bendAngle([-0.25, 0, 0], [1, 0, 0], 1)).toBeCloseTo(-0.5, 12)
    expect(bendAngle([100, 0, 0], [1, 0, 0], 1)).toBe(TWO_PI)
    expect(bendCrossAxis([0.1, 0.9, 0], 2)).toEqual([0, 1, 0])
    expect(bendCrossAxis([0.9, 0.1, 0.3], 2)).toEqual([1, 0, 0])
  })

  it('clamps to the kernel limit on the identity side', () => {
    expect(clampToLimit('twist', 1, 2)).toEqual({ amount: 1, clamped: false })
    const t = clampToLimit('twist', 3, 2)
    expect(t.clamped).toBe(true)
    expect(t.amount).toBeLessThan(2)
    expect(t.amount).toBeCloseTo(2, 5)
    const neg = clampToLimit('bend', -3, -2)
    expect(neg.amount).toBeGreaterThan(-2)
    const taper = clampToLimit('taper', 0.1, 0.3)
    expect(taper.clamped).toBe(true)
    expect(taper.amount).toBeGreaterThan(0.3)
    expect(taper.amount).toBeCloseTo(0.3, 5)
    expect(clampToLimit('taper', 5, 3).amount).toBeLessThan(3)
  })

  it('hits the moving face plane, also behind the eye', () => {
    const top = rayBoxFace(ray([0, 0, 5], [0, 0, -1]), MIN, MAX)!
    const f = frameForFace(MIN, MAX, top)
    expect(hitMovingPlane(f, ray([0.2, 0.1, 5], [0, 0, -1]))).toEqual([0.2, 0.1, 1])
    expect(hitMovingPlane(f, ray([0.2, 0.1, 0], [0, 0, 1]))).toEqual([0.2, 0.1, 1])
    expect(hitMovingPlane(f, ray([0, 0, 5], [1, 0, 0]))).toBeNull()
  })

  it('writes the kernel JSON', () => {
    const top = rayBoxFace(ray([0, 0, 5], [0, 0, -1]), MIN, MAX)!
    const f = frameForFace(MIN, MAX, top)
    const twist = JSON.parse(boxSpecJson('twist', f, Math.PI / 2, 8))
    expect(twist).toMatchObject({ mode: 'twist', angle: Math.PI / 2, stations: 8 })
    expect(twist.frame).toEqual({ origin: [0, 0, 0], axis: [0, 0, 1], u: [1, 0, 0], length: 1 })
    const taper = JSON.parse(boxSpecJson('taper', f, 0.6))
    expect(taper.ratio).toBe(0.6)
    expect(taper.angle).toBeUndefined()
    expect(taper.stations).toBeUndefined()
    const path = JSON.parse(pathSpecJson({ from: [0, 0, 0], to: [0, 0, 1] }, [[0, 0, 0], [1, 0, 0]], false, 4))
    expect(path).toEqual({
      mode: 'bend_to_path', base_from: [0, 0, 0], base_to: [0, 0, 1],
      path: [[0, 0, 0], [1, 0, 0]], closed: false, stations_per_segment: 4,
    })
  })
  it('unwraps a clockwise sweep past a half turn too, bounded at -4π', () => {
    const top = rayBoxFace(ray([0, 0, 5], [0, 0, -1]), MIN, MAX)!
    const f = frameForFace(MIN, MAX, top)
    const c = movingCenter(f)
    const tracker = new TwistTracker()
    let last = 0
    for (let k = 0; k <= 9; k++) {
      const a = (-k * Math.PI) / 6
      last = tracker.update(angleAround(f, c, [c[0] + Math.cos(a), c[1] + Math.sin(a), c[2]]))
    }
    expect(last).toBeCloseTo((-9 * Math.PI) / 6, 12) // -270 degrees, not +90
    for (let k = 0; k < 100; k++) {
      const a = (-(10 + k) * Math.PI) / 6
      tracker.update(angleAround(f, c, [c[0] + Math.cos(a), c[1] + Math.sin(a), c[2]]))
    }
    expect(tracker.total).toBeCloseTo(-FOUR_PI, 12)
  })

  it('the cross axes are the two world axes perpendicular to the face axis', () => {
    expect(crossAxes(2)).toEqual([[1, 0, 0], [0, 1, 0]])
    expect(crossAxes(0)).toEqual([[0, 1, 0], [0, 0, 1]])
  })

  it('a bend with no drag falls back to the first cross axis, and a tie keeps it', () => {
    expect(bendCrossAxis([0, 0, 0], 2)).toEqual([1, 0, 0])
    expect(bendCrossAxis([0.5, 0.5, 0], 2)).toEqual([1, 0, 0])
    expect(bendCrossAxis([0, -0.9, 0], 2)).toEqual([0, 1, 0])
  })

  it('a bend is bounded at -2π too, and a zero length bends nothing', () => {
    expect(bendAngle([-100, 0, 0], [1, 0, 0], 1)).toBe(-TWO_PI)
    expect(bendAngle([1, 0, 0], [1, 0, 0], 0)).toBe(0)
  })

  it('the limit clamp keeps the sign of the amount on each mode', () => {
    expect(clampToLimit('twist', -3, -2).amount).toBeGreaterThan(-2)
    expect(clampToLimit('twist', -3, -2).amount).toBeLessThan(0)
    expect(clampToLimit('bend', 3, 2).amount).toBeGreaterThan(0)
    expect(clampToLimit('bend', -1, -2)).toEqual({ amount: -1, clamped: false })
    expect(clampToLimit('taper', 5, 3).amount).toBeGreaterThan(1)
    expect(clampToLimit('taper', 0.1, 0.3).amount).toBeLessThan(1)
  })
})
