/**
 * FlipTool carrying construction guides, against the REAL compiled wasm
 * module: guides selected with the nodes mirror with them in ONE undo step,
 * a mirrored copy copies them beside the nodes' copies, the mirror plane
 * comes from the nodes alone, and guides alone are refused. Object ▸ Flip
 * Along carries them the same way.
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import * as THREE from 'three'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import init, { Scene } from '../wasm/pkg/wasm_api.js'
import { FlipTool, flipAlongAxis } from './FlipTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'
import { setCopiesLinked } from '../settings/editing'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmBytes = await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm'))
  await init({ module_or_path: wasmBytes })
})

/** An L-shaped board in x in [2, 6], y in [0, 3], 0.5 tall: its gizmo
 *  centers on (4, 1.5, 0.25). */
function lBoard(scene: Scene): bigint {
  const pts: Array<[number, number]> = [[2, 0], [6, 0], [6, 1], [4, 1], [4, 3], [2, 3]]
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
  for (let i = 0; i < pts.length; i++) {
    const [ax, ay] = pts[i]
    const [bx, by] = pts[(i + 1) % pts.length]
    scene.sketch_add_segment(sketch, ax, ay, 0, bx, by, 0)
  }
  const regions = Array.from(scene.sketch_regions(sketch))
  return scene.extrude_region(sketch, regions[0], 0.5)
}

const obj = (id: bigint): NodeRef => ({ kind: 'object', id })
const ground = (x: number, y: number, z = 0): Snap => ({ x, y, z, kind: 'ground' })
/** A ray along -x through (y, z): it meets only the square across red. */
const redRay = (y: number, z: number): Ray => ({ origin: [50, y, z], direction: [-1, 0, 0] })

/** The tool on `selection` carrying `guides`, as the Viewport pushes them. */
function makeTool(scene: Scene, selection: NodeRef[], guides: bigint[], objectsGroup: THREE.Group | null = null) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const tool = new FlipTool(scene, preview, objectsGroup, selection, onCommit, onToast)
  tool.setSelectedGuides(guides)
  tool.activate()
  return { tool, preview, onCommit, onToast }
}

const pointAt = (scene: Scene, g: bigint): number[] => Array.from(scene.guide_geometry(g) ?? []).slice(0, 3)

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

describe('FlipTool — guides carried with the selection', () => {
  it('a flip in place mirrors the board and the guide in one step, about the board alone', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    const g = scene.add_guide_point(8, 1, 0)
    const { tool, onCommit } = makeTool(scene, [obj(b)], [g])
    expectOneStep(scene, () => {
      // Across red, x = 4: the board's center, never pulled toward the guide.
      tool.onPointerDown(ground(0, 0), redRay(1.5, 0.3))
    })
    expect(pointAt(scene, g)).toEqual([0, 1, 0])
    expect(onCommit).toHaveBeenCalledWith([obj(b)], false, [g])
  })

  it('the hover ghost mirrors the guide too', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    const g = scene.add_guide_line(8, 0, 0, 0, 1, 0)
    const { tool, preview } = makeTool(scene, [obj(b)], [g])
    tool.onPointerMove(ground(0, 0), redRay(1.5, 0.3))
    const ghost = preview.getObjectByName('FlipGhost')
    expect(ghost).toBeDefined()
    const lines: THREE.LineSegments[] = []
    ghost!.traverse((c) => { if (c instanceof THREE.LineSegments) lines.push(c) })
    expect(lines).toHaveLength(1)
  })

  it('an independent copy copies the guide beside the board; both copies become the selection', () => {
    setCopiesLinked(false)
    const scene = new Scene()
    const b = lBoard(scene)
    const g = scene.add_guide_point(8, 1, 0)
    const { tool, onCommit } = makeTool(scene, [obj(b)], [g])
    tool.toggleCopyMode()
    expectOneStep(scene, () => tool.onPointerDown(ground(0, 0), redRay(1.5, 0.3)))
    const [nodes, fullRefresh, guides] = onCommit.mock.calls[0] as [NodeRef[], boolean, bigint[]]
    expect(fullRefresh).toBe(false)
    expect(nodes).toHaveLength(1)
    expect(nodes[0].kind).toBe('object')
    expect(nodes[0].id).not.toBe(b)
    expect(guides).toHaveLength(1)
    expect(guides[0]).not.toBe(g)
    expect(pointAt(scene, g)).toEqual([8, 1, 0])
    expect(pointAt(scene, guides[0])).toEqual([0, 1, 0])
  })

  it('a linked copy copies the guide too', () => {
    setCopiesLinked(true)
    const scene = new Scene()
    const b = lBoard(scene)
    const g = scene.add_guide_point(8, 1, 0)
    const { tool, onCommit } = makeTool(scene, [obj(b)], [g])
    tool.toggleCopyMode()
    expectOneStep(scene, () => tool.onPointerDown(ground(0, 0), redRay(1.5, 0.3)))
    const [nodes, fullRefresh, guides] = onCommit.mock.calls[0] as [NodeRef[], boolean, bigint[]]
    expect(fullRefresh).toBe(true)
    expect(nodes[0].kind).toBe('instance')
    expect(guides).toHaveLength(1)
    expect(pointAt(scene, guides[0])).toEqual([0, 1, 0])
  })

  it('beside drawing alone, the guide mirrors in place and copies through the drawing matrix', () => {
    setCopiesLinked(false)
    const scene = new Scene()
    // A drawn line in x in [7, 8]: the mirror across red is x = 7.5.
    const s = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
    scene.sketch_add_segment(s, 7, 0, 0, 8, 2, 0)
    const g = scene.add_guide_point(9, 1, 0)
    const { tool, onCommit } = makeTool(scene, [{ kind: 'sketch', id: s }], [g])
    expectOneStep(scene, () => tool.onPointerDown(ground(0, 0), redRay(1, 0)))
    expect(pointAt(scene, g)[0]).toBeCloseTo(6, 9)
    expect(onCommit.mock.calls[0][2]).toEqual([g])

    // The copy: the original guide (now at x = 6) stays, its copy lands on 9.
    tool.toggleCopyMode()
    expectOneStep(scene, () => tool.onPointerDown(ground(0, 0), redRay(1, 0)))
    const guides = onCommit.mock.calls[1][2] as bigint[]
    expect(guides).toHaveLength(1)
    expect(pointAt(scene, g)[0]).toBeCloseTo(6, 9)
    expect(pointAt(scene, guides[0])[0]).toBeCloseTo(9, 9)
  })

  it('guides alone are refused: a click on nothing says there is nothing to flip', () => {
    const scene = new Scene()
    lBoard(scene)
    const g = scene.add_guide_point(8, 1, 0)
    const before = scene.state_hash()
    const { tool, onCommit, onToast, preview } = makeTool(scene, [], [g])
    tool.setSelectionAcquirer(() => null)
    expect(tool.statusHint()).toMatch(/Guides can't be flipped on their own/)
    expect(preview.getObjectByName('FlipGizmo')).toBeUndefined()
    tool.onPointerDown(ground(3, 0.5), redRay(1.5, 0.3))
    expect(onToast).toHaveBeenCalledWith('Nothing to flip')
    expect(onCommit).not.toHaveBeenCalled()
    expect(scene.state_hash()).toBe(before)
  })

  it('with guides alone selected, a click on an object picks it instead of the guides', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    const g = scene.add_guide_point(8, 1, 0)
    const before = scene.state_hash()
    const { tool, onCommit, onToast, preview } = makeTool(scene, [], [g])
    tool.setSelectionAcquirer(() => [obj(b)])
    tool.onPointerDown(ground(3, 0.5), redRay(1.5, 0.3))
    expect(onToast).not.toHaveBeenCalled()
    expect(preview.getObjectByName('FlipGizmo')).toBeDefined()
    expect(scene.state_hash()).toBe(before)
    // The pick replaced the selection: the next flip leaves the guide alone.
    tool.onPointerDown(ground(0, 0), redRay(1.5, 0.3))
    expect(onCommit).toHaveBeenCalledWith([obj(b)], false, [])
    expect(pointAt(scene, g)).toEqual([8, 1, 0])
  })

  it('inside a component the member and the guide flip in place in one step', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    const inst = scene.make_component(new Uint8Array([0]), new BigUint64Array([b]))
    const def = scene.instance_def(inst) as bigint
    const member = scene.instance_expanded_members(inst)[0]
    const g = scene.add_guide_point(8, 1, 0)
    const { tool, onCommit } = makeTool(scene, [obj(member)], [g])
    tool.setEditContext({ kind: 'instance', id: inst, component: def })
    tool.setSelection([obj(member)])
    expectOneStep(scene, () => tool.onPointerDown(ground(0, 0), redRay(1.5, 0.3)))
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(pointAt(scene, g)[0]).toBeCloseTo(0, 9)
  })
})

describe('flipAlongAxis carrying guides', () => {
  it('mirrors the guides with the nodes in one step', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    const g = scene.add_guide_point(8, 1, 0)
    let flipped: ReturnType<typeof flipAlongAxis> = null
    expectOneStep(scene, () => {
      flipped = flipAlongAxis(scene, [obj(b)], 0, null, [g])
    })
    expect(flipped).toEqual({ nodes: [obj(b)], guides: [g] })
    expect(pointAt(scene, g)).toEqual([0, 1, 0])
  })
})
