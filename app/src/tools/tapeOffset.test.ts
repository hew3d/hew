import { describe, it, expect } from 'vitest'
import {
  AXIS_ALONG_EDGE_DEG,
  AXIS_ALONG_EDGE_SIN,
  AXIS_ALONG_EDGE_COS,
  offsetDirForAxis,
  offsetPlaneNormal,
  viableOffsetAxes,
  projectPointOntoPlane,
  signedOffsetAlong,
  stationOnAxisFromRay,
} from './tapeOffset'
import type { V3 } from '../viewport/geoHelpers'
import type { DrawingAxes } from './drawingAxes'

const DEG = Math.PI / 180

function rotateZ(v: V3, deg: number): V3 {
  const c = Math.cos(deg * DEG)
  const s = Math.sin(deg * DEG)
  return [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]]
}

describe('constants', () => {
  it('AXIS_ALONG_EDGE_DEG matches the kernel SOFT_AXIS_EDGE_ON_DEG value', () => {
    expect(AXIS_ALONG_EDGE_DEG).toBe(3.0)
  })

  it('sin/cos companions are derived from the degree constant', () => {
    expect(AXIS_ALONG_EDGE_SIN).toBeCloseTo(Math.sin(3 * DEG), 15)
    expect(AXIS_ALONG_EDGE_COS).toBeCloseTo(Math.cos(3 * DEG), 15)
  })
})

describe('offsetDirForAxis', () => {
  const d: V3 = [1, 0, 0]

  it('an axis exactly perpendicular to the edge returns the axis itself', () => {
    const a: V3 = [0, 1, 0]
    const result = offsetDirForAxis(d, a)
    expect(result).not.toBeNull()
    expect(result![0]).toBeCloseTo(0, 15)
    expect(result![1]).toBeCloseTo(1, 15)
    expect(result![2]).toBeCloseTo(0, 15)
  })

  it('an axis at 45 degrees returns the normalized perpendicular component', () => {
    const a: V3 = [Math.SQRT1_2, Math.SQRT1_2, 0]
    const result = offsetDirForAxis(d, a)
    expect(result).not.toBeNull()
    expect(result![0]).toBeCloseTo(0, 10)
    expect(result![1]).toBeCloseTo(1, 10)
    expect(result![2]).toBeCloseTo(0, 10)
  })

  it('an axis 1 degree off the edge returns null', () => {
    const a = rotateZ(d, 1)
    expect(offsetDirForAxis(d, a)).toBeNull()
  })

  it('an axis 2.9 degrees off the edge returns null', () => {
    const a = rotateZ(d, 2.9)
    expect(offsetDirForAxis(d, a)).toBeNull()
  })

  it('an axis at exactly the edge direction returns null', () => {
    expect(offsetDirForAxis(d, d)).toBeNull()
  })

  it('an axis 3.5 degrees off the edge is viable and returns a small nonzero result', () => {
    const a = rotateZ(d, 3.5)
    const result = offsetDirForAxis(d, a)
    expect(result).not.toBeNull()
    // The perpendicular component's direction is [0, 1, 0] regardless of
    // the angle (only its magnitude before normalizing shrinks) — after
    // normalizing, the result is still unit-length [0, 1, 0].
    expect(result![0]).toBeCloseTo(0, 10)
    expect(result![1]).toBeCloseTo(1, 10)
    expect(result![2]).toBeCloseTo(0, 10)
  })

  it('an axis nearly opposite the edge (anti-parallel, within 3 degrees) returns null', () => {
    // 179 degrees from d: 1 degree off the exact reverse of d.
    const a = rotateZ(d, 179)
    expect(offsetDirForAxis(d, a)).toBeNull()
  })

  it('the exact reverse of the edge direction returns null', () => {
    const a: V3 = [-1, 0, 0]
    expect(offsetDirForAxis(d, a)).toBeNull()
  })

  it('an axis 175 degrees from the edge (5 degrees short of the reverse) is viable', () => {
    const a = rotateZ(d, 175)
    const result = offsetDirForAxis(d, a)
    expect(result).not.toBeNull()
  })
})

describe('offsetPlaneNormal', () => {
  const d: V3 = [1, 0, 0]

  it('an already-perpendicular m returns m unchanged', () => {
    const m: V3 = [0, 0, 1]
    const result = offsetPlaneNormal(d, m)
    expect(result).not.toBeNull()
    expect(result![0]).toBeCloseTo(0, 15)
    expect(result![1]).toBeCloseTo(0, 15)
    expect(result![2]).toBeCloseTo(1, 15)
  })

  it('a near-parallel m returns null', () => {
    expect(offsetPlaneNormal(d, [1, 0, 0])).toBeNull()
    expect(offsetPlaneNormal(d, [1, 1e-12, 0])).toBeNull()
  })
})

describe('viableOffsetAxes', () => {
  it('is computed relative to a moved (rotated + translated) frame, not world X/Y/Z', () => {
    // A frame rotated 45 degrees about Z and translated away from the
    // origin — origin/translation must not affect axis viability, only
    // the axis directions themselves.
    const frame: DrawingAxes = {
      origin: [5, 7, -2],
      x: rotateZ([1, 0, 0], 45),
      y: rotateZ([0, 1, 0], 45),
      z: [0, 0, 1],
    }

    // Edge runs exactly along the frame's own X — NOT along world X (which
    // is 45 degrees away from frame.x here). If this were computed against
    // world axes instead of the frame's, the "along the edge" axis would
    // wrongly come out viable (world X is 45 degrees off frame.x) and the
    // world-Y-perpendicular case would be wrong too.
    const d = frame.x

    expect(viableOffsetAxes(d, frame)).toEqual([false, true, true])
  })

  it('all three axes are viable when the edge is off-axis in a world-identity frame', () => {
    const frame: DrawingAxes = {
      origin: [0, 0, 0],
      x: [1, 0, 0],
      y: [0, 1, 0],
      z: [0, 0, 1],
    }
    // A diagonal edge direction, not aligned (or near-aligned) with any axis.
    const d = normalizeForTest([1, 1, 1])
    expect(viableOffsetAxes(d, frame)).toEqual([true, true, true])
  })
})

function normalizeForTest(v: V3): V3 {
  const len = Math.hypot(v[0], v[1], v[2])
  return [v[0] / len, v[1] / len, v[2] / len]
}

describe('projectPointOntoPlane', () => {
  const origin: V3 = [0, 0, 0]
  const n: V3 = [0, 0, 1]

  it('is idempotent — projecting twice equals projecting once', () => {
    const p: V3 = [1, 2, 5]
    const once = projectPointOntoPlane(p, origin, n)
    const twice = projectPointOntoPlane(once, origin, n)
    expect(twice).toEqual(once)
  })

  it('is a no-op for a point already on the plane', () => {
    const p: V3 = [3, 4, 0]
    expect(projectPointOntoPlane(p, origin, n)).toEqual(p)
  })

  it('drops the out-of-plane component', () => {
    const p: V3 = [1, 2, 5]
    expect(projectPointOntoPlane(p, origin, n)).toEqual([1, 2, 0])
  })
})

describe('signedOffsetAlong', () => {
  const edgePoint: V3 = [0, 0, 0]
  const u: V3 = [1, 0, 0]

  it('positive offset', () => {
    expect(signedOffsetAlong([5, 0, 0], edgePoint, u)).toBeCloseTo(5, 12)
  })

  it('negative offset', () => {
    expect(signedOffsetAlong([-3, 0, 0], edgePoint, u)).toBeCloseTo(-3, 12)
  })

  it('zero offset for a cursor purely orthogonal to u', () => {
    expect(signedOffsetAlong([0, 7, 0], edgePoint, u)).toBeCloseTo(0, 12)
  })
})

describe('stationOnAxisFromRay', () => {
  it('a ray aimed dead-on at a point on the axis returns that point\'s known station', () => {
    const p0: V3 = [2, 3, 4]
    const axis: V3 = [0, 1, 0]
    // The known target: p0 + axis*6 = [2, 9, 4]. A ray from [10, 9, 4]
    // pointing in -X hits that point exactly.
    const rayOrigin: V3 = [10, 9, 4]
    const rayDir: V3 = [-1, 0, 0]
    expect(stationOnAxisFromRay(p0, axis, rayOrigin, rayDir)).toBeCloseTo(6, 9)
  })

  it('a ray exactly parallel to the axis returns the ray origin\'s own projected station, not 0', () => {
    const p0: V3 = [0, 0, 0]
    const axis: V3 = [0, 1, 0]
    const rayOrigin: V3 = [5, 5, 5]
    const rayDir: V3 = [0, 1, 0] // parallel to axis — denom collapses to 0
    // Kernel-mirroring fallback: dot(rayOrigin - p0, axis) = dot([5,5,5],[0,1,0]) = 5.
    expect(stationOnAxisFromRay(p0, axis, rayOrigin, rayDir)).toBeCloseTo(5, 12)
  })

  it('near-parallel (denom under the 1e-12 threshold) returns dot(rayOrigin - p0, axis), not 0', () => {
    // A known, non-trivial p0/rayOrigin, with rayDir nudged just far enough
    // off exact parallel with axis that it still trips the near-parallel
    // branch (denom = 1 - b*b < 1e-12 requires |b| > sqrt(1 - 1e-12), i.e.
    // within ~1e-6 rad of parallel).
    const p0: V3 = [2, -3, 7]
    const axis = normalizeForTest([1, 1, 0])
    const rayOrigin: V3 = [11, -1, 4]
    const tinyAngle = 1e-8 // rad off parallel — denom ≈ (tinyAngle)^2 ≈ 1e-16 « 1e-12
    // Perpendicular to axis, in-plane, to tilt rayDir slightly away from it.
    const perp = normalizeForTest([-axis[1], axis[0], 0])
    const rayDir = normalizeForTest([
      axis[0] + perp[0] * tinyAngle,
      axis[1] + perp[1] * tinyAngle,
      axis[2] + perp[2] * tinyAngle,
    ])
    const b = axis[0] * rayDir[0] + axis[1] * rayDir[1] + axis[2] * rayDir[2]
    expect(Math.abs(1 - b * b)).toBeLessThan(1e-12) // sanity: branch is actually engaged

    // Expected computed independently of the implementation's own formula:
    // the station of rayOrigin's own foot on the line p0 + t*axis.
    const w = [rayOrigin[0] - p0[0], rayOrigin[1] - p0[1], rayOrigin[2] - p0[2]]
    const expected = w[0] * axis[0] + w[1] * axis[1] + w[2] * axis[2]

    expect(stationOnAxisFromRay(p0, axis, rayOrigin, rayDir)).toBeCloseTo(expected, 9)
  })

  it('an orthographic view straight down the axis (constant rayDir) tracks the cursor smoothly instead of pinning at 0', () => {
    // "Looking straight down the locked axis" in an orthographic view means
    // EVERY on-screen ray shares the exact same direction (parallel
    // projection) — so `denom` is identically 0 for every pixel, and the
    // degenerate branch is live for the whole view, not just one pixel. The
    // only thing that varies from pixel to pixel is `rayOrigin`. Before the
    // fix, every one of these returned a flat, cursor-independent 0; the
    // fix makes the station a continuous (in fact linear) function of
    // `rayOrigin`, so it tracks cursor motion the way a user expects
    // instead of jumping to (and sitting dead on) zero everywhere.
    const p0: V3 = [1, 2, 3]
    const axis: V3 = [0, 0, 1]
    const rayDir: V3 = [0, 0, 1] // identical for every sample, as in ortho view

    // A handful of nearby "cursor positions" (ray origins), stepping a small
    // amount at a time — as on a real screen, no two adjacent samples are
    // wildly far apart.
    const rayOrigins: V3[] = [
      [4, 5, 10],
      [4, 5, 10.2],
      [4, 5, 10.4],
      [4, 5, 10.6],
      [4, 5, 10.8],
      [4, 5, 11],
    ]
    const stations = rayOrigins.map((rayOrigin) =>
      stationOnAxisFromRay(p0, axis, rayOrigin, rayDir),
    )

    // Matches the hand-derived formula exactly (dot(rayOrigin - p0, axis) —
    // just the z-component difference here), and is neither flat nor 0.
    stations.forEach((s, i) => {
      expect(s).toBeCloseTo(rayOrigins[i][2] - p0[2], 12)
    })
    // Strictly increasing as rayOrigin steps in +Z, i.e. genuinely tracking
    // cursor motion rather than sitting flat at a constant (let alone 0).
    for (let i = 1; i < stations.length; i++) {
      expect(stations[i]).toBeGreaterThan(stations[i - 1])
      expect(stations[i] - stations[i - 1]).toBeCloseTo(0.2, 9)
    }
  })

  it('a camera sighting along the axis crosses the near-parallel threshold with no jump', () => {
    // Sample the station as rayDir's angle off the axis sweeps from clearly
    // non-parallel, through the near-parallel threshold, down to exactly
    // parallel, with `rayOrigin` placed ON the axis line itself (camera
    // sighting straight down it) — the realistic geometry in which the
    // reach clamp never saturates before the near-parallel threshold does,
    // so the transition is genuinely continuous rather than merely
    // bounded. (For a camera far off to the side of the axis, the reach
    // clamp — a separate, pre-existing, and deliberately independent
    // mechanism — already pins the reported station at its bound well
    // before the near-parallel threshold is reached; that clamp's own
    // behavior is unchanged by this fix and is covered by the dedicated
    // reach-clamp test below.)
    const p0: V3 = [1, 0, 0]
    const axis: V3 = [0, 1, 0]
    const rayOrigin: V3 = [1, 6, 0] // on the line p0 + t*axis, at t = 6
    const perp: V3 = [1, 0, 0] // perpendicular to axis

    // Angles (rad off parallel) spanning well outside the threshold down to
    // exactly 0. denom = 1 - cos(theta)^2 = sin(theta)^2, so denom < 1e-12
    // once theta is below ~1e-6 rad.
    const angles = [1e-3, 1e-4, 1e-5, 1e-6, 1e-7, 1e-8, 0]
    const stations = angles.map((theta) => {
      const rayDir = normalizeForTest([
        axis[0] * Math.cos(theta) + perp[0] * Math.sin(theta),
        axis[1] * Math.cos(theta) + perp[1] * Math.sin(theta),
        axis[2] * Math.cos(theta) + perp[2] * Math.sin(theta),
      ])
      return stationOnAxisFromRay(p0, axis, rayOrigin, rayDir)
    })

    // Every sample lands on the ray origin's own station (6) — including
    // right at and past the threshold — with no jump anywhere, let alone a
    // jump down to 0.
    for (const s of stations) {
      expect(s).toBeCloseTo(6, 9)
    }
  })

  it('the reach clamp engages for a ray far off to the side, nearly parallel to the axis', () => {
    const p0: V3 = [0, 0, 0]
    const axis: V3 = [1, 0, 0]
    // |w| = 1 (rayOrigin is 1 unit off the axis), so the reach bound is
    // exactly ±5. rayDir is tilted 0.01 rad off the axis direction — close
    // enough to parallel that the UNCLAMPED station (-cos(θ)/sin(θ) ≈ -100)
    // is far outside that bound, so the clamp must engage and pin the
    // result at exactly -5 (not some intermediate blown-up value).
    const rayOrigin: V3 = [0, 1, 0]
    const theta = 0.01
    const rayDir: V3 = [Math.cos(theta), Math.sin(theta), 0]
    const unclamped = -Math.cos(theta) / Math.sin(theta)
    expect(Math.abs(unclamped)).toBeGreaterThan(5) // sanity: clamp is actually needed here
    expect(stationOnAxisFromRay(p0, axis, rayOrigin, rayDir)).toBeCloseTo(-5, 9)
  })
})
