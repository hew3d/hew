/**
 * SelectTool unit tests — the pick_face → pick_sketch_edge →
 * pick_sketch_region fallback chain ("sketches are first-class
 * interactable"; an edge hit selects THAT line, an interior hit the whole
 * sketch). Mirrors the fake-WasmScene pattern of CircleTool.test.ts.
 */
import { describe, it, expect, vi } from 'vitest'
import { SelectTool } from './SelectTool'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'

// A ray straight down the -Z axis from above the origin (tuple-shaped, as the
// real Viewport supplies — the tool indexes ray.origin[0..2]).
const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

/** A fake `FacePickJs` returning the seeded handles. */
function makeFacePick(object: bigint, instance?: bigint) {
  return {
    object: () => object,
    instance: () => instance,
    face: () => 3n,
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

/** A fake `SketchEdgePickJs` returning the seeded handles. */
function makeEdgePick(sketch: bigint, edge: bigint) {
  return {
    sketch: () => sketch,
    edge: () => edge,
    free: vi.fn(),
  }
}

function makeWasmScene(opts: {
  facePick?: ReturnType<typeof makeFacePick>
  edgePick?: ReturnType<typeof makeEdgePick>
  regionPick?: ReturnType<typeof makeRegionPick>
} = {}): WasmScene {
  return {
    pick_face: vi.fn(() => opts.facePick),
    pick_sketch_edge: vi.fn(() => opts.edgePick),
    pick_sketch_region: vi.fn(() => opts.regionPick),
  } as unknown as WasmScene
}

describe('SelectTool — pick fallback chain', () => {
  it('a face hit selects the object (and instance, if any)', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(7n, 9n) })
    const onSelect = vi.fn()
    const tool = new SelectTool(scene, onSelect)

    tool.onPointerDown(null, RAY)

    expect(onSelect).toHaveBeenCalledWith(7n, 9n)
    expect(scene.pick_sketch_edge).not.toHaveBeenCalled()
    expect(scene.pick_sketch_region).not.toHaveBeenCalled()
  })

  it('a face miss falls back to a sketch EDGE hit — selecting THAT edge', () => {
    const edgePick = makeEdgePick(11n, 4n)
    const scene = makeWasmScene({ facePick: undefined, edgePick })
    const onSelect = vi.fn()
    const tool = new SelectTool(scene, onSelect)

    tool.onPointerDown(null, RAY)

    expect(onSelect).toHaveBeenCalledWith(null, undefined, 11n, 4n)
    expect(scene.pick_sketch_region).not.toHaveBeenCalled()
    expect(edgePick.free).toHaveBeenCalledTimes(1)
  })

  it('a face AND edge miss falls back to a sketch region INTERIOR hit (clicking inside a rectangle selects its sketch)', () => {
    const regionPick = makeRegionPick(21n, 5n)
    const scene = makeWasmScene({ facePick: undefined, edgePick: undefined, regionPick })
    const onSelect = vi.fn()
    const tool = new SelectTool(scene, onSelect)

    tool.onPointerDown(null, RAY)

    expect(onSelect).toHaveBeenCalledWith(null, undefined, 21n, undefined, 5n)
    expect(regionPick.free).toHaveBeenCalledTimes(1)
  })

  it('a total miss (no face, no edge, no region) clears the selection', () => {
    const scene = makeWasmScene({ facePick: undefined, edgePick: undefined, regionPick: undefined })
    const onSelect = vi.fn()
    const tool = new SelectTool(scene, onSelect)

    tool.onPointerDown(null, RAY)

    expect(onSelect).toHaveBeenCalledWith(null)
  })
})

describe('SelectTool — cancel', () => {
  it('Escape clears the last-seen hover snap', () => {
    const scene = makeWasmScene()
    const onSelect = vi.fn()
    const tool = new SelectTool(scene, onSelect)

    tool.onPointerMove({ x: 0, y: 0, z: 0, kind: 'ground' }, RAY)
    expect(tool.lastSnap).not.toBeNull()

    tool.onKey({ key: 'Escape', preventDefault: () => {} } as unknown as KeyboardEvent)
    expect(tool.lastSnap).toBeNull()
  })
})
