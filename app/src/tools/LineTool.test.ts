import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { LineTool } from './LineTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

/** A ray straight down (−Z) through world (x, y) — hits a z=1 top face at (x, y, 1). */
function rayThrough(x: number, y: number): Ray {
  return { origin: [x, y, 5], direction: [0, 0, -1] }
}

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

function makePick(object: bigint, face: bigint, instance?: bigint) {
  return {
    object: () => object,
    face: () => face,
    instance: () => instance,
    free: vi.fn(),
  }
}

/** Minimal WasmScene stub — only the members LineTool calls in these paths. */
function makeWasmScene(opts: {
  pick?: () => ReturnType<typeof makePick> | undefined
  /** node_parent(0, id) result per object (a grouped object's group id). */
  parents?: Map<bigint, bigint>
} = {}): WasmScene {
  let sketchCounter = 41n
  return {
    begin_ground_sketch: vi.fn(() => {
      sketchCounter += 1n
      return sketchCounter
    }),
    sketch_begin_gesture: vi.fn(),
    sketch_end_gesture: vi.fn(),
    sketch_add_segment: vi.fn(() => ({
      new_edges: () => new BigUint64Array([]),
      regions_created: () => new BigUint64Array([]),
      regions_removed: () => new BigUint64Array([]),
      free: vi.fn(),
    })),
    pick_face: vi.fn(() => opts.pick?.()),
    pick_sketch: vi.fn(() => undefined), // no committed sketches in these fixtures
    sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, 1])), // every minted sketch is on the ground plane
    node_parent: vi.fn((_kind: number, id: bigint) => opts.parents?.get(id)),
    // A top face at z=1, normal +Z.
    face_normal: vi.fn(() => new Float64Array([0, 0, 1])),
    face_plane: vi.fn(() => new Float64Array([0, 0, 1, 0, 0, 1])),
    split_face: vi.fn(() => ({
      kind: () => 'split',
      free: vi.fn(),
    })),
    clear_transient_segments: vi.fn(),
    add_transient_segment: vi.fn(),
  } as unknown as WasmScene
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onFaceImprint = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new LineTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement)
  return { tool, preview, onCommit, onToast, onFaceImprint, onMeasurement }
}

// The context contract shared by every draw tool (see RectangleTool.test.ts):
// inside an entered object's editing context, drawing is scoped to that
// object — a click on another object's face OR on empty ground is ignored
// outright, never re-routed to a top-level ground sketch.
describe('LineTool — editing-context scoping', () => {
  it('in-context clicks on empty ground do NOT start a top-level ground sketch', () => {
    const scene = makeWasmScene() // pick_face misses — bare ground under the ray
    const { tool } = makeTool(scene)
    tool.setActiveContext(7n)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), rayThrough(1, 1))

    expect(tool.capturingInput()).toBe(false) // no chain ever anchored
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(scene.sketch_add_segment).not.toHaveBeenCalled()
  })

  it('in-context clicks on a DIFFERENT object\'s face are ignored', () => {
    const scene = makeWasmScene({ pick: () => makePick(999n, 3n) })
    const { tool } = makeTool(scene)
    tool.setActiveContext(7n)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1 }), rayThrough(0, 0))

    expect(tool.capturingInput()).toBe(false)
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
  })

  it('in-context clicks on the ENTERED object\'s face anchor a face chain', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)
    tool.setActiveContext(7n)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), rayThrough(0, 0))

    expect(tool.capturingInput()).toBe(true) // face chain anchored
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
  })
})

describe('LineTool — top-level plain-object policy (parity with RectangleTool)', () => {
  it('a plain object\'s face anchors a face chain directly (no edit context)', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), rayThrough(0, 0))
    expect(tool.capturingInput()).toBe(true)
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
  })

  it('a GROUPED object\'s face falls back to ground mode (groups keep the edit step)', () => {
    const scene = makeWasmScene({
      pick: () => makePick(7n, 3n),
      parents: new Map([[7n, 5n]]),
    })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), rayThrough(1, 1))

    expect(scene.sketch_add_segment).toHaveBeenCalledTimes(1) // ground segment
    expect(scene.split_face).not.toHaveBeenCalled()
  })

  it('instanced (component) geometry falls back to ground mode', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n, 12n) })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), rayThrough(1, 1))

    expect(scene.sketch_add_segment).toHaveBeenCalledTimes(1)
    expect(scene.split_face).not.toHaveBeenCalled()
  })
})
