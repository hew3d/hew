import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { RectangleTool } from './RectangleTool'
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

/** Minimal WasmScene stub — only the members RectangleTool calls. */
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
    node_parent: vi.fn((_kind: number, id: bigint) => opts.parents?.get(id)),
    // A top face at z=1, normal +Z.
    face_normal: vi.fn(() => new Float64Array([0, 0, 1])),
    face_plane: vi.fn(() => new Float64Array([0, 0, 1, 0, 0, 1])),
    split_face_inner: vi.fn(() => 99n),
  } as unknown as WasmScene
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onFaceImprint = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new RectangleTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement)
  return { tool, preview, onCommit, onToast, onFaceImprint, onMeasurement }
}

// Deliberate contract change (selection-UX overhaul): drawing on a PLAIN
// solid's face at the top level no longer requires double-clicking into the
// object first — clicking its face with a draw tool means "draw on that
// face". Groups and Components keep the explicit edit step.
describe('RectangleTool — top-level draw-on-face', () => {
  it('two clicks on a plain object\'s face imprint it via split_face_inner (no edit context)', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool, onFaceImprint, onToast } = makeTool(scene)

    // First corner on the face, then the opposite corner 1×2 away.
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 1, kind: 'face' }), rayThrough(1, 2))

    expect(scene.split_face_inner).toHaveBeenCalledTimes(1)
    const [object, face, loopPts] = (scene.split_face_inner as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(object).toBe(7n)
    expect(face).toBe(3n)
    expect((loopPts as Float64Array).length).toBe(12) // 4 corners × xyz
    expect(onFaceImprint).toHaveBeenCalledWith(7n)
    // No ground sketch was created or touched.
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
    expect(scene.sketch_add_segment).not.toHaveBeenCalled()
    expect(onToast).not.toHaveBeenCalled()
  })

  it('a face of a GROUPED object falls back to ground mode (groups keep the edit step)', () => {
    const scene = makeWasmScene({
      pick: () => makePick(7n, 3n),
      parents: new Map([[7n, 5n]]), // object 7 lives inside group 5
    })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 0 }), rayThrough(1, 2))

    expect(scene.split_face_inner).not.toHaveBeenCalled()
    expect(scene.sketch_add_segment).toHaveBeenCalledTimes(4) // ground rectangle
  })

  it('instanced (component) geometry falls back to ground mode (components keep the edit step)', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n, 12n) })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 0 }), rayThrough(1, 2))

    expect(scene.split_face_inner).not.toHaveBeenCalled()
    expect(scene.sketch_add_segment).toHaveBeenCalledTimes(4)
  })

  it('idle snapConstraint locks to a plain object\'s face plane so the first corner lands on it', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)

    const constraint = tool.snapConstraint(rayThrough(0, 0))
    expect(constraint?.constraintPlane).toEqual({ point: [0, 0, 1], normal: [0, 0, 1] })
  })

  it('a mid-gesture GROUND rectangle is not hijacked by hovering a face for the second corner', () => {
    let hovering = false
    const scene = makeWasmScene({ pick: () => (hovering ? makePick(7n, 3n) : undefined) })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // ground anchor
    hovering = true // cursor drifts over a solid mid-gesture
    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 0 }), rayThrough(1, 2))

    expect(scene.sketch_add_segment).toHaveBeenCalledTimes(4) // stayed a ground rectangle
    expect(scene.split_face_inner).not.toHaveBeenCalled()
  })

  it('inside an entered object context only that object\'s faces are drawable (unchanged)', () => {
    const scene = makeWasmScene({ pick: () => makePick(999n, 3n) })
    const { tool } = makeTool(scene)
    tool.setActiveContext(7n)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1 }), rayThrough(0, 0))

    // Ignored outright — no face anchor, and no ground sketch either.
    expect(tool.capturingInput()).toBe(false)
    expect(scene.begin_ground_sketch).not.toHaveBeenCalled()
  })
})
