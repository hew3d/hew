import { describe, it, expect, vi } from 'vitest'
import { FacePickCache, defaultFaceEligible } from './faceDraw'
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
