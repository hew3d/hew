/**
 * An idle draw tool's hover inside a sketch's filled region adopts that
 * sketch's plane (view-plane design §2 step 4b), against the REAL compiled
 * wasm kernel's `pick_sketch_region`. Before, adoption needed the cursor
 * within the edge pick's aperture of a segment, so over a fill's interior
 * the hover fell through to the plane the view faces (step 7), and a Shift
 * there pinned that plane instead of the sketch's. A ground sketch keeps
 * the ground from any camera. Mirrors `viewPlaneSnap.wasm.test.ts`'s
 * real-wasm-in-vitest pattern.
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import init, { Scene } from '../wasm/pkg/wasm_api.js'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { groundDrawPlane, resolveIdleDrawTarget, SketchPickCache, type CameraView } from './drawPlane'
import { setPickRenderMode } from './pickPolicy'
import type { EditContext } from './types'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmBytes = await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm'))
  await init({ module_or_path: wasmBytes })
})

afterEach(() => setPickRenderMode('shaded'))

type V3 = [number, number, number]
const TOP: EditContext = { kind: 'top' }

/** A closed polygon drawn as one sketch on the plane through `p` with
 *  normal `n`. Returns the sketch handle. */
function sketchLoop(scene: Scene, p: V3, n: V3, corners: V3[]): bigint {
  const sketch = scene.begin_sketch_on_plane(...p, ...n)
  for (let i = 0; i < corners.length; i++) {
    const a = corners[i]
    const b = corners[(i + 1) % corners.length]
    scene.sketch_add_segment(sketch, ...a, ...b)
  }
  expect(Array.from(scene.sketch_regions(sketch)).length).toBe(1)
  return sketch
}

/** A rectangle standing on the green plane y = 0: x ∈ [−1, 1], z ∈ [0, 1]. */
function greenRectangle(scene: Scene): bigint {
  return sketchLoop(scene, [0, 0, 0], [0, 1, 0], [[-1, 0, 0], [1, 0, 0], [1, 0, 1], [-1, 0, 1]])
}

/** The camera at `eye` looking at `at`. */
function view(eye: V3, at: V3): CameraView {
  const d: V3 = [at[0] - eye[0], at[1] - eye[1], at[2] - eye[2]]
  const l = Math.hypot(...d)
  return { dir: [d[0] / l, d[1] / l, d[2] / l], eye }
}

/** From `eye` through `at`. */
function ray(eye: V3, at: V3): Ray {
  return { origin: eye, direction: [at[0] - eye[0], at[1] - eye[1], at[2] - eye[2]] }
}

describe('a hover inside a sketch fill adopts the sketch (real kernel)', () => {
  it('the middle of a standing rectangle, far from every edge, gives that sketch', () => {
    const scene = new Scene()
    const sketch = greenRectangle(scene)
    const wasm = scene as unknown as WasmScene
    const r: Ray = { origin: [0, -5, 0.5], direction: [0, 1, 0] }
    // No edge within the pick aperture: only the fill can answer.
    expect(scene.pick_sketch(...r.origin, ...r.direction)).toBeUndefined()
    // From a three-quarter view (step 7 would give the ground).
    const resolved = resolveIdleDrawTarget(wasm, new SketchPickCache(), r, TOP, undefined, '', view([3.5, -3, 2.5], [0, 0, 0]))
    expect(resolved.target).toEqual({ kind: 'existing', handle: sketch, instance: null })
    expect(Math.abs(resolved.plane.normal[1])).toBeCloseTo(1, 9)
    expect(resolved.inferredAxis).toBeUndefined()
    scene.free()
  })

  it('a ground square hovered from a 15° camera gives the ground, not the vertical plane the view faces', () => {
    const scene = new Scene()
    // A 2 m ground square centred on (0.5, 0.5, 0).
    sketchLoop(scene, [0, 0, 0], [0, 0, 1], [[-0.5, -0.5, 0], [1.5, -0.5, 0], [1.5, 1.5, 0], [-0.5, 1.5, 0]])
    const wasm = scene as unknown as WasmScene
    const eye: V3 = [0.5, -6, 6 * Math.tan(15 * Math.PI / 180)]
    const cam = view(eye, [0.5, 0, 0])
    // Over empty ground beside it, the view's own plane: green.
    const beside = resolveIdleDrawTarget(wasm, new SketchPickCache(), ray(eye, [4, 0.5, 0]), TOP, undefined, '', cam)
    expect(beside.inferredAxis).toBe(1)
    // Inside the square: the ground, shaped exactly as the ground fallback.
    const r = ray(eye, [0.5, 0.5, 0])
    expect(scene.pick_sketch(...r.origin, ...r.direction)).toBeUndefined()
    const inside = resolveIdleDrawTarget(wasm, new SketchPickCache(), r, TOP, undefined, '', cam)
    expect(inside).toStrictEqual({ plane: groundDrawPlane(), target: { kind: 'plane', plane: groundDrawPlane(), instance: null } })
    scene.free()
  })

  it('in Wireframe the fill is no target (step 7), while an edge still adopts', () => {
    const scene = new Scene()
    const sketch = greenRectangle(scene)
    const wasm = scene as unknown as WasmScene
    scene.set_render_mode('wireframe')
    setPickRenderMode('wireframe')
    const front = view([0, -5, 0.5], [0, 0, 0.5])
    const middle: Ray = { origin: [0, -5, 0.5], direction: [0, 1, 0] }
    const fill = resolveIdleDrawTarget(wasm, new SketchPickCache(), middle, TOP, undefined, '', front)
    expect(fill.target.kind).toBe('plane')
    expect(fill.inferredAxis).toBe(1)
    // A ray onto the rectangle's right edge (x = 1) from the same eye.
    const onEdge = ray([0, -5, 0.5], [1, 0, 0.5])
    expect(scene.pick_sketch(...onEdge.origin, ...onEdge.direction)).toBe(sketch)
    const edge = resolveIdleDrawTarget(wasm, new SketchPickCache(), onEdge, TOP, undefined, '', front)
    expect(edge.target).toEqual({ kind: 'existing', handle: sketch, instance: null })
    scene.free()
  })
})
