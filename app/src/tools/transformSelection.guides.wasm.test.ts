/**
 * Construction guides carried by the shared transform and copy helpers
 * (`commitSelectionTransform`, `copyOwnersAndDrawing`), against the REAL
 * compiled wasm kernel: nodes and guides land in ONE undo step, which undo
 * and redo reproduce exactly, at the top level and inside a component.
 * Needs a rebuilt `app/src/wasm/pkg` (the mixed calls came with it).
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeAll, describe, expect, it } from 'vitest'
import init, { Scene } from '../wasm/pkg/wasm_api.js'
import type { Scene as WasmScene } from '../wasm/loader'
import { commitSelectionTransform, copyOwnersAndDrawing } from './transformSelection'
import { affineToFloat64, translationAffine } from './transformMath'
import type { NodeRef } from '../panels/treeModel'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmBytes = await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm'))
  await init({ module_or_path: wasmBytes })
})

/** A closed unit square on the ground at `x0`, as a sketch. */
function square(scene: Scene, x0 = 0): bigint {
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
  const c: Array<[number, number]> = [[x0, 0], [x0 + 1, 0], [x0 + 1, 1], [x0, 1]]
  for (let i = 0; i < 4; i++) {
    const a = c[i]
    const b = c[(i + 1) % 4]
    scene.sketch_add_segment(sketch, a[0], a[1], 0, b[0], b[1], 0)
  }
  return sketch
}

/** A [x0,x0+1]x[0,1] box on the ground, 1 tall. */
function makeBox(scene: Scene, x0 = 0): bigint {
  const sketch = square(scene, x0)
  const regions = Array.from(scene.sketch_regions(sketch))
  return scene.extrude_region(sketch, regions[0], 1)
}

const shift = (x: number): Float64Array => affineToFloat64(translationAffine(x, 0, 0))
const ws = (scene: Scene): WasmScene => scene as unknown as WasmScene

/** Run `body`, then check it added exactly one undo step, and that undo and
 *  redo bring back the states before and after it. */
function expectOneStep(scene: Scene, body: () => void): void {
  const depth = scene.undo_depth()
  const before = scene.state_hash()
  body()
  const after = scene.state_hash()
  expect(after).not.toBe(before)
  expect(scene.undo_depth()).toBe(depth + 1)
  scene.scene_undo().free()
  expect(scene.state_hash()).toBe(before)
  scene.scene_redo().free()
  expect(scene.state_hash()).toBe(after)
}

describe('commitSelectionTransform carrying guides', () => {
  it('moves an object and a guide in one step', () => {
    const scene = new Scene()
    const box = makeBox(scene)
    const guide = scene.add_guide_line(0, 2, 0, 0, 0, 1)
    expectOneStep(scene, () => {
      commitSelectionTransform(ws(scene), [{ kind: 'object', id: box }], shift(3), null, 'Move', [guide])
    })
    expect(scene.object_bounds(box)[0]).toBeCloseTo(3)
    expect(scene.guide_geometry(guide)?.[0]).toBeCloseTo(3)
  })

  it('moves guides alone in one step', () => {
    const scene = new Scene()
    makeBox(scene)
    const a = scene.add_guide_line(0, 2, 0, 0, 0, 1)
    const b = scene.add_guide_point(1, 2, 0)
    expectOneStep(scene, () => {
      commitSelectionTransform(ws(scene), [], shift(2), null, 'Move', [a, b])
    })
    expect(scene.guide_geometry(a)?.[0]).toBeCloseTo(2)
    expect(scene.guide_geometry(b)?.[0]).toBeCloseTo(3)
  })

  it('inside a component moves a member and a guide in one step', () => {
    const scene = new Scene()
    const box = makeBox(scene)
    const instance = scene.make_component(Uint8Array.of(0), BigUint64Array.of(box))
    const member = Array.from(scene.component_member_objects(scene.instance_def(instance)!))[0]
    const guide = scene.add_guide_line(0, 2, 0, 0, 0, 1)
    expectOneStep(scene, () => {
      commitSelectionTransform(ws(scene), [{ kind: 'object', id: member }], shift(3), instance, 'Move', [guide])
    })
    expect(scene.guide_geometry(guide)?.[0]).toBeCloseTo(3)
  })

  it('a stale guide refuses the whole move, the document untouched', () => {
    const scene = new Scene()
    const box = makeBox(scene)
    const guide = scene.add_guide_line(0, 2, 0, 0, 0, 1)
    scene.scene_undo().free() // the guide is gone
    const depth = scene.undo_depth()
    const before = scene.state_hash()
    expect(() =>
      commitSelectionTransform(ws(scene), [{ kind: 'object', id: box }], shift(3), null, 'Move', [guide]),
    ).toThrow(/UnknownGuide/)
    expect(scene.state_hash()).toBe(before)
    expect(scene.undo_depth()).toBe(depth)
  })
})

describe('copyOwnersAndDrawing copying guides', () => {
  /** The owners' copy as MoveTool makes it: one mixed call. */
  const mixedCopy = (scene: Scene, owners: bigint[], guides: bigint[], out: { guides: string[] }) => (): NodeRef[] => {
    const json = scene.duplicate_mixed_selection_array(
      new Uint8Array(owners.length),
      BigUint64Array.from(owners),
      BigUint64Array.from(guides),
      shift(2),
      1,
      false,
    )
    const raw = JSON.parse(json) as { nodes: { kind: 'object'; id: string }[]; guides: string[] }
    out.guides = raw.guides
    return raw.nodes.map((n) => ({ kind: n.kind, id: BigInt(n.id) }))
  }

  it('copies a whole sketch and a guide, with no owner, in one step', () => {
    const scene = new Scene()
    const sketch = square(scene)
    const guide = scene.add_guide_line(0, 2, 0, 0, 0, 1)
    const out = { guides: [] as string[] }
    expectOneStep(scene, () => {
      copyOwnersAndDrawing(ws(scene), [], [{ kind: 'sketch', id: sketch }], [shift(2)], mixedCopy(scene, [], [guide], out), true)
    })
    expect(out.guides).toHaveLength(1)
    expect(scene.guide_geometry(BigInt(out.guides[0]))?.[0]).toBeCloseTo(2)
  })

  it('copies an object, a sketch and a guide in one step', () => {
    const scene = new Scene()
    const box = makeBox(scene)
    const sketch = square(scene, 5)
    const guide = scene.add_guide_line(0, 2, 0, 0, 0, 1)
    const out = { guides: [] as string[] }
    let copies: NodeRef[] = []
    expectOneStep(scene, () => {
      copies = copyOwnersAndDrawing(
        ws(scene),
        [{ kind: 'object', id: box }],
        [{ kind: 'sketch', id: sketch }],
        [shift(2)],
        mixedCopy(scene, [box], [guide], out),
        true,
      )
    })
    expect(copies.some((n) => n.kind === 'object')).toBe(true)
    expect(out.guides).toHaveLength(1)
  })

  it('inside a component copies a member and a guide in one step, the guide staying after the close', () => {
    const scene = new Scene()
    const box = makeBox(scene)
    const instance = scene.make_component(Uint8Array.of(0), BigUint64Array.of(box))
    scene.open_explode_session(instance)
    const member = scene.explode_session_objects()![0]!
    const guide = scene.add_guide_line(0, 2, 0, 0, 0, 1)
    const out = { guides: [] as string[] }
    expectOneStep(scene, () => {
      copyOwnersAndDrawing(ws(scene), [{ kind: 'object', id: member }], [], [shift(2)], mixedCopy(scene, [member], [guide], out), true)
    })
    scene.close_innermost_session()
    expect(scene.guide_geometry(BigInt(out.guides[0]))?.[0]).toBeCloseTo(2)
  })
})
