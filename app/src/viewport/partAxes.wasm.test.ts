/**
 * Part axes against the REAL wasm Scene: the entered instance (hidden by the
 * explode session), nested (definition-owned) instances, and user-hidden
 * nodes.
 */
import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeAll, describe, expect, it } from 'vitest'
import init, { Scene } from '../wasm/pkg/wasm_api.js'
import { framesForSelection } from './partAxes'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmBytes = await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm'))
  await init({ module_or_path: wasmBytes })
})

function makeBox(scene: Scene, x0: number, x1: number, y0: number, y1: number, height: number): bigint {
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
  const c: Array<[number, number]> = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
  for (let i = 0; i < 4; i++) {
    const [ax, ay] = c[i]
    const [bx, by] = c[(i + 1) % 4]
    scene.sketch_add_segment(sketch, ax, ay, 0, bx, by, 0)
  }
  const regions = Array.from(scene.sketch_regions(sketch))
  return scene.extrude_region(sketch, regions[0], height)
}
const shift = (dx: number): Float64Array => new Float64Array([1, 0, 0, dx, 0, 1, 0, 0, 0, 0, 1, 0])
const inst = (id: bigint) => ({ kind: 'instance' as const, id })

describe('partAxes (real wasm)', () => {
  it('the_entered_instance_keeps_its_triad_during_an_explode_session', () => {
    const scene = new Scene()
    const i = scene.make_component(new Uint8Array([0]), new BigUint64Array([makeBox(scene, 0, 1, 0, 1, 1)]))
    scene.transform_instance(i, shift(5))
    scene.open_explode_session(i)
    const f = framesForSelection(scene, [], i)
    expect(f).toHaveLength(1)
    expect(f[0].origin[0]).toBeCloseTo(5)
    // Selected while entered: still one triad.
    expect(framesForSelection(scene, [inst(i)], i)).toHaveLength(1)
  })

  it('a_scaled_entered_instance_keeps_its_triad_axes', () => {
    const scene = new Scene()
    const i = scene.make_component(new Uint8Array([0]), new BigUint64Array([makeBox(scene, 0, 1, 0, 1, 1)]))
    scene.transform_instance(i, new Float64Array([2, 0, 0, 5, 0, 2, 0, 3, 0, 0, 2, 0]))
    scene.open_explode_session(i)
    const [f] = framesForSelection(scene, [], i)
    expect(f.origin[0]).toBeCloseTo(5)
    expect(f.origin[1]).toBeCloseTo(3)
    expect(f.x[0]).toBeCloseTo(1)
    expect(f.y[1]).toBeCloseTo(1)
  })

  it('a_nested_instance_is_drawn_in_world_space', () => {
    const scene = new Scene()
    const b = scene.make_component(new Uint8Array([0]), new BigUint64Array([makeBox(scene, 0, 1, 0, 1, 1)]))
    scene.transform_instance(b, shift(2))
    const box2 = makeBox(scene, 20, 21, 20, 21, 1)
    const a = scene.make_component(new Uint8Array([2, 0]), new BigUint64Array([b, box2]))
    scene.transform_instance(a, shift(10))
    const [f] = framesForSelection(scene, [inst(b)], null)
    // Either the world position (12,0,0) or no triad; never the local (2,0,0).
    expect(f === undefined || Math.abs(f.origin[0] - 12) < 1e-9).toBe(true)
    expect(f).toBeDefined()
  })

  it('a_user_hidden_component_has_no_triad', () => {
    const scene = new Scene()
    const i = scene.make_component(new Uint8Array([0]), new BigUint64Array([makeBox(scene, 0, 1, 0, 1, 1)]))
    expect(framesForSelection(scene, [inst(i)], null)).toHaveLength(1)
    scene.set_node_user_hidden(2, i, true)
    expect(framesForSelection(scene, [inst(i)], null)).toHaveLength(0)
  })
})
