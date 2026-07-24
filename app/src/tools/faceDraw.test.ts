import { describe, it, expect, vi } from 'vitest'
import { FacePickCache, defaultFaceEligible, worldFaceNormal } from './faceDraw'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makePick(object: bigint, face: bigint, instance?: bigint) {
  return {
    object: () => object,
    face: () => face,
    instance: () => instance,
    free: vi.fn(),
  }
}

/** node_parent(0, id) — `parents` maps a grouped object to its group id. */
function makeWasmScene(opts: {
  pick?: ReturnType<typeof makePick>
  parents?: Map<bigint, bigint>
} = {}): WasmScene {
  return {
    pick_face: vi.fn(() => opts.pick),
    node_parent: vi.fn((_kind: number, id: bigint) => opts.parents?.get(id)),
  } as unknown as WasmScene
}

describe('defaultFaceEligible (plain objects are directly drawable)', () => {
  it('top level: a plain, ungrouped object is eligible', () => {
    const scene = makeWasmScene()
    expect(defaultFaceEligible(scene, null, 7n, undefined)).toBe(true)
  })

  it('top level: an object inside a group needs the explicit edit step', () => {
    const scene = makeWasmScene({ parents: new Map([[7n, 3n]]) })
    expect(defaultFaceEligible(scene, null, 7n, undefined)).toBe(false)
  })

  it('top level: instanced (component) geometry needs the explicit edit step', () => {
    const scene = makeWasmScene()
    expect(defaultFaceEligible(scene, null, 7n, 12n)).toBe(false)
  })

  it('inside an entered object context: only that object, never instanced geometry', () => {
    const scene = makeWasmScene()
    expect(defaultFaceEligible(scene, 7n, 7n, undefined)).toBe(true)
    expect(defaultFaceEligible(scene, 7n, 8n, undefined)).toBe(false)
    expect(defaultFaceEligible(scene, 7n, 7n, 12n)).toBe(false)
  })
})

describe('FacePickCache', () => {
  it('memoizes the pick per Ray reference — one raycast for repeated queries on the same event', () => {
    const scene = makeWasmScene({ pick: makePick(7n, 3n) })
    const cache = new FacePickCache()
    const eligible = () => true

    const first = cache.pickFor(scene, RAY, eligible)
    const second = cache.pickFor(scene, RAY, eligible)
    expect(first).toEqual({ object: 7n, face: 3n })
    expect(second).toEqual(first)
    expect(scene.pick_face).toHaveBeenCalledTimes(1)

    // A NEW ray object re-picks (the Viewport builds one Ray per event).
    cache.pickFor(scene, { ...RAY }, eligible)
    expect(scene.pick_face).toHaveBeenCalledTimes(2)
  })

  it('hands the pick instance to the eligibility predicate and caches a rejection as null', () => {
    const scene = makeWasmScene({ pick: makePick(7n, 3n, 12n) })
    const cache = new FacePickCache()
    const isEligible = vi.fn(() => false)

    expect(cache.pickFor(scene, RAY, isEligible)).toBeNull()
    expect(isEligible).toHaveBeenCalledWith(7n, 12n)
    expect(cache.pickFor(scene, RAY, isEligible)).toBeNull()
    expect(scene.pick_face).toHaveBeenCalledTimes(1)
  })

  it('frees the wasm pick handle', () => {
    const pick = makePick(7n, 3n)
    const scene = makeWasmScene({ pick })
    new FacePickCache().pickFor(scene, RAY, () => true)
    expect(pick.free).toHaveBeenCalledTimes(1)
  })
})

describe('worldFaceNormal (component-edit-parity.md phase A2)', () => {
  function normalScene(local: [number, number, number], pose?: Float64Array): WasmScene {
    return {
      face_normal: vi.fn(() => new Float64Array(local)),
      instance_pose: vi.fn(() => pose),
    } as unknown as WasmScene
  }

  it('with activeInstance null, returns face_normal raw — a world object or Group member (local == world already)', () => {
    const scene = normalScene([0, 0, 1])
    expect(worldFaceNormal(scene, 3n, 4n, null)).toEqual([0, 0, 1])
  })

  it('with an activeInstance, poses the LOCAL normal forward through its pose (rotation)', () => {
    // 90° about Z: +X normal becomes +Y.
    const pose = new Float64Array([0, -1, 0, 5, 1, 0, 0, -2, 0, 0, 1, 9])
    const scene = normalScene([1, 0, 0], pose)
    const n = worldFaceNormal(scene, 3n, 4n, 42n)
    expect(n).not.toBeNull()
    expect(n![0]).toBeCloseTo(0)
    expect(n![1]).toBeCloseTo(1)
    expect(n![2]).toBeCloseTo(0)
  })

  it('maps by the inverse-transpose under a non-uniform scale, not the plain linear part', () => {
    const pose = new Float64Array([2, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]) // ×2 in X only
    const local: [number, number, number] = [Math.SQRT1_2, Math.SQRT1_2, 0]
    const scene = normalScene(local, pose)
    const n = worldFaceNormal(scene, 3n, 4n, 42n)
    expect(n).not.toBeNull()
    // The correct inverse-transpose of diag(2,1,1) is diag(0.5,1,1); the
    // WRONG plain-linear-part answer (diag(2,1,1) applied directly) would
    // tilt the other way (toward X, not away from it).
    const correctLen = Math.hypot(0.5, 1, 0)
    expect(n![0]).toBeCloseTo(0.5 / correctLen, 6)
    expect(n![1]).toBeCloseTo(1 / correctLen, 6)
    const wrongLen = Math.hypot(2, 1, 0)
    expect(n![0]).not.toBeCloseTo(2 / wrongLen, 2)
  })

  it('returns null for a stale/unknown instance — never falls back to the raw local normal', () => {
    const scene = normalScene([0, 0, 1], undefined)
    expect(worldFaceNormal(scene, 3n, 4n, 42n)).toBeNull()
  })
})
