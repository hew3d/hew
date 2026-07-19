/**
 * SnapService unit tests — the fallback tier (kernel snap absent/thrown).
 * Kernel-candidate tiers (acquire/hysteresis) are exercised indirectly by
 * every tool test that drives snapping; these pin the ray∩plane fallback
 * added for sketches on any plane (Phase 1, the sketch-planes design §3):
 * a supplied `constraintPlane` is the fallback target instead of ground.
 */
import { describe, it, expect, vi } from 'vitest'
import { SnapService } from './snapService'
import type { Scene } from '../wasm/pkg/wasm_api.js'
import type { Ray } from './math'

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
