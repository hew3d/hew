/**
 * followMeStart specs — the app-side mirror of Follow Me's start rule.
 *
 * These pin the property that matters most: the cue must never say "legal
 * here" where the kernel refuses. So every affirmative case is a placement the
 * kernel's own scan in `Object::from_follow_me` accepts, every refusal case is
 * one it rejects, and the four-valued contract (no `'ok'` on an f32 face loop,
 * `'unknown'` inside the slop band, `'orient'` where auto-orientation now
 * folds a placement the kernel used to refuse outright) is pinned explicitly.
 *
 * The end-to-end cross-check against the REAL kernel — the same lathe scene,
 * driven with real pointer input, wrong placement warned about and right
 * placement swept — lives in `app/e2e/follow-me-start-cue.spec.ts`.
 */
import { describe, it, expect } from 'vitest'
import {
  evaluateStart,
  chainPath,
  refusalGuidance,
  orientGuidance,
  type PathPolyline,
  type PathSegment,
  type PlaneDef,
  type Vec3,
} from './followMeStart'

const GROUND_NORMAL: Vec3 = [0, 0, 1]

/** A drawn circle path: `n` facets on the ground plane about the origin, every
 *  segment attributed to one analytic curve — exactly what CircleTool stamps. */
function circlePath(radius = 1, n = 24, center: Vec3 = [0, 0, 0]): PathPolyline {
  const at = (k: number): Vec3 => {
    const a = (k * 2 * Math.PI) / n
    return [center[0] + radius * Math.cos(a), center[1] + radius * Math.sin(a), center[2]]
  }
  const segments: PathSegment[] = []
  for (let k = 0; k < n; k++) {
    segments.push({ a: at(k), b: at((k + 1) % n), curve: { center, radius } })
  }
  return { segments, closed: true, exact: true }
}

/** A closed rectangle of plain segments on the ground. */
function rectPath(exact = true): PathPolyline {
  const pts: Vec3[] = [
    [0, 0, 0],
    [2, 0, 0],
    [2, 1, 0],
    [0, 1, 0],
  ]
  const segments: PathSegment[] = pts.map((p, i) => ({
    a: p,
    b: pts[(i + 1) % pts.length],
    curve: null,
  }))
  return { segments, closed: true, exact }
}

/** The open L the Line tool draws: (0,0,0) → (0,2,0) → (2,2,0). */
function lPath(): PathPolyline {
  return {
    segments: [
      { a: [0, 0, 0], b: [0, 2, 0], curve: null },
      { a: [0, 2, 0], b: [2, 2, 0], curve: null },
    ],
    closed: false,
    exact: true,
  }
}

function plane(point: Vec3, normal: Vec3): PlaneDef {
  return { point, normal }
}

describe('followMeStart — circle path', () => {
  it('accepts a RADIAL profile plane: normal in the circle plane, passing through the centre', () => {
    // The lathe placement: the profile stands on the y = 0 plane, which
    // contains the circle's axis. This is the whole legal family.
    expect(evaluateStart(circlePath(), plane([0, 0, 0], [0, 1, 0]))).toEqual({ kind: 'ok' })
    expect(evaluateStart(circlePath(), plane([0, 0, 0], [1, 0, 0]))).toEqual({ kind: 'ok' })
  })

  it('accepts a radial plane at an arbitrary angle, not only the axis-aligned ones', () => {
    const a = 0.37
    const n: Vec3 = [Math.cos(a), Math.sin(a), 0]
    expect(evaluateStart(circlePath(), plane([0, 0, 0], n))).toEqual({ kind: 'ok' })
  })

  it('informs (not warns) about a profile lying FLAT on the circle plane — auto-orientation folds it up', () => {
    // Used to be the decisive "not radial" refusal; the kernel now folds a
    // flat profile up into the lathe instead of refusing (design §2c), so
    // the cue is informational, not a red warning.
    expect(evaluateStart(circlePath(), plane([0, 0, 0], GROUND_NORMAL))).toEqual({ kind: 'orient' })
  })

  it('informs about an upright profile whose plane MISSES the circle centre — the fold recentres it', () => {
    // Correctly oriented, but offset — the placement a user lands on when the
    // quadrant snap was not taken. Auto-orientation's off-center slide (the
    // "already perpendicular" branch of `orient_profile_to_path`) recentres
    // it instead of refusing.
    expect(evaluateStart(circlePath(), plane([0, 0.3, 0], [0, 1, 0]))).toEqual({ kind: 'orient' })
  })

  it('accepts a joint shared by two SEPARATELY drawn arcs of one circle', () => {
    // The kernel's `same_curve` compares CurveGeom, not chain identity, so two
    // arcs of one circle meeting end to end are a smooth joint it can seam on.
    // Comparing chain ids instead would call that joint a corner and refuse a
    // placement the kernel accepts — which is why the mirror compares geometry.
    const path = circlePath()
    const twoArcs: PathPolyline = {
      ...path,
      // Distinct chain records, identical analytic circle: only an
      // identity-based test would see corners here.
      segments: path.segments.map((s) => ({ ...s, curve: { ...s.curve! } })),
    }
    expect(evaluateStart(twoArcs, plane([0, 0, 0], [0, 1, 0]))).toEqual({ kind: 'ok' })
  })
})

describe('followMeStart — closed polyline path (the rectangle)', () => {
  it('accepts a profile square to a side and crossing its INTERIOR', () => {
    expect(evaluateStart(rectPath(), plane([1, 0, 0], [1, 0, 0]))).toEqual({ kind: 'ok' })
  })

  it('is unknown at a corner without the profile geometry to run the fold test on', () => {
    // Without a `profileRing`, a corner placement can never be blessed OR
    // warned about — it takes the actual profile extent to judge (design
    // §2b), which is the whole point of the optional third argument.
    expect(evaluateStart(rectPath(), plane([0, 0, 0], [1, 0, 0]))).toEqual({ kind: 'unknown' })
  })

  it('accepts a corner seam whose profile sits entirely BEYOND the corner (design §2b)', () => {
    // Corner (0,0,0): the perpendicular flank is (0,0,0)->(2,0,0) (+x); the
    // OTHER flank arrives from (0,1,0), so "beyond the corner" is y <= 0. This
    // profile (y in [-0.5, -0.1]) sits fully on the legal side — the mitered
    // picture-frame band the kernel's own spec builds.
    const ring: Vec3[] = [
      [0, -0.5, -0.2],
      [0, -0.1, -0.2],
      [0, -0.1, 0.2],
      [0, -0.5, 0.2],
    ]
    expect(evaluateStart(rectPath(), plane([0, 0, 0], [1, 0, 0]), ring)).toEqual({ kind: 'ok' })
  })

  it('refuses a corner seam whose profile hangs back over the incoming flank', () => {
    // Same corner, but the profile straddles it (y in [-0.2, 0.2]) — part of
    // it hangs back over the arriving flank (y > 0 there), the fold the
    // kernel's advance check refuses as PathTooTight.
    const ring: Vec3[] = [
      [0, -0.2, -0.2],
      [0, 0.2, -0.2],
      [0, 0.2, 0.2],
      [0, -0.2, 0.2],
    ]
    expect(evaluateStart(rectPath(), plane([0, 0, 0], [1, 0, 0]), ring)).toEqual({
      kind: 'refused',
      reason: 'corner-overhang',
    })
    expect(refusalGuidance('corner-overhang')).toContain('corner')
  })

  it('orientGuidance is informational — no "refused" vocabulary', () => {
    const copy = orientGuidance()
    expect(copy.toLowerCase()).not.toContain('refus')
    expect(copy.toLowerCase()).toContain('upright')
  })

  it('informs (not warns) about a profile square to nothing — auto-orientation folds it upright', () => {
    // Used to be the decisive "not square" refusal; the kernel now folds a
    // profile square to nothing into a legal placement (design §2c) — see
    // `follow_me_auto_orients_a_flat_profile_around_a_frame` in op_specs.rs.
    const d = Math.SQRT1_2
    expect(evaluateStart(rectPath(), plane([1, 0.5, 0], [d, d, 0]))).toEqual({ kind: 'orient' })
  })

  it('refuses a profile square to the path but not touching it', () => {
    expect(evaluateStart(rectPath(), plane([5, 0, 0], [1, 0, 0]))).toEqual({
      kind: 'refused',
      reason: 'detached',
    })
  })
})

describe('followMeStart — open path', () => {
  it('accepts a profile through an END, square to that end segment (attached, not carried)', () => {
    expect(evaluateStart(lPath(), plane([0, 0, 0], [0, 1, 0]))).toEqual({
      kind: 'ok',
      carried: false,
    })
    expect(evaluateStart(lPath(), plane([2, 2, 0], [1, 0, 0]))).toEqual({
      kind: 'ok',
      carried: false,
    })
  })

  it('accepts (CARRIED) a profile square to an end segment but sitting somewhere else on it', () => {
    // Design §2a: a perpendicular-but-detached open end is no longer a
    // refusal — the path is carried rigidly to the profile. `carried: true`
    // is how the tool knows to say so, rather than implying the path itself
    // passes through here.
    expect(evaluateStart(lPath(), plane([0, 2, 0], [0, 1, 0]))).toEqual({
      kind: 'ok',
      carried: true,
    })
  })

  it('informs (not warns) about a profile square to neither end — auto-orientation folds it onto the nearer one', () => {
    // Used to be the decisive "not square" refusal; the kernel now folds the
    // profile onto whichever end it's nearest and retries (design §2c) — see
    // `follow_me_auto_orient_hinges_at_a_touching_flap` in op_specs.rs.
    expect(evaluateStart(lPath(), plane([0, 0, 0], [0, 0, 1]))).toEqual({ kind: 'orient' })
  })
})

describe('followMeStart — same-curve tolerance matches the kernel exactly', () => {
  it('measures centre agreement as a BALL, not a per-axis box', () => {
    // The kernel's `same_curve` compares centres with `Point3::approx_eq`,
    // which is Euclidean: `(a - b).length_squared() <= POINT_MERGE²`. A
    // per-axis comparison would be a cube of half-width POINT_MERGE, and the
    // cube's corners stick out of the ball — so a joint offset diagonally by
    // 6e-10 per axis (1.04e-9 away) would read as "same curve" here while the
    // kernel refused it, and the cue would say `ok` for a placement the sweep
    // then rejects with PathDetachedFromProfile.
    const e = 6e-10
    const path: PathPolyline = {
      segments: [
        { a: [0, 0, -1], b: [0, 0, 0], curve: { center: [0, 0, 0], radius: 1 } },
        { a: [0, 0, 0], b: [0, 0, 1], curve: { center: [e, e, e], radius: 1 } },
        { a: [0, 0, 1], b: [5, 0, 0], curve: null },
        { a: [5, 0, 0], b: [0, 0, -1], curve: null },
      ],
      closed: true,
      exact: true,
    }
    // Each axis offset is inside POINT_MERGE, so a per-axis test would call
    // this a facet joint and bless it. The kernel does not, so neither do we.
    expect(Math.abs(e)).toBeLessThanOrEqual(1e-9)
    expect(Math.hypot(e, e, e)).toBeGreaterThan(1e-9)
    expect(evaluateStart(path, plane([0, 0, 0], [0, 0, 1]))).not.toEqual({ kind: 'ok' })
  })

  it('still accepts a joint whose centres agree within the ball', () => {
    // The same construction with the offset small enough to be inside the
    // kernel's ball — this one the kernel does seam, so `ok` is correct.
    const e = 4e-10 // hypot = 6.9e-10 < 1e-9
    const path: PathPolyline = {
      segments: [
        { a: [0, 0, -1], b: [0, 0, 0], curve: { center: [0, 0, 0], radius: 1 } },
        { a: [0, 0, 0], b: [0, 0, 1], curve: { center: [e, e, e], radius: 1 } },
        { a: [0, 0, 1], b: [5, 0, 0], curve: null },
        { a: [5, 0, 0], b: [0, 0, -1], curve: null },
      ],
      closed: true,
      exact: true,
    }
    expect(Math.hypot(e, e, e)).toBeLessThan(1e-9)
    expect(evaluateStart(path, plane([0, 0, 0], [0, 0, 1]))).toEqual({ kind: 'ok' })
  })
})

describe('followMeStart — what it refuses to claim', () => {
  it('never returns ok for an f32 face loop, however right the placement is', () => {
    // face_boundary crosses the WASM boundary as f32, ~7 orders coarser than
    // the kernel's 1e-9 tolerances, so an affirmative verdict would be a
    // guess. The same placement on an f64 sketch-edge path IS ok.
    const placement = plane([1, 0, 0], [1, 0, 0])
    expect(evaluateStart(rectPath(true), placement)).toEqual({ kind: 'ok' })
    expect(evaluateStart(rectPath(false), placement)).toEqual({ kind: 'unknown' })
  })

  it('still warns about a decisively wrong placement on an f32 face loop', () => {
    // The corner fold test needs no kernel-exact precision to decisively
    // refuse — it runs on the SLOP band regardless of `path.exact` — so an
    // f32 face loop still earns a `refused` here, even though it could never
    // earn the `ok` a legal corner seam gets.
    const ring: Vec3[] = [
      [0, -0.2, -0.2],
      [0, 0.2, -0.2],
      [0, 0.2, 0.2],
      [0, -0.2, 0.2],
    ]
    expect(evaluateStart(rectPath(false), plane([0, 0, 0], [1, 0, 0]), ring)).toEqual({
      kind: 'refused',
      reason: 'corner-overhang',
    })
    // Square to nothing is 'orient' now (auto-orientation), not a refusal —
    // even on an f32 face loop, which never earns the affirmative 'ok'.
    const d = Math.SQRT1_2
    expect(evaluateStart(rectPath(false), plane([1, 0.5, 0], [d, d, 0]))).toEqual({ kind: 'orient' })
  })

  it('says unknown, not refused, for a placement inside the slop band', () => {
    // Tilted by 1e-6 rad: the kernel refuses this (its tolerance is 1e-9), but
    // it is far too close to call from the app side, so nothing is claimed.
    const e = 1e-6
    const n: Vec3 = [0, Math.cos(e), Math.sin(e)]
    expect(evaluateStart(circlePath(), plane([0, 0, 0], n))).toEqual({ kind: 'unknown' })
  })

  it('says unknown when there is no path at all', () => {
    expect(evaluateStart({ segments: [], closed: true, exact: true }, plane([0, 0, 0], [0, 1, 0])))
      .toEqual({ kind: 'unknown' })
  })
})

describe('followMeStart — chainPath', () => {
  it('recognises a closed loop and an open chain', () => {
    expect(chainPath(rectPath().segments)?.closed).toBe(true)
    expect(chainPath(lPath().segments)?.closed).toBe(false)
    expect(chainPath(circlePath().segments)?.closed).toBe(true)
  })

  it('ORIENTS the walk, so every joint is the start of exactly one segment', () => {
    // A sketch edge stores its endpoints in whatever direction it was drawn,
    // and `sketch_edge_endpoints` reports them verbatim. Two arcs that each
    // END at a shared snapped point therefore have that joint as the `b` of
    // BOTH — and the kernel never sees that, because `chain_sketch_edges`
    // walks and reorients first. Without this walk the facet-joint seam test
    // (which, like the kernel, only looks at a segment's START) would miss
    // that vertex on both sides and call a legal lathe start `detached`.
    const backwards: PathSegment[] = rectPath().segments.map((seg, i) =>
      i % 2 === 0 ? seg : { ...seg, a: seg.b, b: seg.a },
    )
    const chained = chainPath(backwards)
    expect(chained).not.toBeNull()
    const { segments } = chained!
    for (let k = 0; k < segments.length; k++) {
      expect(segments[k].b).toEqual(segments[(k + 1) % segments.length].a)
    }
  })

  it('reads the same verdict whichever way the edges happened to be drawn', () => {
    // The end-to-end statement of the bug above: a mis-oriented circle path
    // must not change what the cue says about a placement the kernel accepts.
    const forward = circlePath()
    const scrambled = chainPath(
      forward.segments.map((seg, i) => (i % 3 === 0 ? { ...seg, a: seg.b, b: seg.a } : seg)),
    )
    expect(scrambled).not.toBeNull()
    const p = plane([0, 0, 0], [0, 1, 0])
    expect(evaluateStart({ ...scrambled!, exact: true }, p)).toEqual(
      evaluateStart(forward, p),
    )
    expect(evaluateStart({ ...scrambled!, exact: true }, p)).toEqual({ kind: 'ok' })
  })

  it('rejects a branching path (the kernel refuses it as PathBranches)', () => {
    const segments: PathSegment[] = [
      { a: [0, 0, 0], b: [1, 0, 0], curve: null },
      { a: [1, 0, 0], b: [2, 0, 0], curve: null },
      { a: [1, 0, 0], b: [1, 1, 0], curve: null },
    ]
    expect(chainPath(segments)).toBeNull()
  })

  it('rejects two disjoint loops (the kernel refuses them as PathDisconnected)', () => {
    const shift = (p: Vec3): Vec3 => [p[0] + 10, p[1], p[2]]
    const a = rectPath().segments
    const b = a.map((s) => ({ ...s, a: shift(s.a), b: shift(s.b) }))
    expect(chainPath(a)?.closed).toBe(true)
    expect(chainPath([...a, ...b])).toBeNull()
  })

  it('rejects an empty path', () => {
    expect(chainPath([])).toBeNull()
  })
})
