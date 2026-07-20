import { describe, expect, it } from 'vitest'
import {
  createSectionPlane,
  offsetSectionPlane,
  SectionManager,
  toggleSectionPlaneActive,
  type SectionPlane,
} from './sectionManager'

/**
 * The three.js CLIP plane the renderer builds from a section (see
 * `SceneRenderer.setSectionPlane`): its normal is the NEGATED section normal,
 * so `constant = -clipNormal·origin = +sectionNormal·origin`. three.js keeps
 * fragments where `distanceToPoint >= 0` and discards those `< 0`, so with the
 * negated normal the `-sectionNormal` interior side is KEPT and the
 * `+sectionNormal` near/outer side (the one the section normal points toward)
 * is REMOVED (DESIGN §2/§3 as corrected in the clip-side fix).
 */
function clipConstant(plane: SectionPlane): number {
  const [nx, ny, nz] = plane.normal
  const [ox, oy, oz] = plane.origin
  return nx * ox + ny * oy + nz * oz // = -(-normal)·origin
}

function clipDistanceToPoint(plane: SectionPlane, point: [number, number, number]): number {
  const [nx, ny, nz] = plane.normal
  return -(nx * point[0] + ny * point[1] + nz * point[2]) + clipConstant(plane)
}

describe('createSectionPlane', () => {
  it('builds a plane from a face-like origin/normal, active by default', () => {
    const plane = createSectionPlane([1, 2, 3], [0, 0, 1])
    expect(plane.origin).toEqual([1, 2, 3])
    expect(plane.normal).toEqual([0, 0, 1])
    expect(plane.active).toBe(true)
  })

  it('normalizes a non-unit normal', () => {
    const plane = createSectionPlane([0, 0, 0], [0, 0, 5])
    expect(plane.normal).toEqual([0, 0, 1])
  })

  it('falls back to +Z for a degenerate (near-zero) normal', () => {
    const plane = createSectionPlane([0, 0, 0], [0, 0, 0])
    expect(plane.normal).toEqual([0, 0, 1])
  })

  it('the +normal (near/outer) side is REMOVED, the -normal (interior) side is kept', () => {
    const plane = createSectionPlane([0, 0, 0], [1, 0, 0])
    // A point one meter toward +normal (the side you clicked) is CUT AWAY (< 0).
    expect(clipDistanceToPoint(plane, [1, 0, 0])).toBeLessThan(0)
    // A point one meter toward -normal (the interior) is KEPT (>= 0).
    expect(clipDistanceToPoint(plane, [-1, 0, 0])).toBeGreaterThan(0)
    // The origin itself is exactly on the plane.
    expect(clipDistanceToPoint(plane, [0, 0, 0])).toBeCloseTo(0, 10)
  })

  it('does not mutate the input arrays (defensive copy)', () => {
    const origin: [number, number, number] = [1, 2, 3]
    const normal: [number, number, number] = [0, 0, 1]
    const plane = createSectionPlane(origin, normal)
    plane.origin[0] = 999
    expect(origin[0]).toBe(1)
  })
})

describe('offsetSectionPlane', () => {
  it('moves the origin along the normal by the given signed distance', () => {
    const plane = createSectionPlane([0, 0, 0], [0, 0, 1])
    const offset = offsetSectionPlane(plane, 2.5)
    expect(offset.origin).toEqual([0, 0, 2.5])
    expect(offset.normal).toEqual([0, 0, 1])
  })

  it('moves the clip-plane constant by exactly the typed distance', () => {
    const plane = createSectionPlane([0, 0, 0], [0, 0, 1])
    const before = clipConstant(plane)
    const offset = offsetSectionPlane(plane, 3)
    const after = clipConstant(offset)
    // clipConstant = +sectionNormal·origin, so moving +3 along the normal
    // increases it by exactly 3 (the clip sweeps 3 m along the normal).
    expect(after - before).toBeCloseTo(3, 10)
  })

  it('supports a negative (backward) offset', () => {
    const plane = createSectionPlane([1, 1, 1], [1, 0, 0])
    const offset = offsetSectionPlane(plane, -4)
    expect(offset.origin).toEqual([-3, 1, 1])
  })

  it('offsets along a non-axis-aligned normal', () => {
    const plane = createSectionPlane([0, 0, 0], [1, 1, 0]) // normalizes to [√2/2, √2/2, 0]
    const offset = offsetSectionPlane(plane, Math.SQRT2)
    expect(offset.origin[0]).toBeCloseTo(1, 10)
    expect(offset.origin[1]).toBeCloseTo(1, 10)
    expect(offset.origin[2]).toBeCloseTo(0, 10)
  })

  it('does not mutate the input plane', () => {
    const plane = createSectionPlane([0, 0, 0], [0, 0, 1])
    offsetSectionPlane(plane, 5)
    expect(plane.origin).toEqual([0, 0, 0])
  })

  it('preserves active', () => {
    const plane = { ...createSectionPlane([0, 0, 0], [0, 0, 1]), active: false }
    expect(offsetSectionPlane(plane, 1).active).toBe(false)
  })
})

describe('toggleSectionPlaneActive', () => {
  it('flips active from true to false', () => {
    const plane = createSectionPlane([0, 0, 0], [0, 0, 1])
    expect(plane.active).toBe(true)
    expect(toggleSectionPlaneActive(plane).active).toBe(false)
  })

  it('flips active from false to true', () => {
    const plane = { ...createSectionPlane([0, 0, 0], [0, 0, 1]), active: false }
    expect(toggleSectionPlaneActive(plane).active).toBe(true)
  })

  it('leaves origin/normal unchanged', () => {
    const plane = createSectionPlane([1, 2, 3], [0, 1, 0])
    const toggled = toggleSectionPlaneActive(plane)
    expect(toggled.origin).toEqual(plane.origin)
    expect(toggled.normal).toEqual(plane.normal)
  })

  it('does not mutate the input plane', () => {
    const plane = createSectionPlane([0, 0, 0], [0, 0, 1])
    toggleSectionPlaneActive(plane)
    expect(plane.active).toBe(true)
  })
})

describe('SectionManager', () => {
  it('starts with no section', () => {
    const mgr = new SectionManager()
    expect(mgr.current).toBeNull()
  })

  it('place() creates an active section', () => {
    const mgr = new SectionManager()
    const plane = mgr.place([0, 0, 0], [0, 0, 1])
    expect(mgr.current).toEqual(plane)
    expect(plane.active).toBe(true)
  })

  it('place() again REPLACES the previous section (one at a time — DESIGN §1)', () => {
    const mgr = new SectionManager()
    mgr.place([0, 0, 0], [0, 0, 1])
    const second = mgr.place([5, 0, 0], [1, 0, 0])
    expect(mgr.current).toEqual(second)
    expect(mgr.current?.origin).toEqual([5, 0, 0])
  })

  it('offset() moves the current plane and returns it', () => {
    const mgr = new SectionManager()
    mgr.place([0, 0, 0], [0, 0, 1])
    const moved = mgr.offset(2)
    expect(moved?.origin).toEqual([0, 0, 2])
    expect(mgr.current?.origin).toEqual([0, 0, 2])
  })

  it('offset() on an empty manager is a no-op returning null', () => {
    const mgr = new SectionManager()
    expect(mgr.offset(2)).toBeNull()
    expect(mgr.current).toBeNull()
  })

  it('toggleActive() flips active and returns the new plane', () => {
    const mgr = new SectionManager()
    mgr.place([0, 0, 0], [0, 0, 1])
    expect(mgr.toggleActive()?.active).toBe(false)
    expect(mgr.current?.active).toBe(false)
    expect(mgr.toggleActive()?.active).toBe(true)
  })

  it('toggleActive() on an empty manager is a no-op returning null', () => {
    const mgr = new SectionManager()
    expect(mgr.toggleActive()).toBeNull()
  })

  it('setPlane() replaces the current plane outright (drag-offset commit)', () => {
    const mgr = new SectionManager()
    mgr.place([0, 0, 0], [0, 0, 1])
    const committed: SectionPlane = { origin: [0, 0, 7], normal: [0, 0, 1], active: true }
    expect(mgr.setPlane(committed)).toEqual(committed)
    expect(mgr.current).toEqual(committed)
  })

  it('delete() clears the section — the model returns to whole', () => {
    const mgr = new SectionManager()
    mgr.place([0, 0, 0], [0, 0, 1])
    mgr.delete()
    expect(mgr.current).toBeNull()
  })

  it('delete() on an empty manager is a harmless no-op', () => {
    const mgr = new SectionManager()
    expect(() => mgr.delete()).not.toThrow()
    expect(mgr.current).toBeNull()
  })
})
