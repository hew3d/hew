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
    split_face_in_instance: vi.fn(() => ({
      kind: () => 'split',
      free: vi.fn(),
    })),
    begin_sketch_on_plane_in_instance: vi.fn(() => {
      sketchCounter += 1n
      return sketchCounter
    }),
    instance_pose: vi.fn(() => new Float64Array([1, 0, 0, 5, 0, 1, 0, 0, 0, 0, 1, 0])), // translated +5 in x
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
    tool.setEditContext({ kind: 'object', id: 7n })

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), rayThrough(1, 1))

    expect(tool.capturingInput()).toBe(false) // no chain ever anchored
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(scene.sketch_add_segment).not.toHaveBeenCalled()
  })

  it('in-context clicks on a DIFFERENT object\'s face are ignored', () => {
    const scene = makeWasmScene({ pick: () => makePick(999n, 3n) })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'object', id: 7n })

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1 }), rayThrough(0, 0))

    expect(tool.capturingInput()).toBe(false)
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
  })

  it('in-context clicks on the ENTERED object\'s face anchor a face chain', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'object', id: 7n })

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

// Component-edit-parity.md phase A2: inside an INSTANCE editing context,
// draw tools route to the definition-owned wasm surface instead of the
// world one — the fix for the axis-lock symptom (see the idle-lock case
// below, the flagship repro) and for face-mode cuts refusing outright.
describe('LineTool — instance editing context (component-edit-parity.md phase A2)', () => {
  const INSTANCE = 42n
  const COMPONENT = 5n
  const INSTANCE_CTX = { kind: 'instance' as const, id: INSTANCE, component: COMPONENT }

  it('face mode routes to split_face_in_instance, never the world split_face', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n, INSTANCE) })
    const { tool, onFaceImprint } = makeTool(scene)
    tool.setEditContext(INSTANCE_CTX)

    // `faceDrawEligible` isn't injected in this unit test, but the default
    // fallback policy (defaultFaceEligible) refuses any instanced pick —
    // inject the richer predicate directly, mirroring what the Viewport's
    // `faceDrawEligible` would report for a member of the entered instance.
    tool.setFaceEligibility((_object, instance) => instance === INSTANCE)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 1, kind: 'face' }), rayThrough(1, 0))
    tool.onDoubleClick(null, rayThrough(1, 0)) // ends the chain, commits the cut

    expect(scene.split_face_in_instance).toHaveBeenCalledTimes(1)
    expect(scene.split_face_in_instance).toHaveBeenCalledWith(
      INSTANCE, 7n, 3n, expect.any(Float64Array),
    )
    expect(scene.split_face).not.toHaveBeenCalled()
    expect(onFaceImprint).toHaveBeenCalledWith(7n)
  })

  it('plane mode (idle-locked, THE original axis-lock symptom) mints via begin_sketch_on_plane_in_instance, never begin_ground_sketch', () => {
    const scene = makeWasmScene({ pick: () => undefined }) // no face under the cursor
    const { tool } = makeTool(scene)
    tool.setEditContext(INSTANCE_CTX)

    // Arrow-key idle lock (Z/blue axis) — the exact repro named in the design.
    tool.onKey({ key: 'ArrowUp', repeat: false } as unknown as KeyboardEvent)
    tool.onPointerDown(makeSnap({ x: 6, y: 1, z: 0.5 }), rayThrough(6, 1))
    tool.onPointerDown(makeSnap({ x: 6, y: 2, z: 0.5 }), rayThrough(6, 2))

    expect(scene.begin_sketch_on_plane_in_instance).toHaveBeenCalledTimes(1)
    expect(scene.begin_sketch_on_plane_in_instance).toHaveBeenCalledWith(INSTANCE, 6, 1, 0.5, 0, 0, 1)
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    // The segment's points are mapped into DEFINITION-local space (pose⁻¹ of
    // the +5-in-x translation): world (6,1,0.5) → local (1,1,0.5).
    expect(scene.sketch_add_segment).toHaveBeenCalledWith(
      expect.any(BigInt), 1, 1, 0.5, 1, 2, 0.5,
    )
  })

  it('plane mode on empty space (no idle lock) ALSO mints a def-owned sketch, not a world one', () => {
    const scene = makeWasmScene({ pick: () => undefined })
    const { tool } = makeTool(scene)
    tool.setEditContext(INSTANCE_CTX)

    tool.onPointerDown(makeSnap({ x: 6, y: 1, z: 0 }), rayThrough(6, 1))
    tool.onPointerDown(makeSnap({ x: 7, y: 1, z: 0 }), rayThrough(7, 1))

    expect(scene.begin_sketch_on_plane_in_instance).toHaveBeenCalledTimes(1)
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
  })
})
