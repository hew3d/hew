import { describe, expect, it } from 'vitest'
import {
  chamferFromAngle,
  clampBlendSegments,
  customArcSegments,
  clampCornerSegments,
  dihedralFromNormals,
  limitedSentence,
  parseProfileWidth,
  parseRadiusLimit,
  profileTooWideSentence,
  roundRadiusFromSetback,
  setbackFromPointer,
  setbackFromRoundRadius,
  shapeArcRadiusMax,
  type ProfileShapeJson,
  type V3,
} from './filletMath'

const near = (a: number, b: number, digits = 9): void => expect(a).toBeCloseTo(b, digits)

describe('round sizing', () => {
  it('a right-angle edge has radius equal to the setback', () => {
    near(roundRadiusFromSetback(0.01, Math.PI / 2), 0.01)
  })
  it('a shallower edge needs a bigger ball for the same setback', () => {
    // θ = 120°: tan 60° ≈ 1.732
    near(roundRadiusFromSetback(0.01, (2 * Math.PI) / 3), 0.01 * Math.sqrt(3))
    near(setbackFromRoundRadius(roundRadiusFromSetback(0.02, 1.1), 1.1), 0.02)
  })
  it('reads the dihedral from two outward normals', () => {
    near(dihedralFromNormals([0, 0, 1], [0, 1, 0]), Math.PI / 2)
    near(dihedralFromNormals([0, 0, 1], [0, 0.5, Math.sqrt(0.75)]), Math.PI - Math.PI / 6)
  })
})

describe('chamfer by distance and angle', () => {
  it('45 degrees on a right-angle edge makes equal setbacks', () => {
    const r = chamferFromAngle(0.01, Math.PI / 4, Math.PI / 2)
    expect(r.ok).toBe(true)
    if (r.ok) near(r.setbackB, 0.01)
  })
  it('a steeper angle reaches less along face B', () => {
    const r = chamferFromAngle(0.01, Math.PI / 6, Math.PI / 2)
    expect(r.ok && r.setbackB < 0.01).toBe(true)
  })
  it('refuses an angle that cannot close the triangle', () => {
    expect(chamferFromAngle(0.01, Math.PI / 2, Math.PI / 2).ok).toBe(false)
    expect(chamferFromAngle(0.01, 0, Math.PI / 2).ok).toBe(false)
    expect(chamferFromAngle(0.01, -0.1, Math.PI / 2).ok).toBe(false)
    expect(chamferFromAngle(0.01, 2, Math.PI / 2).ok).toBe(false)
  })
})

describe('segment clamps', () => {
  it('keeps a round in the kernel range and a 2D corner in 1..=1024', () => {
    expect(clampBlendSegments(1)).toBe(2)
    expect(clampBlendSegments(500)).toBe(96)
    expect(clampCornerSegments(0)).toBe(1)
    expect(clampCornerSegments(5000)).toBe(1024)
    expect(clampCornerSegments(6.4)).toBe(6)
  })
})

describe('setback from the pointer', () => {
  // A box's top-front edge: along +x at y = 0, z = 1. Face A is the top
  // (normal +z, extends toward +y), face B the front (normal -y, extends
  // toward -z).
  const edge: V3 = [0, 0, 1]
  const dir: V3 = [1, 0, 0]
  const top = { normal: [0, 0, 1] as V3, inside: [0.5, 0.5, 1] as V3 }
  const front = { normal: [0, -1, 0] as V3, inside: [0.5, 0, 0.5] as V3 }

  it('reads the distance on the top face when the ray comes down onto it', () => {
    const r = setbackFromPointer([0.5, 0.2, 5], [0, 0, -1], edge, dir, top, front)
    expect(r).not.toBeNull()
    expect(r?.side).toBe('A')
    near(r?.d ?? NaN, 0.2)
  })

  it('reads the distance on the front face when the ray hits it', () => {
    const r = setbackFromPointer([0.5, -5, 0.7], [0, 1, 0], edge, dir, top, front)
    expect(r?.side).toBe('B')
    near(r?.d ?? NaN, 0.3)
  })

  it('ignores the plane whose hit lies off its own face', () => {
    // From above and in front, aimed down and toward +y: the ray meets the
    // front plane above the box (off face B) and the top plane inside face A.
    const o: V3 = [0.5, -1, 3]
    const d: V3 = [0, 0.6, -0.8]
    const r = setbackFromPointer(o, d, edge, dir, top, front)
    expect(r?.side).toBe('A')
    near(r?.d ?? NaN, 0.5)
  })

  it('returns null when the pointer is off both faces', () => {
    // Beyond the edge, outside the solid: the ray meets the top plane at y < 0.
    expect(setbackFromPointer([0.5, -0.5, 5], [0, 0, -1], edge, dir, top, front)).toBeNull()
    // A ray parallel to both planes' normals' complement.
    expect(setbackFromPointer([0.5, 0.5, 5], [1, 0, 0], edge, dir, top, front)).toBeNull()
  })
})

describe('limit copy', () => {
  const fmt = (m: number): string => `${Math.round(m * 1000)} mm`
  it('names the feature that limits a size', () => {
    expect(limitedSentence('Radius', 0.014, 'adjacent_face', fmt)).toBe(
      'Radius limited to 14 mm by the adjacent face.',
    )
    expect(limitedSentence('Radius', 0.2, 'hole', fmt)).toBe('Radius limited to 200 mm by the hole in this face.')
    expect(limitedSentence('Radius', 0.2, 'something_new', fmt)).toBe(
      'Radius limited to 200 mm by the shape of the model.',
    )
  })
  it('names a size that fits, not called a limit', () => {
    expect(limitedSentence('Radius', 0.002, 'known_fit', fmt)).toBe('Radius set to 2 mm, a size that fits here.')
    expect(limitedSentence('Distance', 0.002, 'known_fit', fmt)).toBe('Distance set to 2 mm, a size that fits here.')
  })
  it('says what a profile needs', () => {
    expect(profileTooWideSentence(0.04, 0.025, fmt)).toBe('This profile needs 40 mm; 25 mm is available.')
  })
  it('reads the numbers back out of the kernel messages', () => {
    expect(parseRadiusLimit('RADIUS_TOO_LARGE: radius limited to 0.0123 by short_edge')).toEqual({
      max: 0.0123,
      limitedBy: 'short_edge',
    })
    expect(parseRadiusLimit('nothing here')).toBeNull()
    expect(parseProfileWidth('PROFILE_TOO_WIDE: the profile needs 0.04; 0.025 is available')).toEqual({
      needed: 0.04,
      available: 0.025,
    })
  })
})

describe('profile shapes', () => {
  const ogee: ProfileShapeJson = {
    b: 1,
    segs: [
      { kind: 'arc', to: [0.5, 0.5], center: [1, 0.5], ccw: false },
      { kind: 'arc', to: [0, 1], center: [0, 0.5], ccw: true },
    ],
  }
  it('finds the largest normalized arc radius', () => {
    expect(shapeArcRadiusMax(ogee)).toBeCloseTo(0.5, 9)
    expect(shapeArcRadiusMax({ b: 1, segs: [{ kind: 'line', to: [0, 1] }] })).toBe(1)
  })
  it('scales the facet count with the drawn width', () => {
    // 24 per turn at a small radius: 6 per quarter, floor 2.
    expect(customArcSegments(ogee, 0.01, () => 24)).toBe(6)
    expect(customArcSegments(ogee, 0.01, () => 4)).toBe(2)
    expect(customArcSegments(ogee, 1, (r) => (r === 0.5 ? 96 : 0))).toBe(24)
  })
})
