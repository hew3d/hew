/**
 * Make Unique keeps a copied board's identity, exercised through the REAL
 * compiled wasm module (the `booleanAutoExplode.wasm.test.ts` pattern): a
 * named, tagged loose board becomes a component, a second placement is
 * made unique, and the board inside the private copy reads the same in
 * the Outliner (the shared `makeLabelFor` resolver) and keeps its tag.
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeAll, describe, expect, it } from 'vitest'
import init, { Scene } from '../wasm/pkg/wasm_api.js'
import { makeLabelFor } from '../panels/treeModel'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmPath = resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm')
  const wasmBytes = await readFile(wasmPath)
  await init({ module_or_path: wasmBytes })
})

function makeBox(scene: Scene, x0: number, x1: number): bigint {
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
  const corners: Array<[number, number]> = [
    [x0, 0],
    [x1, 0],
    [x1, 1],
    [x0, 1],
  ]
  for (let i = 0; i < 4; i++) {
    const [ax, ay] = corners[i]
    const [bx, by] = corners[(i + 1) % 4]
    scene.sketch_add_segment(sketch, ax, ay, 0, bx, by, 0)
  }
  const regions = Array.from(scene.sketch_regions(sketch))
  expect(regions.length).toBe(1)
  return scene.extrude_region(sketch, regions[0], 1)
}

describe('Make Unique', () => {
  it('make_unique_keeps_the_copied_boards_name_in_the_outliner', () => {
    const scene = new Scene()
    const board = makeBox(scene, 0, 4)
    scene.set_node_name(0, board, 'Leg')
    scene.add_node_tag(0, board, ['Structure', 'Leg'])
    const first = scene.make_component(new Uint8Array([0]), new BigUint64Array([board]))
    const def = scene.instance_def(first)
    expect(def).toBeDefined()
    const copy = scene.place_instance(def!, new Float64Array([1, 0, 0, 6, 0, 1, 0, 0, 0, 0, 1, 0]))

    const labelFor = makeLabelFor(scene, new Map())
    const before = labelFor({ kind: 'object', id: board })
    expect(before).toBe('Leg')

    const newDef = scene.make_unique(copy)
    expect(newDef).not.toBe(def)
    const members = Array.from(scene.component_member_objects(newDef))
    expect(members.length).toBe(1)
    expect(members[0]).not.toBe(board)
    expect(labelFor({ kind: 'object', id: members[0] })).toBe(before)
    expect(Array.from(scene.node_tags(0, members[0]))).toEqual(['Structure/Leg'])

    // Undo returns the shared state; redo brings the named copy back.
    scene.scene_undo()
    expect(scene.instance_def(copy)).toBe(def)
    scene.scene_redo()
    expect(scene.instance_def(copy)).toBe(newDef)
    expect(labelFor({ kind: 'object', id: members[0] })).toBe('Leg')
    scene.free()
  })
})
