import { describe, expect, it } from 'vitest'
import {
  ANGLE_STEP,
  FULL_TURN,
  axisFromPoints,
  axisInPlane,
  pointerAngle,
  referencePoint,
  revolveFrame,
  revolveProfileJson,
  revolveSegments,
  revolveSpecJson,
  snapAngle,
  type V3,
} from './revolveMath'
import { segmentsPerTurn } from './arcMath'

const XZ_PLANE = { point: [0, 0, 0] as V3, normal: [0, 1, 0] as V3 }

describe('revolveMath', () => {
  it('builds an axis from two points, refusing a zero-length one', () => {
    expect(axisFromPoints([0, 0, 0], [0, 0, 2])).toEqual({ point: [0, 0, 0], dir: [0, 0, 1] })
    expect(axisFromPoints([1, 1, 1], [1, 1, 1])).toBeNull()
  })

  it('accepts only axes lying in the profile plane', () => {
    expect(axisInPlane([0, 0, 0], [0, 0, 1], XZ_PLANE)).toBe(true)
    expect(axisInPlane([0, 0, 0], [0, 1, 0], XZ_PLANE)).toBe(false)
    expect(axisInPlane([0, 0.5, 0], [0, 0.5, 1], XZ_PLANE)).toBe(false)
    expect(axisInPlane([0, 1e-7, 0], [0, -1e-7, 1], XZ_PLANE)).toBe(true)
  })

  it('takes the profile mean as the reference, or the farthest vertex when that is on the axis', () => {
    const ring: V3[] = [[1, 0, 0], [2, 0, 0], [2, 0, 1], [1, 0, 1]]
    expect(referencePoint(ring, [0, 0, 0], [0, 0, 1])).toEqual([1.5, 0, 0.5])
    const symmetric: V3[] = [[-1, 0, 0], [1, 0, 0], [1, 0, 1], [-1, 0, 1]]
    const ref = referencePoint(symmetric, [0, 0, 0], [0, 0, 1])
    expect(Math.abs(ref![0])).toBe(1)
    expect(referencePoint([], [0, 0, 0], [0, 0, 1])).toBeNull()
  })

  it('measures the pointer angle right-handed about the axis from the profile', () => {
    // Axis Z through the origin, profile toward +X.
    const frame = revolveFrame([0, 0, 0], [0, 0, 1], [1, 0, 0.5])!
    expect(frame.g).toEqual([0, 0, 0.5])
    // Looking straight down -Z at the plane z = 0.5.
    const down = (x: number, y: number) => ({ origin: [x, y, 10] as V3, direction: [0, 0, -1] as V3 })
    expect(pointerAngle(frame, down(1, 0))).toBeCloseTo(FULL_TURN, 12) // on the profile itself: a full turn
    expect(pointerAngle(frame, down(0, 1))).toBeCloseTo(Math.PI / 2, 12)
    expect(pointerAngle(frame, down(-1, 0))).toBeCloseTo(Math.PI, 12)
    expect(pointerAngle(frame, down(0, -1))).toBeCloseTo((3 * Math.PI) / 2, 12)
    // A ray lying in the angle plane has no answer.
    expect(pointerAngle(frame, { origin: [5, 0, 0.5], direction: [-1, 0, 0] })).toBeNull()
    // A hit behind the eye still answers.
    expect(pointerAngle(frame, { origin: [0, 1, -10], direction: [0, 0, -1] })).toBeCloseTo(Math.PI / 2, 12)
  })

  it('snaps to 15 degree steps within 3 degrees only', () => {
    const deg = (d: number) => (d * Math.PI) / 180
    expect(snapAngle(deg(88))).toBeCloseTo(deg(90), 12)
    expect(snapAngle(deg(93))).toBeCloseTo(deg(90), 12)
    expect(snapAngle(deg(93.5))).toBeCloseTo(deg(93.5), 12)
    expect(snapAngle(deg(7))).toBeCloseTo(deg(7), 12)
    expect(snapAngle(ANGLE_STEP * 4)).toBeCloseTo(deg(60), 12)
  })

  it('snaps at the edges of its window and near a full turn', () => {
    const deg = (d: number) => (d * Math.PI) / 180
    expect(snapAngle(deg(93))).toBeCloseTo(deg(90), 12) // exactly 3 degrees out still snaps
    expect(snapAngle(deg(87.01))).toBeCloseTo(deg(90), 12)
    expect(snapAngle(deg(86.9))).toBeCloseTo(deg(86.9), 12)
    expect(snapAngle(deg(359.8))).toBeCloseTo(deg(360), 12)
    // The profile's own direction reads as a full turn, not a zero angle.
    expect(snapAngle(deg(0.2))).toBeCloseTo(deg(360), 12)
  })

  it('segments come from the drawing tools at the largest radius', () => {
    const ring: V3[] = [[0.1, 0, 0], [3, 0, 0], [3, 0, 1]]
    expect(revolveSegments(ring, [0, 0, 0], [0, 0, 1])).toBe(segmentsPerTurn(3))
    expect(revolveSegments([], [0, 0, 0], [0, 0, 1])).toBe(segmentsPerTurn(0))
  })

  it('writes the kernel JSON', () => {
    expect(JSON.parse(revolveSpecJson([0, 0, 0], [0, 0, 1], Math.PI, 24))).toEqual({
      axis_point: [0, 0, 0], axis: [0, 0, 1], angle: Math.PI, segments: 24,
    })
    expect(JSON.parse(revolveProfileJson({ kind: 'region', sketch: 3n, region: 9n }))).toEqual({
      region: { sketch: '3', region: '9' },
    })
    expect(JSON.parse(revolveProfileJson({ kind: 'face', object: 4n, face: 2n }))).toEqual({
      face: { object: '4', face: '2' },
    })
  })
})
