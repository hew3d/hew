/**
 * PushPullTool unit tests — Path A (object face) and Path B (sketch region,
 * now resolved via `pick_sketch_region` across ALL live sketches rather than
 * the old single "active sketch handle" bookkeeping — "sketches are
 * first-class interactable"). Mirrors the fake-WasmScene pattern used by
 * CircleTool.test.ts/ArcTool.test.ts.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { PushPullTool } from './PushPullTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

/** A fake `FacePickJs` returning the seeded handles. */
function makeFacePick(object: bigint, face: bigint) {
  return {
    object: () => object,
    face: () => face,
    instance: () => undefined,
    free: vi.fn(),
  }
}

/** A fake `SketchRegionPickJs` returning the seeded handles. */
function makeRegionPick(sketch: bigint, region: bigint) {
  return {
    sketch: () => sketch,
    region: () => region,
    free: vi.fn(),
  }
}

function makeWasmScene(opts: {
  facePick?: ReturnType<typeof makeFacePick>
  regionPick?: ReturnType<typeof makeRegionPick>
} = {}): WasmScene {
  return {
    pick_face: vi.fn(() => opts.facePick),
    pick_sketch_region: vi.fn(() => opts.regionPick),
    face_normal: vi.fn(() => new Float64Array([0, 0, 1])),
    region_boundary: vi.fn(() => new Float32Array([])),
    face_boundary: vi.fn(() => new Float32Array([])),
    extrude_region: vi.fn(() => 55n),
    push_pull: vi.fn(() => ({
      is_through: () => false,
      result_objects: () => new BigUint64Array([]),
      free: vi.fn(),
    })),
  } as unknown as WasmScene
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new PushPullTool(scene, preview, onCommit, onToast, onMeasurement)
  return { tool, preview, onCommit, onToast, onMeasurement }
}

describe('PushPullTool — Path A (object face)', () => {
  it('two clicks on a face commit push_pull with the picked object/face', () => {
    const facePick = makeFacePick(3n, 4n)
    const scene = makeWasmScene({ facePick })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 2, kind: 'endpoint' }), RAY)

    expect(scene.push_pull).toHaveBeenCalledTimes(1)
    const call = (scene.push_pull as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(3n)
    expect(call[1]).toBe(4n)
    expect(call[2]).toBeCloseTo(2)
    expect(onCommit).toHaveBeenCalledWith(3n)
    expect(onToast).not.toHaveBeenCalled()
  })
})

describe('PushPullTool — Path B (sketch region, any live sketch)', () => {
  it('extrudes a region resolved by pick_sketch_region, even from a sketch handle the tool never saw before', () => {
    // 99n stands in for "not the most recently drawn sketch" — the tool has no
    // per-tool bookkeeping of it at all anymore; pick_sketch_region is the only
    // source of truth.
    const regionPick = makeRegionPick(99n, 7n)
    const scene = makeWasmScene({ facePick: undefined, regionPick })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 3, kind: 'endpoint' }), RAY)

    expect(scene.pick_sketch_region).toHaveBeenCalled()
    expect(scene.extrude_region).toHaveBeenCalledTimes(1)
    const call = (scene.extrude_region as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(99n)
    expect(call[1]).toBe(7n)
    expect(call[2]).toBeCloseTo(3)
    expect(onCommit).toHaveBeenCalledWith(55n)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('is suppressed inside an editing context (region extrusion is a top-level act)', () => {
    const regionPick = makeRegionPick(99n, 7n)
    const scene = makeWasmScene({ facePick: undefined, regionPick })
    const { tool } = makeTool(scene)
    tool.setActiveContext(1n)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(scene.pick_sketch_region).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(false)
  })

  it('a total miss (no face, no region) leaves the tool idle', () => {
    const scene = makeWasmScene({ facePick: undefined, regionPick: undefined })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), RAY)

    expect(tool.capturingInput()).toBe(false)
  })
})
