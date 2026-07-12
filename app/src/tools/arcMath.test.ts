import { describe, it, expect } from 'vitest'
import {
  MIN_SEGMENTS_PER_TURN,
  MAX_SEGMENTS_PER_TURN,
  DRAW_SAGITTA_TOL_M,
  segmentsPerTurn,
  ARC_MIN_SAGITTA_M,
  arcFromChord,
  arcPolyline,
  arcPolylineOnPlane,
  arcSegmentCount,
  chordSagitta,
  type Vec2,
  type Vec3,
} from './arcMath'

const A: Vec2 = [0, 0]
const B: Vec2 = [2, 0]

describe('chordSagitta', () => {
  it('is positive on the CCW side of A→B and negative on the CW side', () => {
    // Chord along +X: CCW perpendicular is +Y.
    expect(chordSagitta(A, B, [1, 0.5])).toBeCloseTo(0.5, 12)
    expect(chordSagitta(A, B, [1, -0.5])).toBeCloseTo(-0.5, 12)
  })

  it('measures only the perpendicular component (position along the chord is irrelevant)', () => {
    expect(chordSagitta(A, B, [0.25, 0.5])).toBeCloseTo(0.5, 12)
    expect(chordSagitta(A, B, [1.9, 0.5])).toBeCloseTo(0.5, 12)
  })

  it('returns null for a degenerate (zero-length) chord', () => {
    expect(chordSagitta(A, [0, 0], [1, 1])).toBeNull()
  })
})

describe('arcFromChord — center and radius', () => {
  it('semicircle: s = h gives r = h and center at the chord midpoint', () => {
    const arc = arcFromChord(A, B, 1)!
    expect(arc.radius).toBeCloseTo(1, 12)
    expect(arc.center[0]).toBeCloseTo(1, 12)
    expect(arc.center[1]).toBeCloseTo(0, 12)
    expect(Math.abs(arc.sweep)).toBeCloseTo(Math.PI, 12)
  })

  it('minor arc: known values for h=1, s=0.5 (r = 1.25, center at (1, −0.75))', () => {
    // r = (h² + s²) / (2·|s|) = (1 + 0.25) / 1 = 1.25
    const arc = arcFromChord(A, B, 0.5)!
    expect(arc.radius).toBeCloseTo(1.25, 12)
    expect(arc.center[0]).toBeCloseTo(1, 12)
    expect(arc.center[1]).toBeCloseTo(-0.75, 12) // opposite side from the bulge
    expect(Math.abs(arc.sweep)).toBeCloseTo(2 * Math.asin(1 / 1.25), 12)
  })

  it('center is equidistant (r) from A, B, and the bulge apex, for both bulge signs', () => {
    for (const s of [0.5, -0.5, 1.7, -1.7]) {
      const arc = arcFromChord(A, B, s)!
      const dist = (p: Vec2) => Math.hypot(p[0] - arc.center[0], p[1] - arc.center[1])
      const apex: Vec2 = [1, s] // midpoint + s along the CCW perpendicular (+Y)
      expect(dist(A)).toBeCloseTo(arc.radius, 12)
      expect(dist(B)).toBeCloseTo(arc.radius, 12)
      expect(dist(apex)).toBeCloseTo(arc.radius, 12)
    }
  })

  it('sweep passes through the bulge point: the mid-sweep vertex is the apex', () => {
    for (const s of [0.5, -0.5, 1, -1, 1.7]) {
      const arc = arcFromChord(A, B, s)!
      const midAngle = arc.startAngle + arc.sweep / 2
      const mid: Vec2 = [
        arc.center[0] + arc.radius * Math.cos(midAngle),
        arc.center[1] + arc.radius * Math.sin(midAngle),
      ]
      expect(mid[0]).toBeCloseTo(1, 10)
      expect(mid[1]).toBeCloseTo(s, 10) // apex = M + s·perpCCW = (1, s)
    }
  })

  it('sweep sign is opposite the sagitta sign (arc runs around the far-side center)', () => {
    expect(arcFromChord(A, B, 0.5)!.sweep).toBeLessThan(0)
    expect(arcFromChord(A, B, -0.5)!.sweep).toBeGreaterThan(0)
  })

  it('major arc: |s| > h gives sweep beyond π (but below 2π)', () => {
    const arc = arcFromChord(A, B, 1.8)!
    expect(Math.abs(arc.sweep)).toBeGreaterThan(Math.PI)
    expect(Math.abs(arc.sweep)).toBeLessThan(2 * Math.PI)
  })

  it('returns null for a flat arc (|s| below tolerance) and a zero chord', () => {
    expect(arcFromChord(A, B, 0)).toBeNull()
    expect(arcFromChord(A, B, ARC_MIN_SAGITTA_M / 2)).toBeNull()
    expect(arcFromChord(A, [0, 0], 0.5)).toBeNull()
  })
})

describe('segmentsPerTurn (docs/design/true-curves.md §6)', () => {
  it('small radii take the floor: 24 segments per turn', () => {
    expect(segmentsPerTurn(0.02)).toBe(MIN_SEGMENTS_PER_TURN)
    expect(segmentsPerTurn(0.05)).toBe(MIN_SEGMENTS_PER_TURN)
  })

  it('large radii cap at 96 segments per turn', () => {
    expect(segmentsPerTurn(1.0)).toBe(MAX_SEGMENTS_PER_TURN)
    expect(segmentsPerTurn(100)).toBe(MAX_SEGMENTS_PER_TURN)
  })

  it('mid radii adapt, rounded up to a multiple of 4 (quadrant vertices)', () => {
    const n = segmentsPerTurn(0.5)
    expect(n % 4).toBe(0)
    expect(n).toBeGreaterThan(MIN_SEGMENTS_PER_TURN)
    expect(n).toBeLessThan(MAX_SEGMENTS_PER_TURN)
    // The count honors the sagitta budget wherever it is not clamped.
    const sagitta = 0.5 * (1 - Math.cos(Math.PI / n))
    expect(sagitta).toBeLessThanOrEqual(DRAW_SAGITTA_TOL_M)
  })

  it('degenerate radii take the floor rather than exploding', () => {
    expect(segmentsPerTurn(0)).toBe(MIN_SEGMENTS_PER_TURN)
    expect(segmentsPerTurn(Number.NaN)).toBe(MIN_SEGMENTS_PER_TURN)
    expect(segmentsPerTurn(-1)).toBe(MIN_SEGMENTS_PER_TURN)
  })
})

describe('arcSegmentCount', () => {
  it('is the per-turn density scaled by the sweep fraction', () => {
    // Small radius: 24 per turn -> a quarter turn gets 6.
    expect(arcSegmentCount(Math.PI / 2, 0.02)).toBe(6)
    // Large radius: 96 per turn -> a quarter turn gets 24.
    expect(arcSegmentCount(Math.PI / 2, 5)).toBe(24)
  })

  it('ignores the sweep sign', () => {
    expect(arcSegmentCount(Math.PI, 0.02)).toBe(arcSegmentCount(-Math.PI, 0.02))
  })

  it('rounds partial fractions up', () => {
    expect(arcSegmentCount(Math.PI / 2 + 0.05, 0.02)).toBe(7)
  })

  it('never drops below 2 segments, even for a sliver sweep', () => {
    expect(arcSegmentCount(0.01, 0.02)).toBe(2)
  })
})

describe('arcPolyline', () => {
  it('emits segmentCount + 1 points, all interior points at distance r from the center', () => {
    const s = 0.5
    const arc = arcFromChord(A, B, s)!
    const pts = arcPolyline(A, B, s)!
    expect(pts.length).toBe(arcSegmentCount(arc.sweep, arc.radius) + 1)
    for (const p of pts) {
      const d = Math.hypot(p[0] - arc.center[0], p[1] - arc.center[1])
      expect(d).toBeCloseTo(arc.radius, 10)
    }
  })

  it('endpoints are EXACTLY A and B (assigned, not recomputed — no float drift)', () => {
    const a: Vec2 = [0.1234567890123, -7.654321098765]
    const b: Vec2 = [3.3219280948874, 1.4426950408889]
    const pts = arcPolyline(a, b, 0.7)!
    const first = pts[0]
    const last = pts[pts.length - 1]
    expect(first[0]).toBe(a[0])
    expect(first[1]).toBe(a[1])
    expect(last[0]).toBe(b[0])
    expect(last[1]).toBe(b[1])
  })

  it('every point stays on the bulge side of the chord (except the endpoints)', () => {
    const pts = arcPolyline(A, B, 0.5)!
    for (const p of pts.slice(1, -1)) {
      expect(p[1]).toBeGreaterThan(0) // bulge side is +Y for chord A→B along +X
    }
    const ptsNeg = arcPolyline(A, B, -0.5)!
    for (const p of ptsNeg.slice(1, -1)) {
      expect(p[1]).toBeLessThan(0)
    }
  })

  it('returns null on degenerate input', () => {
    expect(arcPolyline(A, B, 0)).toBeNull()
    expect(arcPolyline(A, [0, 0], 0.5)).toBeNull()
  })
})

describe('arcPolylineOnPlane', () => {
  it('ground plane (u=X, v=Y) matches the 2D polyline with z carried from A', () => {
    const a3: Vec3 = [0, 0, 0]
    const b3: Vec3 = [2, 0, 0]
    const pts3 = arcPolylineOnPlane(a3, b3, 0.5, [1, 0, 0], [0, 1, 0])!
    const pts2 = arcPolyline(A, B, 0.5)!
    expect(pts3.length).toBe(pts2.length)
    for (let i = 0; i < pts3.length; i++) {
      expect(pts3[i][0]).toBeCloseTo(pts2[i][0], 12)
      expect(pts3[i][1]).toBeCloseTo(pts2[i][1], 12)
      expect(pts3[i][2]).toBe(0)
    }
  })

  it('endpoints are EXACTLY the given 3D points', () => {
    const a3: Vec3 = [0.5, 2, 1]
    const b3: Vec3 = [1.5, 2, 1]
    // Arc in the z=1 plane.
    const pts = arcPolylineOnPlane(a3, b3, -0.4, [1, 0, 0], [0, 1, 0])!
    expect(pts[0]).toEqual(a3)
    expect(pts[0][0]).toBe(a3[0])
    expect(pts[pts.length - 1][0]).toBe(b3[0])
    expect(pts[pts.length - 1][1]).toBe(b3[1])
    expect(pts[pts.length - 1][2]).toBe(b3[2])
  })

  it('lies in an arbitrary plane: vertical XZ plane (u=X, v=Z) keeps y constant', () => {
    const a3: Vec3 = [0, 3, 0]
    const b3: Vec3 = [2, 3, 0]
    const pts = arcPolylineOnPlane(a3, b3, 0.5, [1, 0, 0], [0, 0, 1])!
    for (const p of pts) {
      expect(p[1]).toBeCloseTo(3, 12)
    }
    // Bulge is toward +v = +Z.
    const apexish = pts[Math.floor(pts.length / 2)]
    expect(apexish[2]).toBeGreaterThan(0)
  })

  it('all points are equidistant from the lifted 3D center', () => {
    const a3: Vec3 = [1, 1, 2]
    const b3: Vec3 = [1, 4, 2]
    const u: Vec3 = [0, 1, 0]
    const v: Vec3 = [0, 0, 1]
    const s = 0.9
    const arc = arcFromChord([0, 0], [3, 0], s)! // chord length 3 in the (u,v) frame
    const center3: Vec3 = [
      a3[0] + u[0] * arc.center[0] + v[0] * arc.center[1],
      a3[1] + u[1] * arc.center[0] + v[1] * arc.center[1],
      a3[2] + u[2] * arc.center[0] + v[2] * arc.center[1],
    ]
    const pts = arcPolylineOnPlane(a3, b3, s, u, v)!
    for (const p of pts) {
      const d = Math.hypot(p[0] - center3[0], p[1] - center3[1], p[2] - center3[2])
      expect(d).toBeCloseTo(arc.radius, 10)
    }
  })

  it('returns null on degenerate input', () => {
    expect(arcPolylineOnPlane([0, 0, 0], [2, 0, 0], 0, [1, 0, 0], [0, 1, 0])).toBeNull()
    expect(arcPolylineOnPlane([0, 0, 0], [0, 0, 0], 0.5, [1, 0, 0], [0, 1, 0])).toBeNull()
  })
})
