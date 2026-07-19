import { describe, it, expect, vi } from 'vitest'
import {
  groundDrawPlane,
  planeFromSketch,
  axisDrawPlane,
  pointOnPlane,
  planeKey,
  isGroundPlane,
  drawPlaneCue,
  SketchPickCache,
  type DrawPlane,
} from './drawPlane'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

function makeScene(sketchPlanes: Map<bigint, Float64Array | undefined>): WasmScene {
  return {
    sketch_plane: vi.fn((h: bigint) => sketchPlanes.get(h)),
    pick_sketch: vi.fn(),
  } as unknown as WasmScene
}

describe('groundDrawPlane', () => {
  it('returns the exact literal ground plane', () => {
    expect(groundDrawPlane()).toEqual({
      origin: [0, 0, 0],
      normal: [0, 0, 1],
      u: [1, 0, 0],
      v: [0, 1, 0],
      ground: true,
    })
  })
})

describe('isGroundPlane', () => {
  it('true for the origin/+Z ground plane', () => {
    expect(isGroundPlane([0, 0, 0], [0, 0, 1])).toBe(true)
  })

  it('true for a flipped-but-coincident (-Z through the origin) plane — orientation-free', () => {
    expect(isGroundPlane([0, 0, 0], [0, 0, -1])).toBe(true)
  })

  it('true for any in-plane point, not just the literal origin', () => {
    expect(isGroundPlane([5, -3, 0], [0, 0, 1])).toBe(true)
  })

  it('false for a plane tilted off Z', () => {
    expect(isGroundPlane([0, 0, 0], [0, -1, 0])).toBe(false)
  })

  it('false for a plane parallel to ground but offset in Z', () => {
    expect(isGroundPlane([0, 0, 2], [0, 0, 1])).toBe(false)
  })
})

describe('planeFromSketch', () => {
  it('returns null for a stale/hidden sketch (sketch_plane undefined)', () => {
    const scene = makeScene(new Map([[7n, undefined]]))
    expect(planeFromSketch(scene, 7n)).toBeNull()
  })

  it('returns the EXACT groundDrawPlane() when the sketch plane is the ground plane', () => {
    const scene = makeScene(new Map([[7n, new Float64Array([0, 0, 0, 0, 0, 1])]]))
    expect(planeFromSketch(scene, 7n)).toEqual(groundDrawPlane())
  })

  it('returns the EXACT groundDrawPlane() for a flipped-but-coincident ground sketch plane', () => {
    const scene = makeScene(new Map([[7n, new Float64Array([0, 0, 0, 0, 0, -1])]]))
    expect(planeFromSketch(scene, 7n)).toEqual(groundDrawPlane())
  })

  it('returns a non-ground plane with a facePlaneBasis-derived basis, ground: false', () => {
    // A sketch stood upright: y = 0 plane, -Y normal (rotated 90 deg about X).
    const scene = makeScene(new Map([[7n, new Float64Array([0, 0, 0, 0, -1, 0])]]))
    const plane = planeFromSketch(scene, 7n)
    expect(plane).not.toBeNull()
    expect(plane!.ground).toBe(false)
    expect(plane!.origin).toEqual([0, 0, 0])
    expect(plane!.normal).toEqual([0, -1, 0])
    // u, v, normal orthonormal and right-handed.
    const { u, v, normal } = plane!
    expect(Math.hypot(...u)).toBeCloseTo(1, 9)
    expect(Math.hypot(...v)).toBeCloseTo(1, 9)
    const cross: [number, number, number] = [
      u[1] * v[2] - u[2] * v[1],
      u[2] * v[0] - u[0] * v[2],
      u[0] * v[1] - u[1] * v[0],
    ]
    expect(cross[0]).toBeCloseTo(normal[0], 9)
    expect(cross[1]).toBeCloseTo(normal[1], 9)
    expect(cross[2]).toBeCloseTo(normal[2], 9)
  })
})

describe('axisDrawPlane', () => {
  it('axis 2 (Z) through a z=0 point returns the exact ground frame, ground: true', () => {
    const plane = axisDrawPlane(2, [3, -2, 0])
    expect(plane.origin).toEqual([3, -2, 0])
    expect(plane.normal).toEqual([0, 0, 1])
    expect(plane.u).toEqual([1, 0, 0])
    expect(plane.v).toEqual([0, 1, 0])
    expect(plane.ground).toBe(true)
  })

  it('axis 2 (Z) through a point off z=0 is a non-ground plane', () => {
    const plane = axisDrawPlane(2, [0, 0, 5])
    expect(plane.ground).toBe(false)
    expect(plane.normal).toEqual([0, 0, 1])
  })

  it('axis 0 (X) is a vertical plane through the given point, ground: false', () => {
    const plane = axisDrawPlane(0, [1, 2, 3])
    expect(plane.origin).toEqual([1, 2, 3])
    expect(plane.normal).toEqual([1, 0, 0])
    expect(plane.ground).toBe(false)
  })

  it('axis 1 (Y) is a vertical plane through the given point, ground: false', () => {
    const plane = axisDrawPlane(1, [1, 2, 3])
    expect(plane.normal).toEqual([0, 1, 0])
    expect(plane.ground).toBe(false)
  })
})

describe('pointOnPlane', () => {
  it('intersects a straight-down ray with the ground plane', () => {
    const ray: Ray = { origin: [1, 2, 5], direction: [0, 0, -1] }
    const p = pointOnPlane(ray, groundDrawPlane())
    expect(p).toEqual([1, 2, 0])
  })

  it('returns null for a ray parallel to the plane', () => {
    const ray: Ray = { origin: [0, 0, 5], direction: [1, 0, 0] }
    const p = pointOnPlane(ray, groundDrawPlane())
    expect(p).toBeNull()
  })
})

describe('planeKey', () => {
  it('the ground plane and a coplanar-but-flipped-normal plane produce the same key', () => {
    const a = groundDrawPlane()
    const b: DrawPlane = { origin: [0, 0, 0], normal: [0, 0, -1], u: [1, 0, 0], v: [0, -1, 0], ground: false }
    expect(planeKey(a)).toBe(planeKey(b))
  })

  it('the ground plane and a plane through a DIFFERENT z=0 point produce the same key', () => {
    const a = groundDrawPlane()
    const b: DrawPlane = { origin: [7, -3, 0], normal: [0, 0, 1], u: [1, 0, 0], v: [0, 1, 0], ground: false }
    expect(planeKey(a)).toBe(planeKey(b))
  })

  it('a plane offset in Z from the ground plane produces a DIFFERENT key', () => {
    const a = groundDrawPlane()
    const b: DrawPlane = { origin: [0, 0, 2], normal: [0, 0, 1], u: [1, 0, 0], v: [0, 1, 0], ground: false }
    expect(planeKey(a)).not.toBe(planeKey(b))
  })

  it('a tilted plane and its flipped-normal twin produce the same key', () => {
    const a: DrawPlane = { origin: [0, 0, 0], normal: [0, -1, 0], u: [1, 0, 0], v: [0, 0, -1], ground: false }
    const b: DrawPlane = { origin: [0, 0, 0], normal: [0, 1, 0], u: [1, 0, 0], v: [0, 0, 1], ground: false }
    expect(planeKey(a)).toBe(planeKey(b))
  })

  it('two distinct tilted planes produce different keys', () => {
    const a: DrawPlane = { origin: [0, 0, 0], normal: [0, -1, 0], u: [1, 0, 0], v: [0, 0, -1], ground: false }
    const b: DrawPlane = { origin: [0, 0, 0], normal: [1, 0, 0], u: [0, 1, 0], v: [0, 0, -1], ground: false }
    expect(planeKey(a)).not.toBe(planeKey(b))
  })
})

describe('drawPlaneCue', () => {
  const TILTED: DrawPlane = { origin: [0, 0, 0], normal: [0, -1, 0], u: [1, 0, 0], v: [0, 0, -1], ground: false }

  it('null when nothing is anchored and no idle lock is active', () => {
    expect(drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: null, idleHover: null })).toBeNull()
  })

  it('null when anchored on the GROUND plane (the world grid already covers it)', () => {
    expect(
      drawPlaneCue({ anchoredPlane: groundDrawPlane(), anchoredThrough: [1, 2, 0], idleLock: null, idleHover: null }),
    ).toBeNull()
  })

  it('the frozen plane, through the anchor, when anchored on a NON-ground plane', () => {
    const cue = drawPlaneCue({ anchoredPlane: TILTED, anchoredThrough: [3, 0, 5], idleLock: null, idleHover: null })
    expect(cue).toEqual({ plane: TILTED, through: [3, 0, 5] })
  })

  it('anchored takes priority over an idle lock (mutually exclusive in practice, but anchored wins)', () => {
    const cue = drawPlaneCue({ anchoredPlane: TILTED, anchoredThrough: [3, 0, 5], idleLock: 2, idleHover: [9, 9, 9] })
    expect(cue).toEqual({ plane: TILTED, through: [3, 0, 5] })
  })

  it('null when idle-locked but no hover has landed yet', () => {
    expect(drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: 0, idleHover: null })).toBeNull()
  })

  it('null when idle and a hover exists but no lock is active', () => {
    expect(
      drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: null, idleHover: [1, 2, 3] }),
    ).toBeNull()
  })

  it('the locked axis plane through the tracked hover point, when idle-locked with a hover', () => {
    const cue = drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: 0, idleHover: [4, 5, 6] })
    expect(cue).toEqual({ plane: axisDrawPlane(0, [4, 5, 6]), through: [4, 5, 6] })
  })

  it('null when a Z-axis lock through a z=0 hover resolves to the exact ground plane', () => {
    // axisDrawPlane(2, [x,y,0]) returns groundDrawPlane()'s exact frame — the
    // same "reuses the ground path" case the click-time commit special-cases
    // (begin_ground_sketch, not begin_sketch_on_plane). No cue either.
    expect(
      drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: 2, idleHover: [1, 2, 0] }),
    ).toBeNull()
  })

  it('a Z-axis lock through a hover OFF z=0 does produce a cue (a horizontal plane elevated off ground)', () => {
    const cue = drawPlaneCue({ anchoredPlane: null, anchoredThrough: null, idleLock: 2, idleHover: [1, 2, 5] })
    expect(cue).toEqual({ plane: axisDrawPlane(2, [1, 2, 5]), through: [1, 2, 5] })
  })
})

describe('SketchPickCache', () => {
  const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

  it('memoizes pick_sketch per Ray reference', () => {
    const scene = { pick_sketch: vi.fn(() => 9n) } as unknown as WasmScene
    const cache = new SketchPickCache()

    expect(cache.pickFor(scene, RAY)).toBe(9n)
    expect(cache.pickFor(scene, RAY)).toBe(9n)
    expect(scene.pick_sketch).toHaveBeenCalledTimes(1)

    cache.pickFor(scene, { ...RAY })
    expect(scene.pick_sketch).toHaveBeenCalledTimes(2)
  })

  it('caches a miss (undefined) as null', () => {
    const scene = { pick_sketch: vi.fn(() => undefined) } as unknown as WasmScene
    const cache = new SketchPickCache()
    expect(cache.pickFor(scene, RAY)).toBeNull()
    expect(cache.pickFor(scene, RAY)).toBeNull()
    expect(scene.pick_sketch).toHaveBeenCalledTimes(1)
  })
})
