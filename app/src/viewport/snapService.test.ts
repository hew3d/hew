/**
 * SnapService unit tests — the fallback tier (kernel snap absent/thrown).
 * Kernel-candidate tiers (acquire/hysteresis) are exercised indirectly by
 * every tool test that drives snapping; these pin the ray∩plane fallback
 * added for sketches on any plane (Phase 1, the sketch-planes design §3):
 * a supplied `constraintPlane` is the fallback target instead of ground.
 */
import { describe, it, expect, vi } from 'vitest'
import { SnapService, SNAP_RADIUS_PX } from './snapService'
import type { Scene } from '../wasm/pkg/wasm_api.js'
import { pixelRadiusToAperture, type Ray } from './math'

/** A ray straight down -Z from above the origin. */
const DOWN: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

/** A fake Scene whose `snap()` always misses (undefined) — every resolve()
 *  call falls through to the fallback tier under test. */
function fakeScene(): Scene {
  return { snap: vi.fn(() => undefined) } as unknown as Scene
}

describe('SnapService — fallback tier', () => {
  it('without a constraintPlane, falls back to ground (kind "ground")', () => {
    const svc = new SnapService(fakeScene())
    const { snap, fromKernel } = svc.resolve(DOWN, 800, 45)
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
    const { snap, fromKernel } = svc.resolve(ray, 800, 45, undefined, undefined, constraintPlane)
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
    const { snap, fromKernel } = svc.resolve(ray, 800, 45, undefined, undefined, constraintPlane)
    expect(fromKernel).toBe(false)
    expect(snap).toBeNull()
  })
})

describe('SnapService — precision mode', () => {
  it('passes the precision flag through to Scene.snap() as the trailing argument', () => {
    const scene = fakeScene()
    const svc = new SnapService(scene)
    const snapFn = scene.snap as unknown as ReturnType<typeof vi.fn>

    svc.resolve(DOWN, 800, 45)
    // The kernel owns the weighting; only this boolean crosses the boundary.
    expect(snapFn.mock.calls[0].at(-1)).toBe(false)

    expect(svc.setPrecision(true)).toBe(true)
    expect(svc.isPrecision()).toBe(true)
    svc.resolve(DOWN, 800, 45)
    expect(snapFn.mock.calls.at(-1)?.at(-1)).toBe(true)

    expect(svc.setPrecision(false)).toBe(true)
    svc.resolve(DOWN, 800, 45)
    expect(snapFn.mock.calls.at(-1)?.at(-1)).toBe(false)
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
    expect(svc.resolve(DOWN, 800, 45).snap?.kind).toBe('endpoint')

    // Control: with the mode unchanged, losing the endpoint costs TWO queries
    // (acquire, then the wider release-resisting one).
    hit = false
    snapFn.mockClear()
    expect(svc.resolve(DOWN, 800, 45).snap?.kind).toBe('ground')
    expect(snapFn.mock.calls.length).toBe(2)

    // Re-acquire, then toggle: the held snap is gone, so one query only.
    hit = true
    svc.resolve(DOWN, 800, 45)
    svc.setPrecision(true)
    hit = false
    snapFn.mockClear()
    expect(svc.resolve(DOWN, 800, 45).snap?.kind).toBe('ground')
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
    expect(svc.resolve(DOWN, 800, 45).snap?.kind).toBe('center')

    // Cursor drifts off A onto B's neighbourhood. B is not the same target,
    // so the result RELEASES (ground fallback), and is certainly not B.
    phase = 'missThenB'
    const released = svc.resolve(DOWN, 800, 45).snap
    expect(released?.kind).toBe('ground')
    expect(released?.sketchCurve).toBeUndefined()
  })
})
