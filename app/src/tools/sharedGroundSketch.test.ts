/**
 * Cross-tool shared ground sketch — the Viewport hands every draw tool ONE
 * `SketchHandleCache`, so geometry drawn with different tools lands in the
 * same sketch and can close regions together (an arc closed by a Line chord,
 * a rectangle meeting an arc). These tests drive two real tools against one
 * fake WasmScene and assert the sketch handle is minted once and shared.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { ArcTool } from './ArcTool'
import { CircleTool } from './CircleTool'
import { makeSketchHandleCache } from './sketchGesture'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

/** Fake WasmScene covering what ArcTool and CircleTool call in ground mode.
 *  Records the sketch handle passed to every `sketch_add_segment`. */
function makeWasmScene() {
  const segmentSketches: bigint[] = []
  let sketchCounter = 41n
  const scene = {
    begin_ground_sketch: vi.fn(() => {
      sketchCounter += 1n
      return sketchCounter
    }),
    sketch_begin_gesture: vi.fn(),
    sketch_end_gesture: vi.fn(),
    sketch_begin_curve: vi.fn(() => 91n),
    sketch_end_curve: vi.fn(),
    sketch_add_segment: vi.fn((sketch: bigint) => {
      segmentSketches.push(sketch)
      return {
        new_edges: () => new BigUint64Array([]),
        regions_created: () => new BigUint64Array([]),
        regions_removed: () => new BigUint64Array([]),
        free: vi.fn(),
      }
    }),
    pick_face: vi.fn(() => undefined),
  }
  return { scene: scene as unknown as WasmScene, segmentSketches }
}

function makeArcTool(scene: WasmScene, cache: ReturnType<typeof makeSketchHandleCache>) {
  return new ArcTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn(), cache)
}

function makeCircleTool(scene: WasmScene, cache: ReturnType<typeof makeSketchHandleCache>) {
  return new CircleTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn(), cache)
}

describe('shared ground-sketch cache across draw tools', () => {
  it('an arc and a circle drawn by different tools land in the same sketch', () => {
    const { scene, segmentSketches } = makeWasmScene()
    const cache = makeSketchHandleCache()

    // Arc tool commits first (as if the user drew an arc, then switched tools).
    const arc = makeArcTool(scene, cache)
    arc.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)
    arc.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY)
    arc.onPointerDown(makeSnap({ x: 1, y: 0.5 }), RAY)

    // A fresh CircleTool instance — tools are recreated on every switch —
    // sharing the same cache commits into the SAME sketch.
    const circle = makeCircleTool(scene, cache)
    circle.onPointerDown(makeSnap({ x: 5, y: 5 }), RAY)
    circle.onPointerDown(makeSnap({ x: 6, y: 5 }), RAY)

    const beginSketch = (scene as unknown as { begin_ground_sketch: ReturnType<typeof vi.fn> }).begin_ground_sketch
    expect(beginSketch).toHaveBeenCalledTimes(1)
    expect(segmentSketches.length).toBeGreaterThan(0)
    expect(new Set(segmentSketches).size).toBe(1)
  })

  it('onDocumentReset on ONE tool clears the shared handle for all of them', () => {
    const { scene, segmentSketches } = makeWasmScene()
    const cache = makeSketchHandleCache()

    const arc = makeArcTool(scene, cache)
    arc.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)
    arc.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY)
    arc.onPointerDown(makeSnap({ x: 1, y: 0.5 }), RAY)
    const firstSketch = segmentSketches[0]

    const circle = makeCircleTool(scene, cache)
    circle.onDocumentReset() // document replaced — stale handle dropped

    arc.onPointerDown(makeSnap({ x: 0, y: 0 }), RAY)
    arc.onPointerDown(makeSnap({ x: 2, y: 0 }), RAY)
    arc.onPointerDown(makeSnap({ x: 1, y: 0.5 }), RAY)

    const beginSketch = (scene as unknown as { begin_ground_sketch: ReturnType<typeof vi.fn> }).begin_ground_sketch
    expect(beginSketch).toHaveBeenCalledTimes(2)
    expect(segmentSketches.at(-1)).not.toBe(firstSketch)
  })
})
