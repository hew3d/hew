/**
 * SnapService unit tests — the fallback tier (kernel snap absent/thrown).
 * Kernel-candidate tiers (acquire/hysteresis) are exercised indirectly by
 * every tool test that drives snapping; these pin the ray∩plane fallback
 * added for sketches on any plane (Phase 1, the sketch-planes design §3):
 * a supplied `constraintPlane` is the fallback target instead of ground.
 */
import { describe, it, expect, vi } from 'vitest'
import { SnapService, SNAP_RADIUS_PX, SOFT_AXIS_BREAK_APERTURE_SCALE } from './snapService'
import type { Scene } from '../wasm/pkg/wasm_api.js'
import { pixelRadiusToAperture, type ApertureBasis, type Ray } from './math'

/** A ray straight down -Z from above the origin. */
const DOWN: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

/** The perspective aperture basis every pre-existing test below used to
 * pass as a bare `45` (fovYDeg) — same meaning, now explicit (SnapService's
 * `resolve` takes an `ApertureBasis` so the same query code path also
 * serves parallel projection; see the 'ApertureBasis — projection-aware
 * aperture' describe block below for the parallel-projection cases). */
const PERSPECTIVE_45: ApertureBasis = { kind: 'perspective', fovYDeg: 45 }

/** A fake Scene whose `snap()` always misses (undefined) — every resolve()
 *  call falls through to the fallback tier under test. */
function fakeScene(): Scene {
  return { snap: vi.fn(() => undefined) } as unknown as Scene
}

describe('SnapService — fallback tier', () => {
  it('without a constraintPlane, falls back to ground (kind "ground")', () => {
    const svc = new SnapService(fakeScene())
    const { snap, fromKernel } = svc.resolve(DOWN, 800, PERSPECTIVE_45)
    expect(fromKernel).toBe(false)
    expect(snap).not.toBeNull()
    expect(snap?.kind).toBe('ground')
    expect(snap).toMatchObject({ x: 0, y: 0, z: 0 })
  })

  it('with a constraintPlane and no kernel candidate, falls back to ray∩plane (kind "plane")', () => {
    const svc = new SnapService(fakeScene())
    // A vertical plane through (2,0,0) with normal +X: the ray from (0,0,5)
    // straight down never reaches x=2, so this also proves the fallback
    // actually intersects the SUPPLIED plane rather than z=0.
    const constraintPlane = { point: [2, 0, 0] as [number, number, number], normal: [1, 0, 0] as [number, number, number] }
    const ray: Ray = { origin: [0, 0, 0], direction: [1, 0, 0] }
    const { snap, fromKernel } = svc.resolve(ray, 800, PERSPECTIVE_45, undefined, undefined, constraintPlane)
    expect(fromKernel).toBe(false)
    expect(snap?.kind).toBe('plane')
    expect(snap?.x).toBeCloseTo(2)
    expect(snap?.y).toBeCloseTo(0)
    expect(snap?.z).toBeCloseTo(0)
  })

  it('a constraintPlane the ray never reaches yields no snap at all (never falls through to ground)', () => {
    const svc = new SnapService(fakeScene())
    // Plane normal perpendicular to the ray direction — parallel miss.
    const constraintPlane = { point: [0, 0, 1] as [number, number, number], normal: [0, 0, 1] as [number, number, number] }
    const ray: Ray = { origin: [0, 0, 0], direction: [1, 0, 0] }
    const { snap, fromKernel } = svc.resolve(ray, 800, PERSPECTIVE_45, undefined, undefined, constraintPlane)
    expect(fromKernel).toBe(false)
    expect(snap).toBeNull()
  })

  it('a fallbackPlane is the miss target (kind "plane") but never reaches the kernel', () => {
    const scene = fakeScene()
    const svc = new SnapService(scene)
    const snapFn = scene.snap as unknown as ReturnType<typeof vi.fn>
    // The green plane y = 0 seen from the front: a level ray that never
    // reaches the ground still lands on it.
    const fallbackPlane = { point: [0, 0, 0] as [number, number, number], normal: [0, 1, 0] as [number, number, number] }
    const ray: Ray = { origin: [1, -10, 0.5], direction: [0, 1, 0] }
    const { snap, fromKernel } = svc.resolve(
      ray, 800, PERSPECTIVE_45, undefined, undefined, undefined, undefined, undefined, undefined, undefined, undefined, fallbackPlane,
    )
    expect(fromKernel).toBe(false)
    expect(snap?.kind).toBe('plane')
    expect(snap?.x).toBeCloseTo(1, 12)
    expect(snap?.y).toBeCloseTo(0, 12)
    expect(snap?.z).toBeCloseTo(0.5, 12)
    // Argument 9 is the kernel's constraint plane: unset, so geometry
    // anywhere stays a candidate.
    expect(snapFn).toHaveBeenCalled()
    for (const call of snapFn.mock.calls) expect(call[9]).toBeNull()
  })

  it('without a fallbackPlane the same level ray finds nothing (it never reaches the ground)', () => {
    const svc = new SnapService(fakeScene())
    const ray: Ray = { origin: [1, -10, 0.5], direction: [0, 1, 0] }
    expect(svc.resolve(ray, 800, PERSPECTIVE_45).snap).toBeNull()
  })

  it('a constraintPlane beats a fallbackPlane, and is the one sent to the kernel', () => {
    const scene = fakeScene()
    const svc = new SnapService(scene)
    const snapFn = scene.snap as unknown as ReturnType<typeof vi.fn>
    const constraintPlane = { point: [2, 0, 0] as [number, number, number], normal: [1, 0, 0] as [number, number, number] }
    const fallbackPlane = { point: [0, 3, 0] as [number, number, number], normal: [0, 1, 0] as [number, number, number] }
    const ray: Ray = { origin: [0, 0, 0], direction: [Math.SQRT1_2, Math.SQRT1_2, 0] }
    const { snap } = svc.resolve(
      ray, 800, PERSPECTIVE_45, undefined, undefined, constraintPlane, undefined, undefined, undefined, undefined, undefined, fallbackPlane,
    )
    expect(snap?.kind).toBe('plane')
    expect(snap?.x).toBeCloseTo(2, 12)
    expect(snap?.y).toBeCloseTo(2, 12)
    expect(Array.from(snapFn.mock.calls.at(-1)?.[9] as Float64Array)).toEqual([2, 0, 0, 1, 0, 0])
  })

  it('forwards offPlanePoints to Scene.snap as argument 13 (false when omitted)', () => {
    // The 3d-line staircase fix rides entirely on this bit reaching the
    // engine: LineTool opts in via `snapConstraint().offPlanePoints` and the
    // Viewport passes it through here.
    const scene = fakeScene()
    const svc = new SnapService(scene)
    const snapFn = scene.snap as unknown as ReturnType<typeof vi.fn>

    svc.resolve(DOWN, 800, PERSPECTIVE_45)
    expect(snapFn.mock.calls.at(-1)?.[13]).toBe(false)

    const constraintPlane = { point: [0, 1, 1] as [number, number, number], normal: [0, 1, 0] as [number, number, number] }
    svc.resolve(DOWN, 800, PERSPECTIVE_45, [1, 1, 1], undefined, constraintPlane, true)
    expect(snapFn.mock.calls.at(-1)?.[13]).toBe(true)
  })
})

describe('SnapService — precision mode', () => {
  it('passes the precision flag through to Scene.snap() as argument 10 (cylinder, soft_axis_aperture_scale, off_plane_points, pick trail it)', () => {
    const scene = fakeScene()
    const svc = new SnapService(scene)
    const snapFn = scene.snap as unknown as ReturnType<typeof vi.fn>
    const precisionArg = () => snapFn.mock.calls.at(-1)?.[10]

    svc.resolve(DOWN, 800, PERSPECTIVE_45)
    // The kernel owns the weighting; only this boolean crosses the boundary.
    // (The trailing arguments after precision are `cylinder`, then
    // `soft_axis_aperture_scale` — finding E — then `off_plane_points`,
    // then `pick`.)
    expect(precisionArg()).toBe(false)

    expect(svc.setPrecision(true)).toBe(true)
    expect(svc.isPrecision()).toBe(true)
    svc.resolve(DOWN, 800, PERSPECTIVE_45)
    expect(precisionArg()).toBe(true)

    expect(svc.setPrecision(false)).toBe(true)
    svc.resolve(DOWN, 800, PERSPECTIVE_45)
    expect(precisionArg()).toBe(false)
  })

  it('setting the same mode again is a no-op (keydown autorepeat must be free)', () => {
    const svc = new SnapService(fakeScene())
    expect(svc.setPrecision(false)).toBe(false)
    expect(svc.setPrecision(true)).toBe(true)
    expect(svc.setPrecision(true)).toBe(false)
  })

  it('toggling drops the held sticky snap, so hysteresis cannot pin the old target', () => {
    // A sticky snap is normally held across a miss: the acquire query losing
    // it triggers a second, wider "resist release" query. After a mode change
    // there is nothing to hold — the whole point of the toggle is that a
    // different candidate should win — so only the acquire query runs.
    const held = {
      x: () => 1, y: () => 2, z: () => 3,
      kind: () => 'endpoint',
      direction: () => undefined,
      projected_from: () => undefined,
      pierces_face: () => false,
      meeting_line: () => undefined,
      object: () => undefined,
      instance: () => undefined,
      element: () => 7n,
      element_kind: () => 'vertex',
      sketch: () => undefined,
      sketch_region: () => undefined,
      sketch_curve: () => undefined,
      free: () => {},
    }
    let hit = true
    const snapFn = vi.fn(() => (hit ? held : undefined))
    const scene = { snap: snapFn } as unknown as Scene
    const svc = new SnapService(scene)
    expect(svc.resolve(DOWN, 800, PERSPECTIVE_45).snap?.kind).toBe('endpoint')

    // Control: with the mode unchanged, losing the endpoint costs TWO queries
    // (acquire, then the wider release-resisting one).
    hit = false
    snapFn.mockClear()
    expect(svc.resolve(DOWN, 800, PERSPECTIVE_45).snap?.kind).toBe('ground')
    expect(snapFn.mock.calls.length).toBe(2)

    // Re-acquire, then toggle: the held snap is gone, so one query only.
    hit = true
    svc.resolve(DOWN, 800, PERSPECTIVE_45)
    svc.setPrecision(true)
    hit = false
    snapFn.mockClear()
    expect(svc.resolve(DOWN, 800, PERSPECTIVE_45).snap?.kind).toBe('ground')
    expect(snapFn.mock.calls.length).toBe(1)
  })

  it('hysteresis does NOT resist-release onto a DIFFERENT drawn circle whose centre shares every field but sketchCurve', () => {
    // Two circles in one sketch have Centre snaps identical in
    // kind/object/element/elementKind/sketch — they differ ONLY in
    // `sketchCurve`. When the cursor drifts off circle A's centre, the narrow
    // acquire misses and the wider release-resist query finds circle B's
    // centre. `sameTarget` must reject B (different curve) so the held snap
    // RELEASES to the ground fallback rather than silently jumping to B.
    // Remove the `a.sketchCurve === b.sketchCurve` clause and this flips: B is
    // treated as the same target and grabbed (kind 'center').
    const centreSnap = (curveId: bigint) => ({
      x: () => 1, y: () => 2, z: () => 0,
      kind: () => 'center',
      direction: () => undefined,
      projected_from: () => undefined,
      pierces_face: () => false,
      meeting_line: () => undefined,
      object: () => undefined,
      instance: () => undefined,
      element: () => undefined,
      element_kind: () => 'sketch-curve',
      sketch: () => 10n,
      sketch_region: () => undefined,
      sketch_curve: () => curveId,
      free: () => {},
    })
    const A = centreSnap(101n)
    const B = centreSnap(202n) // a DIFFERENT circle: only sketchCurve differs
    const narrowAperture = pixelRadiusToAperture(SNAP_RADIUS_PX, 800, 45)

    let phase: 'acquireA' | 'missThenB' = 'acquireA'
    const snapFn = vi.fn((..._args: unknown[]) => {
      if (phase === 'acquireA') return A
      // The 7th positional arg is the aperture; the narrow acquire query
      // misses, only the wider release-resist query (larger aperture) sees B.
      const aperture = _args[6] as number
      return aperture <= narrowAperture ? undefined : B
    })
    const svc = new SnapService({ snap: snapFn } as unknown as Scene)

    // Acquire A: a sticky Centre snap becomes the held target.
    expect(svc.resolve(DOWN, 800, PERSPECTIVE_45).snap?.kind).toBe('center')

    // Cursor drifts off A onto B's neighbourhood. B is not the same target,
    // so the result RELEASES (ground fallback), and is certainly not B.
    phase = 'missThenB'
    const released = svc.resolve(DOWN, 800, PERSPECTIVE_45).snap
    expect(released?.kind).toBe('ground')
    expect(released?.sketchCurve).toBeUndefined()
  })

  it('the resist-release query widens the soft-axis candidate\'s OWN aperture too, not just the pixel-derived one (finding E)', () => {
    // `on-axis` is a STICKY_KINDS member whose candidate never reads the
    // widened `aperture` the release query normally relies on (it has its
    // own fixed cone in the kernel) — without a dedicated scale argument, a
    // held soft-axis snap would have no hysteresis at all.
    const axisSnap = {
      x: () => 1, y: () => 0, z: () => 0,
      kind: () => 'on-axis',
      direction: () => new Float64Array([1, 0, 0]),
      projected_from: () => undefined,
      pierces_face: () => false,
      meeting_line: () => undefined,
      object: () => undefined,
      instance: () => undefined,
      element: () => undefined,
      element_kind: () => undefined,
      sketch: () => undefined,
      sketch_region: () => undefined,
      sketch_curve: () => undefined,
      free: () => {},
    }
    let hit = true
    const snapFn = vi.fn(() => (hit ? axisSnap : undefined))
    const svc = new SnapService({ snap: snapFn } as unknown as Scene)

    expect(svc.resolve(DOWN, 800, PERSPECTIVE_45).snap?.kind).toBe('on-axis')
    // The acquire query passes no scale (unscaled — the ordinary behavior).
    // The scale is positional arg 12 of `Scene.snap`; `off_plane_points`
    // and `pick` follow it.
    expect((snapFn.mock.calls[0] as unknown[])[SCALE_ARG]).toBeNull()

    hit = false
    snapFn.mockClear()
    svc.resolve(DOWN, 800, PERSPECTIVE_45)
    // Two queries: acquire (unscaled), then the wider release-resist query,
    // which must ALSO widen the soft-axis candidate's own tolerance.
    expect(snapFn.mock.calls.length).toBe(2)
    expect((snapFn.mock.calls[0] as unknown[])[SCALE_ARG]).toBeNull()
    expect((snapFn.mock.calls[1] as unknown[])[SCALE_ARG]).toBe(SOFT_AXIS_BREAK_APERTURE_SCALE)
  })

  it('passes `pick` to both the acquire and the release query, and false by default', () => {
    const held = {
      x: () => 0, y: () => 0, z: () => 0,
      kind: () => 'midpoint',
      direction: () => undefined,
      projected_from: () => undefined,
      pierces_face: () => false,
      meeting_line: () => undefined,
      object: () => undefined,
      instance: () => undefined,
      element: () => undefined,
      element_kind: () => undefined,
      sketch: () => undefined,
      sketch_region: () => undefined,
      sketch_curve: () => undefined,
      free: () => {},
    }
    let hit = true
    const snapFn = vi.fn(() => (hit ? held : undefined))
    const svc = new SnapService({ snap: snapFn } as unknown as Scene)

    svc.resolve(DOWN, 800, PERSPECTIVE_45)
    expect(snapFn.mock.calls[0].at(-2)).toBe(false)

    snapFn.mockClear()
    svc.resolve(DOWN, 800, PERSPECTIVE_45, undefined, undefined, undefined, undefined, undefined, undefined, true)
    expect(snapFn.mock.calls[0].at(-2)).toBe(true)
    hit = false
    snapFn.mockClear()
    svc.resolve(DOWN, 800, PERSPECTIVE_45, undefined, undefined, undefined, undefined, undefined, undefined, true)
    expect(snapFn.mock.calls.length).toBe(2)
    expect(snapFn.mock.calls[1].at(-2)).toBe(true)
  })
  it('passes `seeThroughPoints` to both the acquire and the release query, and false by default', () => {
    const held = {
      x: () => 0, y: () => 0, z: () => 0,
      kind: () => 'midpoint',
      direction: () => undefined,
      projected_from: () => undefined,
      pierces_face: () => false,
      meeting_line: () => undefined,
      object: () => undefined,
      instance: () => undefined,
      element: () => undefined,
      element_kind: () => undefined,
      sketch: () => undefined,
      sketch_region: () => undefined,
      sketch_curve: () => undefined,
      free: () => {},
    }
    let hit = true
    const snapFn = vi.fn(() => (hit ? held : undefined))
    const svc = new SnapService({ snap: snapFn } as unknown as Scene)

    svc.resolve(DOWN, 800, PERSPECTIVE_45)
    expect(snapFn.mock.calls[0].at(-1)).toBe(false)

    snapFn.mockClear()
    svc.resolve(DOWN, 800, PERSPECTIVE_45, undefined, undefined, undefined, undefined, undefined, undefined, undefined, true)
    expect(snapFn.mock.calls[0].at(-1)).toBe(true)
    hit = false
    snapFn.mockClear()
    svc.resolve(DOWN, 800, PERSPECTIVE_45, undefined, undefined, undefined, undefined, undefined, undefined, undefined, true)
    expect(snapFn.mock.calls.length).toBe(2)
    expect(snapFn.mock.calls[1].at(-1)).toBe(true)
  })

})

/** `Scene.snap`'s `soft_axis_aperture_scale` position. */
const SCALE_ARG = 12

describe('SnapService — ApertureBasis (projection-aware aperture, camera.md §1)', () => {
  // A held (sticky) endpoint snap the kernel returns whenever it's asked,
  // regardless of aperture — these tests are about what aperture gets SENT,
  // not the kernel's own admit/reject logic (that's inference's job).
  const held = {
    x: () => 1, y: () => 2, z: () => 3,
    kind: () => 'endpoint',
    direction: () => undefined,
    projected_from: () => undefined,
    pierces_face: () => false,
    meeting_line: () => undefined,
    object: () => undefined,
    instance: () => undefined,
    element: () => 7n,
    element_kind: () => 'vertex',
    sketch: () => undefined,
    sketch_region: () => undefined,
    sketch_curve: () => undefined,
    free: () => {},
  }

  it('a perspective basis sends cylinder=false and an angular (radians) aperture', () => {
    const snapFn = vi.fn((..._args: unknown[]) => held)
    const svc = new SnapService({ snap: snapFn } as unknown as Scene)

    svc.resolve(DOWN, 800, PERSPECTIVE_45)
    const aperture = snapFn.mock.calls[0][6] as number
    const cylinder = snapFn.mock.calls[0][11] as boolean

    expect(cylinder).toBe(false)
    expect(aperture).toBeCloseTo(pixelRadiusToAperture(SNAP_RADIUS_PX, 800, 45), 12)
  })

  it('a parallel basis sends cylinder=true and a world-radius (meters) aperture — a TRUE cylindrical tolerance, not a synthesized angle', () => {
    const snapFn = vi.fn((..._args: unknown[]) => held)
    const svc = new SnapService({ snap: snapFn } as unknown as Scene)

    const worldPerPixel = 0.02
    const parallelBasis: ApertureBasis = { kind: 'parallel', worldPerPixel }
    svc.resolve(DOWN, 800, parallelBasis)
    const aperture = snapFn.mock.calls[0][6] as number
    const cylinder = snapFn.mock.calls[0][11] as boolean

    expect(cylinder).toBe(true)
    // Exactly pixelRadius * worldPerPixel — no depth/target-distance enters
    // the computation at all (unlike phase 1's interim cone synthesis).
    expect(aperture).toBeCloseTo(SNAP_RADIUS_PX * worldPerPixel, 12)
  })

  it('a wider parallel worldPerPixel (zoomed further out) sends a proportionally wider aperture', () => {
    const snapFn = vi.fn((..._args: unknown[]) => held)
    const svc = new SnapService({ snap: snapFn } as unknown as Scene)

    svc.resolve(DOWN, 800, { kind: 'parallel', worldPerPixel: 0.01 })
    const tight = snapFn.mock.calls[0][6] as number

    snapFn.mockClear()
    svc.resolve(DOWN, 800, { kind: 'parallel', worldPerPixel: 0.05 })
    const wide = snapFn.mock.calls[0][6] as number

    expect(wide).toBeGreaterThan(tight)
    expect(wide).toBeCloseTo(5 * tight, 12)
  })
})

// Shop-mode playtest finding 5.
describe('SnapService — clearHold (finding 5a: discrete-tap hysteresis)', () => {
  it('drops a held sticky snap — the next miss releases in ONE query instead of resist-releasing in two', () => {
    const held = {
      x: () => 1, y: () => 2, z: () => 3,
      kind: () => 'endpoint',
      direction: () => undefined,
      projected_from: () => undefined,
      pierces_face: () => false,
      meeting_line: () => undefined,
      object: () => undefined,
      instance: () => undefined,
      element: () => 7n,
      element_kind: () => 'vertex',
      sketch: () => undefined,
      sketch_region: () => undefined,
      sketch_curve: () => undefined,
      free: () => {},
    }
    let hit = true
    const snapFn = vi.fn(() => (hit ? held : undefined))
    const svc = new SnapService({ snap: snapFn } as unknown as Scene)

    // Acquire the sticky endpoint.
    expect(svc.resolve(DOWN, 800, PERSPECTIVE_45).snap?.kind).toBe('endpoint')

    // Control (mirrors the precision-mode test above): with the hold
    // intact, losing it costs TWO queries — acquire, then the wider
    // resist-release query.
    hit = false
    snapFn.mockClear()
    expect(svc.resolve(DOWN, 800, PERSPECTIVE_45).snap?.kind).toBe('ground')
    expect(snapFn.mock.calls.length).toBe(2)

    // Re-acquire, then clear the hold explicitly — the next miss has
    // nothing left to resist-release, so only the acquire query runs.
    hit = true
    svc.resolve(DOWN, 800, PERSPECTIVE_45)
    svc.clearHold()
    hit = false
    snapFn.mockClear()
    expect(svc.resolve(DOWN, 800, PERSPECTIVE_45).snap?.kind).toBe('ground')
    expect(snapFn.mock.calls.length).toBe(1)
  })

  it('is safe to call with no held snap at all (idle taps, or two in a row)', () => {
    const svc = new SnapService({ snap: vi.fn(() => undefined) } as unknown as Scene)
    expect(() => {
      svc.clearHold()
      svc.clearHold()
    }).not.toThrow()
  })
})

describe('SnapService — apertureScaleOverride (finding 5b: tap-inspect vs. coarse-pointer widening)', () => {
  const held = {
    x: () => 1, y: () => 2, z: () => 3,
    kind: () => 'endpoint',
    direction: () => undefined,
    projected_from: () => undefined,
    pierces_face: () => false,
    meeting_line: () => undefined,
    object: () => undefined,
    instance: () => undefined,
    element: () => 7n,
    element_kind: () => 'vertex',
    sketch: () => undefined,
    sketch_region: () => undefined,
    sketch_curve: () => undefined,
    free: () => {},
  }

  it('omitted (every caller but dispatchSelectPick under readOnly): the acquire aperture is UNCHANGED', () => {
    const snapFn = vi.fn((..._args: unknown[]) => held)
    const svc = new SnapService({ snap: snapFn } as unknown as Scene)
    svc.resolve(DOWN, 800, PERSPECTIVE_45)
    const aperture = snapFn.mock.calls[0][6] as number
    expect(aperture).toBeCloseTo(pixelRadiusToAperture(SNAP_RADIUS_PX, 800, 45), 12)
  })

  it('an explicit override of 1 sends the UNSCALED (mouse-tuned) radius regardless of the platform\'s own coarse-pointer scale', () => {
    const snapFn = vi.fn((..._args: unknown[]) => held)
    const svc = new SnapService({ snap: snapFn } as unknown as Scene)
    // Passed positionally as the 8th argument (after offPlanePoints) —
    // dispatchSelectPick's own call site.
    svc.resolve(DOWN, 800, PERSPECTIVE_45, undefined, undefined, undefined, undefined, 1)
    const aperture = snapFn.mock.calls[0][6] as number
    // Exactly the SNAP_RADIUS_PX-derived aperture — no COARSE_POINTER_APERTURE_SCALE
    // multiplier applied, whatever isCoarsePointer() would otherwise report.
    expect(aperture).toBeCloseTo(pixelRadiusToAperture(SNAP_RADIUS_PX, 800, 45), 12)
  })

  it('a wider explicit override scales the aperture proportionally, same as the coarse-pointer path would', () => {
    const snapFn = vi.fn((..._args: unknown[]) => held)
    const svc = new SnapService({ snap: snapFn } as unknown as Scene)
    svc.resolve(DOWN, 800, PERSPECTIVE_45, undefined, undefined, undefined, undefined, 3)
    const aperture = snapFn.mock.calls[0][6] as number
    expect(aperture).toBeCloseTo(pixelRadiusToAperture(SNAP_RADIUS_PX * 3, 800, 45), 12)
  })
})

describe('SnapService — facesOnly (design v1.1 Lane E "Push/Pull face-first")', () => {
  // Ray origin already lies ON this plane (z=5), so a ray∩plane fallback
  // resolves at t=0 — the ray's own origin.
  const constraintPlane = { point: [0, 0, 5] as [number, number, number], normal: [0, 0, 1] as [number, number, number] }

  function fakeSnap(kind: string) {
    return {
      x: () => 1, y: () => 2, z: () => 5,
      kind: () => kind,
      direction: () => undefined,
      projected_from: () => undefined,
      pierces_face: () => false,
      meeting_line: () => undefined,
      object: () => undefined,
      instance: () => undefined,
      element: () => undefined,
      element_kind: () => undefined,
      sketch: () => undefined,
      sketch_region: () => undefined,
      sketch_curve: () => undefined,
      free: () => {},
    }
  }

  it('an on-face kernel candidate is honored as-is', () => {
    const scene = { snap: vi.fn(() => fakeSnap('on-face')) } as unknown as Scene
    const svc = new SnapService(scene)
    const { snap, fromKernel } = svc.resolve(
      DOWN, 800, PERSPECTIVE_45, undefined, undefined, constraintPlane, undefined, undefined, true,
    )
    expect(fromKernel).toBe(true)
    expect(snap?.kind).toBe('on-face')
  })

  it('an endpoint kernel winner is replaced by the constraintPlane fallback — inference\'s rank order can never let on-face outrank it, so facesOnly must override the winner itself', () => {
    const scene = { snap: vi.fn(() => fakeSnap('endpoint')) } as unknown as Scene
    const svc = new SnapService(scene)
    const { snap, fromKernel } = svc.resolve(
      DOWN, 800, PERSPECTIVE_45, undefined, undefined, constraintPlane, undefined, undefined, true,
    )
    expect(fromKernel).toBe(false)
    expect(snap?.kind).toBe('plane')
    expect(snap?.x).toBeCloseTo(0)
    expect(snap?.y).toBeCloseTo(0)
    expect(snap?.z).toBeCloseTo(5)
  })

  it('every non-on-face kind is replaced the same way (midpoint, on-edge, on-axis)', () => {
    for (const kind of ['midpoint', 'on-edge', 'on-axis']) {
      const scene = { snap: vi.fn(() => fakeSnap(kind)) } as unknown as Scene
      const svc = new SnapService(scene)
      const { snap } = svc.resolve(
        DOWN, 800, PERSPECTIVE_45, undefined, undefined, constraintPlane, undefined, undefined, true,
      )
      expect(snap?.kind).toBe('plane')
    }
  })

  it('a snap held STICKY from a PRIOR (non-facesOnly) query is never resurrected by the hysteresis "resist release" step', () => {
    // Acquire a genuine endpoint without facesOnly first — e.g. this same
    // tool's own state from a moment ago, or another tool's hover — so it
    // becomes SnapService's held sticky target.
    let phase: 'sticky' | 'facesOnly' = 'sticky'
    const snapFn = vi.fn(() => (phase === 'sticky' ? fakeSnap('endpoint') : undefined))
    const svc = new SnapService({ snap: snapFn } as unknown as Scene)
    expect(svc.resolve(DOWN, 800, PERSPECTIVE_45).snap?.kind).toBe('endpoint')

    // A facesOnly query where the kernel now returns nothing new: the
    // ordinary (non-facesOnly) hysteresis would re-query at the wider break
    // radius and re-latch the stale endpoint. Under facesOnly that step must
    // be skipped entirely — the query falls straight through to the
    // constraintPlane fallback instead.
    phase = 'facesOnly'
    snapFn.mockClear()
    const { snap, fromKernel } = svc.resolve(
      DOWN, 800, PERSPECTIVE_45, undefined, undefined, constraintPlane, undefined, undefined, true,
    )
    expect(fromKernel).toBe(false)
    expect(snap?.kind).toBe('plane')
    expect(snapFn.mock.calls.length).toBe(1) // only the (missed) acquire query — no hold-resist re-query
  })
})

describe('snapJsToSnap — pierces_face', () => {
  const snapJs = (pierces: boolean) => ({
    x: () => 1, y: () => 0.9, z: () => 0.6,
    kind: () => 'intersection',
    direction: () => undefined,
    projected_from: () => undefined,
    pierces_face: () => pierces,
    meeting_line: () => undefined,
    object: () => 7n,
    instance: () => undefined,
    element: () => 3n,
    element_kind: () => 'edge',
    sketch: () => undefined,
    sketch_region: () => undefined,
    sketch_curve: () => undefined,
    free: () => {},
  })

  it('carries the kernel’s pierced-face flag onto the snap, true and false alike', () => {
    const pierced = new SnapService({ snap: vi.fn(() => snapJs(true)) } as unknown as Scene)
    const snap = pierced.resolve(DOWN, 800, PERSPECTIVE_45).snap
    expect(snap?.kind).toBe('intersection')
    expect(snap?.piercesFace).toBe(true)

    const crossing = new SnapService({ snap: vi.fn(() => snapJs(false)) } as unknown as Scene)
    expect(crossing.resolve(DOWN, 800, PERSPECTIVE_45).snap?.piercesFace).toBe(false)
  })
})

describe('snapJsToSnap — meeting_line', () => {
  const snapJs = (line: number[] | undefined) => ({
    x: () => 1, y: () => 0.8, z: () => 0.5,
    kind: () => (line !== undefined ? 'on-intersection-line' : 'on-face'),
    direction: () => undefined,
    projected_from: () => undefined,
    pierces_face: () => false,
    meeting_line: () => line,
    object: () => 7n,
    instance: () => undefined,
    element: () => 3n,
    // The kernel never reports a snap on a meeting line as a face pick.
    element_kind: () => (line !== undefined ? 'meeting-line' : 'face'),
    sketch: () => undefined,
    sketch_region: () => undefined,
    sketch_curve: () => undefined,
    free: () => {},
  })

  it('splits the kernel’s six numbers into the line’s two ends, and leaves a plain face without one', () => {
    const onLine = new SnapService({ snap: vi.fn(() => snapJs([1, 0.6, 0.5, 1, 1.4, 0.5])) } as unknown as Scene)
    const snap = onLine.resolve(DOWN, 800, PERSPECTIVE_45).snap
    expect(snap?.kind).toBe('on-intersection-line')
    expect(snap?.meetingLine).toEqual([[1, 0.6, 0.5], [1, 1.4, 0.5]])
    // Edge-class, not a face pick; the first face stays the owner of record.
    expect(snap?.elementKind).toBe('meeting-line')
    expect(snap?.object).toBe(7n)
    expect(snap?.element).toBe(3n)

    const onFace = new SnapService({ snap: vi.fn(() => snapJs(undefined)) } as unknown as Scene)
    const face = onFace.resolve(DOWN, 800, PERSPECTIVE_45).snap
    expect(face?.meetingLine).toBeUndefined()
    expect(face?.elementKind).toBe('face')
  })

  it('holds a meeting line only while the release query still finds that same line, not another on the same face', () => {
    // A slab through a wall: its top and bottom lines both name the wall's
    // face as the first face, and only their ends tell them apart.
    const top = [1, 0.6, 0.5, 1, 1.4, 0.5]
    const bottom = [1, 0.6, 0.4, 1, 1.4, 0.4]
    let acquire: number[] | undefined = top
    let release: number[] | undefined = top
    const snapFn = vi.fn((...args: unknown[]) => {
      // The release query is the one that passes a soft-axis scale.
      const line = args[12] === null || args[12] === undefined ? acquire : release
      return snapJs(line)
    })
    const svc = new SnapService({ snap: snapFn } as unknown as Scene)
    expect(svc.resolve(DOWN, 800, PERSPECTIVE_45).snap?.meetingLine).toEqual([[1, 0.6, 0.5], [1, 1.4, 0.5]])

    // The cursor drifts off the top line onto the wall: the acquire query
    // finds only the face, the release query still the top line. Held.
    acquire = undefined
    expect(svc.resolve(DOWN, 800, PERSPECTIVE_45).snap?.kind).toBe('on-intersection-line')

    // The release query now finds the bottom line instead: a different
    // target, so the hold lets go to the face.
    release = bottom
    const snap = svc.resolve(DOWN, 800, PERSPECTIVE_45).snap
    expect(snap?.kind).toBe('on-face')
    expect(snap?.meetingLine).toBeUndefined()
  })
})
