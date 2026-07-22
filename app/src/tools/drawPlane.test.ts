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
  rayLandsOnSketch,
  resolveIdleDrawTarget,
  resolveClickDrawTarget,
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

// ------------------------------------------------- sketch-mode adoption gate

/**
 * The regression this gate exists for, in numbers. A vertical sketch (y = 0)
 * seen from a 3/4 camera at (8, 6, 8) is met at ~28° incidence, so
 * `pick_sketch`'s cone — which measures the PERPENDICULAR distance from the
 * ray axis — reports a 0.245 m miss for an edge that is 0.52 m away IN THE
 * PLANE. The cone's half-angle is 0.02 rad, so the pick "hits" (0.245 / 12.8
 * = 0.0192 rad) even though the user is pointing half a metre off the sketch,
 * at the world origin on the ground.
 */
describe('rayLandsOnSketch', () => {
  const WALL_SKETCH = 7n
  /** The y = 0 wall plane, +Y normal. */
  const WALL: DrawPlane = { origin: [0, 0, 0], normal: [0, 1, 0], u: [0, 0, 1], v: [1, 0, 0], ground: false }
  /** A 0.6 m square standing on the wall plane, spanning x ∈ [-0.3, 0.3], z ∈ [0.4, 1]. */
  const SQUARE = new Float32Array([
    -0.3, 0, 0.4, 0.3, 0, 0.4,
    0.3, 0, 0.4, 0.3, 0, 1,
    0.3, 0, 1, -0.3, 0, 1,
    -0.3, 0, 1, -0.3, 0, 0.4,
  ])

  function wallScene(lines: Float32Array | (() => never) = SQUARE): WasmScene {
    return {
      sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 1, 0])),
      pick_sketch: vi.fn(() => WALL_SKETCH),
      sketch_lines: vi.fn(() => (typeof lines === 'function' ? lines() : lines)),
    } as unknown as WasmScene
  }

  /** A ray from the 3/4 camera through `target`. */
  function rayThrough(target: [number, number, number]): Ray {
    const o: [number, number, number] = [8, 6, 8]
    return { origin: o, direction: [target[0] - o[0], target[1] - o[1], target[2] - o[2]] }
  }

  it('rejects the grazing near-miss: aiming at the world origin does NOT land on the standing square', () => {
    expect(rayLandsOnSketch(wallScene(), WALL_SKETCH, WALL, rayThrough([0, 0, 0]))).toBe(false)
  })

  it('accepts a ray aimed at the square\'s own edge', () => {
    // Distance is measured to the sketch's EDGES, matching what
    // `pick_sketch`'s cone tests — a ray through the middle of a closed
    // loop is never a pick_sketch hit in the first place, so the gate never
    // sees it.
    expect(rayLandsOnSketch(wallScene(), WALL_SKETCH, WALL, rayThrough([0, 0, 0.4]))).toBe(true)
  })

  it('accepts a ray just outside an edge but within the cone radius at that depth', () => {
    // Pierce point (0, 0, 0.35) — 0.05 m below the bottom edge. The cone
    // radius at ~12.9 m is 0.02 × 12.9 ≈ 0.26 m, so this still lands.
    expect(rayLandsOnSketch(wallScene(), WALL_SKETCH, WALL, rayThrough([0, 0, 0.35]))).toBe(true)
  })

  it('rejects a ray PARALLEL to the plane — it never pierces it at all', () => {
    const parallel: Ray = { origin: [5, 0, 0.4], direction: [-1, 0, 0] }
    expect(rayLandsOnSketch(wallScene(), WALL_SKETCH, WALL, parallel)).toBe(false)
  })

  it('rejects a sketch whose handle went stale (sketch_lines throws)', () => {
    const stale = wallScene(() => { throw new Error('UnknownSketch') })
    expect(rayLandsOnSketch(stale, WALL_SKETCH, WALL, rayThrough([0, 0, 0.4]))).toBe(false)
  })

  it('rejects an empty sketch — no geometry to land on', () => {
    expect(rayLandsOnSketch(wallScene(new Float32Array([])), WALL_SKETCH, WALL, rayThrough([0, 0, 0.4]))).toBe(false)
  })
})

describe('resolveIdleDrawTarget', () => {
  const WALL_SKETCH = 7n
  const SQUARE = new Float32Array([
    -0.3, 0, 0.4, 0.3, 0, 0.4,
    0.3, 0, 0.4, 0.3, 0, 1,
    0.3, 0, 1, -0.3, 0, 1,
    -0.3, 0, 1, -0.3, 0, 0.4,
  ])

  /** `pick: null` means `pick_sketch` MISSES (returns undefined). */
  function scene(plane: Float64Array, pick: bigint | null = WALL_SKETCH): WasmScene {
    return {
      sketch_plane: vi.fn(() => plane),
      pick_sketch: vi.fn(() => pick ?? undefined),
      sketch_lines: vi.fn(() => SQUARE),
    } as unknown as WasmScene
  }

  function rayThrough(target: [number, number, number]): Ray {
    const o: [number, number, number] = [8, 6, 8]
    return { origin: o, direction: [target[0] - o[0], target[1] - o[1], target[2] - o[2]] }
  }

  const WALL_PLANE_ARR = new Float64Array([0, 0, 0, 0, 1, 0])

  it('adopts the hovered sketch when the ray lands on it', () => {
    const resolved = resolveIdleDrawTarget(scene(WALL_PLANE_ARR), new SketchPickCache(), rayThrough([0, 0, 0.4]))
    expect(resolved.target).toEqual({ kind: 'existing', handle: WALL_SKETCH })
    expect(resolved.plane.ground).toBe(false)
  })

  it('falls back to the GROUND plane on a grazing pick the ray does not land on', () => {
    const resolved = resolveIdleDrawTarget(scene(WALL_PLANE_ARR), new SketchPickCache(), rayThrough([0, 0, 0]))
    expect(resolved.plane).toEqual(groundDrawPlane())
    expect(resolved.target).toEqual({ kind: 'plane', plane: groundDrawPlane() })
  })

  it('falls back to the ground plane when nothing is picked', () => {
    const resolved = resolveIdleDrawTarget(
      scene(WALL_PLANE_ARR, null), new SketchPickCache(), rayThrough([0, 0, 0.4]),
    )
    expect(resolved.target).toEqual({ kind: 'plane', plane: groundDrawPlane() })
  })

  it('never adopts a sketch that is itself on the ground plane', () => {
    const groundArr = new Float64Array([0, 0, 0, 0, 0, 1])
    const resolved = resolveIdleDrawTarget(scene(groundArr), new SketchPickCache(), rayThrough([0, 0, 0]))
    expect(resolved.target).toEqual({ kind: 'plane', plane: groundDrawPlane() })
  })
})

describe('resolveClickDrawTarget', () => {
  const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }
  const scene = { pick_sketch: vi.fn(() => undefined) } as unknown as WasmScene

  it('an active idle plane lock beats sketch/ground resolution', () => {
    const resolved = resolveClickDrawTarget(
      scene, new SketchPickCache(), 1, { x: 2, y: 3, z: 4, kind: 'endpoint' }, RAY,
    )
    expect(resolved).toEqual({ plane: axisDrawPlane(1, [2, 3, 4]), target: { kind: 'plane', plane: axisDrawPlane(1, [2, 3, 4]) } })
  })

  it('a lock with no snap yet resolves to null — nothing to click through', () => {
    expect(resolveClickDrawTarget(scene, new SketchPickCache(), 1, null, RAY)).toBeNull()
  })

  it('with no lock it defers to resolveIdleDrawTarget', () => {
    const resolved = resolveClickDrawTarget(scene, new SketchPickCache(), null, null, RAY)
    expect(resolved).toEqual({ plane: groundDrawPlane(), target: { kind: 'plane', plane: groundDrawPlane() } })
  })
})
