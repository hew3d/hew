/**
 * Push/Pull driven through the REAL compiled wasm module, for what a lock and
 * a linked copy do to it (OW-1): a detached shape drawn on a locked face pulls
 * into a new Object, Ctrl/Cmd Push/Pull (new object) works on a locked face
 * and on a linked copy's face, and neither touches the thing it was pulled
 * from.
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import * as THREE from 'three'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import init, { Scene } from '../wasm/pkg/wasm_api.js'
import { PushPullTool } from './PushPullTool'
import type { Ray } from '../viewport/math'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import { PREVIEW_EXTRUDE_NEW_TOKEN } from './previewTint'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmBytes = await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm'))
  await init({ module_or_path: wasmBytes })
})

/** A [x0,x0+1]x[0,1] box on the ground, 1 tall. */
function makeBox(scene: Scene, x0 = 0): bigint {
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
  const c: Array<[number, number]> = [[x0, 0], [x0 + 1, 0], [x0 + 1, 1], [x0, 1]]
  for (let i = 0; i < 4; i++) {
    const a = c[i]
    const b = c[(i + 1) % 4]
    scene.sketch_add_segment(sketch, a[0], a[1], 0, b[0], b[1], 0)
  }
  const regions = Array.from(scene.sketch_regions(sketch))
  return scene.extrude_region(sketch, regions[0], 1)
}

/** The world-object ids, sorted. */
function objects(scene: Scene): bigint[] {
  return Array.from(scene.object_ids()).sort((a, b) => (a < b ? -1 : a > b ? 1 : 0))
}

/** A ray from straight above (x, y) looking down. */
const down = (x: number, y: number): Ray => ({ origin: [x, y, 10], direction: [0, 0, -1] })
const on = (x: number, y: number, z: number): Snap => ({ x, y, z, kind: 'face' })

function harness(scene: Scene) {
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const tool = new PushPullTool(scene as unknown as WasmScene, new THREE.Group(), onCommit, onToast)
  // The Viewport's top-level Push/Pull eligibility: a plain Object's face,
  // or a face reached through a placement of a LINKED set (pushing the
  // shared shape needs no context); any other component face does not.
  tool.setFaceEligibility((_object, instance) => instance === undefined || scene.is_linked(instance))
  return { tool, onCommit, onToast }
}

/** Hover, click, drag up to `z`, click: a real Push/Pull gesture. */
function pull(tool: PushPullTool, x: number, y: number, from: number, to: number): void {
  const ray = down(x, y)
  tool.onPointerMove(on(x, y, from), ray)
  tool.onPointerDown(on(x, y, from), ray)
  const up: Ray = { origin: [x + 5, y, to], direction: [-1, 0, 0] }
  tool.onPointerMove(on(x, y, to), up)
  tool.onPointerDown(on(x, y, to), up)
}

type Loop = Array<[number, number]>
const INSIDE: Loop = [[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75]]
const WHOLE: Loop = [[0, 0], [1, 0], [1, 1], [0, 1]]
const ON_EDGE: Loop = [[0, 0], [0.5, 0], [0.5, 0.5], [0, 0.5]]

/** Draws `loop` on the box's top face the way a draw tool opens it. */
function drawOnTop(scene: Scene, box: bigint, face: bigint, loop: Loop, route: 'face' | 'detached' | 'plane'): void {
  const d = route === 'face'
    ? scene.draft_begin_face(box, face, 'Rectangle')
    : route === 'plane'
      // The face's plane, as a plane draft (a draw that adopted the plane,
      // not the face): the locked box takes no part, so this is free drawing
      // lying on the face, not a detached shape.
      ? scene.draft_begin_plane(0, 0, 1, 0, 0, 1, 'Rectangle')
      : scene.draft_begin_detached(JSON.stringify({ face: { object: Number(box), face: Number(face) } }), 'Rectangle')
  for (let i = 0; i < loop.length; i++) {
    const a = loop[i]
    const b = loop[(i + 1) % loop.length]
    scene.draft_add_segment(d, a[0], a[1], 1, b[0], b[1], 1).free()
  }
  scene.draft_end(d)
}

describe('Push/Pull on a locked box', () => {
  it.each([
    ['inside the face, draft_begin_face', INSIDE, 'face', 0.5, 0.5],
    ['inside the face, draft_begin_detached', INSIDE, 'detached', 0.5, 0.5],
    ['on the whole face', WHOLE, 'face', 0.5, 0.5],
    ['along two of its edges', ON_EDGE, 'face', 0.25, 0.25],
    ['as free drawing on its plane', INSIDE, 'plane', 0.5, 0.5],
    ['as free drawing over the whole face', WHOLE, 'plane', 0.5, 0.5],
  ] as const)('pulls a shape drawn on the locked face (%s) into a new Object, the box untouched', (_, loop, route, x, y) => {
    const scene = new Scene()
    const box = makeBox(scene)
    const top = scene.pick_face(0.5, 0.5, 10, 0, 0, -1)!
    const face = top.face()
    top.free()
    scene.set_nodes_locked(Uint8Array.of(0), BigUint64Array.of(box), true)
    drawOnTop(scene, box, face, [...loop], route)
    const before = objects(scene)

    const { tool, onToast, onCommit } = harness(scene)
    tool.onPointerMove(on(x, y, 1), down(x, y))
    expect(tool.hoverHint()).toBeNull()
    expect(tool.hoverFace()?.colorToken).toBe(PREVIEW_EXTRUDE_NEW_TOKEN)
    pull(tool, x, y, 1, 1.5)

    expect(onToast).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledTimes(1)
    const after = objects(scene)
    expect(after.length).toBe(before.length + 1)
    expect(scene.node_locked(0, box)).toBe(true)
    const fresh = after.find((o) => !before.includes(o))!
    expect(scene.node_frozen(0, fresh)).toBeUndefined()
  })

  it('Ctrl/Cmd Push/Pull extrudes a new Object from the locked face, cued blue, the box untouched', () => {
    const scene = new Scene()
    const box = makeBox(scene)
    scene.set_nodes_locked(Uint8Array.of(0), BigUint64Array.of(box), true)
    const before = objects(scene)
    const { tool, onToast, onCommit } = harness(scene)
    tool.toggleExtrudeAsNew()

    tool.onPointerMove(on(0.5, 0.5, 1), down(0.5, 0.5))
    expect(tool.hoverHint()).toBeNull()
    expect(tool.hoverFace()?.colorToken).toBe(PREVIEW_EXTRUDE_NEW_TOKEN)
    pull(tool, 0.5, 0.5, 1, 1.5)

    expect(onToast).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(objects(scene).length).toBe(before.length + 1)
    expect(scene.node_locked(0, box)).toBe(true)
  })

  it('tapping Ctrl over the locked face turns the locked line into the blue new-object cue, and back', () => {
    const scene = new Scene()
    const box = makeBox(scene)
    scene.set_nodes_locked(Uint8Array.of(0), BigUint64Array.of(box), true)
    const { tool } = harness(scene)
    tool.onPointerMove(on(0.5, 0.5, 1), down(0.5, 0.5))
    expect(tool.hoverHint()).not.toBeNull()
    expect(tool.hoverFace()).toBeNull()
    tool.toggleExtrudeAsNew()
    expect(tool.hoverHint()).toBeNull()
    expect(tool.hoverFace()?.colorToken).toBe(PREVIEW_EXTRUDE_NEW_TOKEN)
    tool.toggleExtrudeAsNew()
    expect(tool.hoverHint()).not.toBeNull()
    expect(tool.hoverFace()).toBeNull()
  })

  it('a pull that began on the locked face stays a new Object when Ctrl is released before the committing click', () => {
    const scene = new Scene()
    const box = makeBox(scene)
    scene.set_nodes_locked(Uint8Array.of(0), BigUint64Array.of(box), true)
    const before = objects(scene)
    const { tool, onToast, onCommit } = harness(scene)
    tool.toggleExtrudeAsNew()
    const ray = down(0.5, 0.5)
    tool.onPointerMove(on(0.5, 0.5, 1), ray)
    tool.onPointerDown(on(0.5, 0.5, 1), ray)
    // Ctrl comes up mid-drag: the mode must not flip under a locked pull.
    tool.toggleExtrudeAsNew()
    const up: Ray = { origin: [5.5, 0.5, 1.5], direction: [-1, 0, 0] }
    tool.onPointerMove(on(0.5, 0.5, 1.5), up)
    expect(tool.statusHint()).toMatch(/NEW object/)
    tool.onPointerDown(on(0.5, 0.5, 1.5), up)

    expect(onToast).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(objects(scene).length).toBe(before.length + 1)
    expect(scene.node_locked(0, box)).toBe(true)
    // The latch ended with the gesture: the next hover on the locked face refuses again.
    tool.onPointerMove(on(0.5, 0.5, 1), ray)
    expect(tool.hoverHint()).not.toBeNull()
  })

  it('without Ctrl the locked face still refuses, with the locked line', () => {
    const scene = new Scene()
    const box = makeBox(scene)
    scene.set_nodes_locked(Uint8Array.of(0), BigUint64Array.of(box), true)
    const { tool, onToast, onCommit } = harness(scene)
    tool.onPointerMove(on(0.5, 0.5, 1), down(0.5, 0.5))
    expect(tool.hoverHint()).not.toBeNull()
    tool.onPointerDown(on(0.5, 0.5, 1), down(0.5, 0.5))
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onCommit).not.toHaveBeenCalled()
  })
})

/** A unit box and a linked copy of it 3 m along +x: two placements of one
 *  shared shape. Returns the two placements. */
function linkedPair(scene: Scene): bigint[] {
  const box = makeBox(scene)
  const shift = Float64Array.of(1, 0, 0, 3, 0, 1, 0, 0, 0, 0, 1, 0)
  scene.duplicate_selection_array(Uint8Array.of(0), BigUint64Array.of(box), shift, 1, true).forEach((n) => n.free())
  const instances = Array.from(scene.instance_ids())
  expect(instances.length).toBe(2)
  return instances
}

describe('Ctrl/Cmd Push/Pull on a linked copy', () => {
  it.each([
    ['unlocked', false],
    ['locked', true],
  ] as const)('makes a new plain Object at the clicked copy (%s), leaving the linked set as it was', (_, locked) => {
    const scene = new Scene()
    const instances = linkedPair(scene)
    if (locked) scene.set_nodes_locked(Uint8Array.of(2), BigUint64Array.of(instances[0]), true)
    const before = objects(scene)
    const hash = scene.state_hash()

    const { tool, onToast, onCommit } = harness(scene)
    tool.toggleExtrudeAsNew()
    tool.onPointerMove(on(3.5, 0.5, 1), down(3.5, 0.5))
    expect(tool.hoverHint()).toBeNull()
    expect(tool.hoverFace()?.colorToken).toBe(PREVIEW_EXTRUDE_NEW_TOKEN)
    pull(tool, 3.5, 0.5, 1, 1.5)

    expect(onToast).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledTimes(1)
    const after = objects(scene)
    expect(after.length).toBe(before.length + 1)
    const fresh = after.find((o) => !before.includes(o))!
    // Where the clicked copy is: x in [3, 4], z from 1 up to 1.5.
    const bb = scene.object_bounds(fresh)!
    expect(bb[0]).toBeCloseTo(3)
    expect(bb[3]).toBeCloseTo(4)
    expect(bb[2]).toBeCloseTo(1)
    expect(bb[5]).toBeCloseTo(1.5)
    // The set is unchanged: still two placements, and undo is exact.
    expect(Array.from(scene.instance_ids()).length).toBe(2)
    expect(scene.node_frozen(0, fresh)).toBeUndefined()
    scene.scene_undo().free()
    expect(scene.state_hash()).toBe(hash)
  })
})

describe('a component copy (not linked) at the top level', () => {
  it('keeps its double-click-to-edit refusal, Ctrl or not: the face is not this level’s to act on', () => {
    const scene = new Scene()
    const box = makeBox(scene)
    scene.make_component(Uint8Array.of(0), BigUint64Array.of(box))
    const { tool, onToast, onCommit } = harness(scene)
    tool.toggleExtrudeAsNew()
    tool.onPointerMove(on(0.5, 0.5, 1), down(0.5, 0.5))
    expect(tool.hoverHint()).not.toBeNull()
    tool.onPointerDown(on(0.5, 0.5, 1), down(0.5, 0.5))
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onCommit).not.toHaveBeenCalled()
  })
})
