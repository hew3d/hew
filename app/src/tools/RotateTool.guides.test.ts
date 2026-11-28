/**
 * RotateTool on construction guides: the guide target, the pivot click on a
 * guide (taken for the gesture only with nothing selected; just a pivot
 * with a guide already selected — SketchUp's rule), and the copy / ×N fan /
 * retype paths through `transform_guide` and `duplicate_guide_array`; and
 * the guides carried with nodes, or several at once (`setCarriedGuides`),
 * through the mixed transform and copy calls — against a mocked WasmScene. `isMac` is mocked to true so a bare Alt tap
 * toggles copy mode (see RotateTool.test.ts).
 */

import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { RotateTool } from './RotateTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'

vi.mock('../platform', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../platform')>()
  return { ...actual, isMac: true, COPY_MODIFIER_KEY: 'Alt', COPY_MODIFIER_LABEL: 'Option' }
})

/** A ray straight down through (x, y): the ground rotation plane at z = 0. */
const rayAt = (x: number, y: number): Ray => ({ origin: [x, y, 5], direction: [0, 0, -1] })
const snapAt = (x: number, y: number): Snap => ({ x, y, z: 0, kind: 'ground' })
const key = (k: string): KeyboardEvent =>
  ({ key: k, repeat: false, preventDefault: () => { /* no-op */ } }) as unknown as KeyboardEvent
const type = (tool: RotateTool, text: string): void => {
  for (const ch of text) tool.onKey(key(ch))
}
/** The angle of a row-major 3×4 affine rotating about world Z. */
const thetaOf = (affine: Float64Array): number => Math.atan2(affine[4], affine[0])

/** The members RotateTool calls on a guide gesture. Every commit, undo and
 *  redo moves the history generation, as the kernel's would. */
function makeScene() {
  const state = { gen: 1n, nextGuide: 50n }
  const bump = () => { state.gen += 1n }
  return {
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    guide_kind: vi.fn(() => 'line'),
    guide_geometry: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0])),
    transform_guide: vi.fn((_g: bigint, _a: Float64Array) => bump()),
    duplicate_guide_array: vi.fn((_g: bigint, _a: Float64Array, count: number) => {
      bump()
      return BigUint64Array.from({ length: count }, () => state.nextGuide++)
    }),
    duplicate_selection_array: vi.fn(() => { bump(); return [] }),
    transform_selection: vi.fn(() => bump()),
    transform_mixed_selection: vi.fn(
      (_k: Uint8Array, _i: BigUint64Array, _s: BigUint64Array, _g: BigUint64Array, _a: Float64Array) => bump(),
    ),
    transform_def_selection: vi.fn((_instance: bigint, ..._rest: unknown[]) => bump()),
    /** Copies every listed node and guide once per copy, copy-major. */
    duplicate_mixed_selection_array: vi.fn(
      (_k: Uint8Array, ids: BigUint64Array, guides: BigUint64Array, _a: Float64Array, count: number, _l: boolean) => {
        bump()
        return mixedCopyJson(state, ids, guides, count)
      },
    ),
    history_bracket_open: vi.fn(),
    history_bracket_commit: vi.fn(() => bump()),
    history_bracket_abort: vi.fn(),
    history_generation: vi.fn(() => state.gen),
    max_array_count: vi.fn(() => 1000),
    scene_undo: vi.fn(() => { bump(); return { free: () => { /* no-op */ } } }),
    scene_redo: vi.fn(() => { bump(); return { free: () => { /* no-op */ } } }),
  }
}

/** The mixed copy call's JSON: object copies from 100, guide copies from
 *  `nextGuide`, copy-major. */
function mixedCopyJson(state: { nextGuide: bigint }, ids: BigUint64Array, guides: BigUint64Array, reps: number): string {
  const nodes: { kind: string; id: string }[] = []
  const copies: string[] = []
  for (let r = 0; r < reps; r += 1) {
    ids.forEach((_, i) => nodes.push({ kind: 'object', id: String(100 + r * ids.length + i) }))
    guides.forEach(() => copies.push(String(state.nextGuide++)))
  }
  return JSON.stringify({ nodes, linked: [], guides: copies })
}

/** A tool with `selected` as the app's guide and a guide acquirer hitting
 *  `hit`; both hooks select what they name in the app, which pushes it back
 *  into the tool, as the Viewport does. */
function makeTool(opts: { selected?: bigint | null; hit?: bigint | null; nodes?: NodeRef[] } = {}) {
  const scene = makeScene()
  const preview = new THREE.Group()
  const onToast = vi.fn()
  const onCommit = vi.fn()
  const tool = new RotateTool(scene as never, preview, null, opts.nodes ?? [], onCommit, onToast)
  const app = { guide: opts.selected ?? null }
  const select = (g: bigint | null) => {
    app.guide = g
    tool.setGuideTarget(g)
  }
  tool.setGuideTarget(app.guide)
  const acquire = vi.fn(() => {
    const g = opts.hit ?? null
    if (g !== null) select(g)
    return g
  })
  tool.setGuideHooks(acquire, (g) => select(g))
  const nodeAcquire = vi.fn(() => null)
  tool.setSelectionAcquirer(nodeAcquire)
  return { tool, scene, preview, onToast, onCommit, app, acquire, nodeAcquire }
}

/** Pivot at the origin, reference along +X: the sweep is about +Z. */
function setPivotAndReference(tool: RotateTool): void {
  tool.onPointerMove(snapAt(0, 0), rayAt(0, 0))
  tool.onPointerDown(snapAt(0, 0), rayAt(0, 0))
  tool.onPointerDown(snapAt(1, 0), rayAt(1, 0))
}

/** A full quarter turn about +Z through the origin. */
function rotateQuarter(tool: RotateTool): void {
  setPivotAndReference(tool)
  tool.onPointerMove(snapAt(0, 1), rayAt(0, 1))
  tool.onPointerDown(snapAt(0, 1), rayAt(0, 1))
}

describe('RotateTool — rotating a selected guide', () => {
  it('commits one transform_guide by the swept angle; the guide stays selected', () => {
    const t = makeTool({ selected: 7n })
    setPivotAndReference(t.tool)
    expect(t.preview.children.some((c) => c instanceof THREE.LineSegments)).toBe(true) // the ghost
    t.tool.onPointerMove(snapAt(0, 1), rayAt(0, 1))
    t.tool.onPointerDown(snapAt(0, 1), rayAt(0, 1))

    expect(t.scene.transform_guide).toHaveBeenCalledTimes(1)
    const [g, affine] = t.scene.transform_guide.mock.calls[0]
    expect(g).toBe(7n)
    expect(thetaOf(affine)).toBeCloseTo(Math.PI / 2, 9)
    expect(t.app.guide).toBe(7n)
  })

  it('the pivot click on ANOTHER guide only sets the pivot — the selected guide rotates', () => {
    const t = makeTool({ selected: 7n, hit: 9n })
    rotateQuarter(t.tool)
    expect(t.acquire).not.toHaveBeenCalled()
    expect(t.scene.transform_guide.mock.calls[0][0]).toBe(7n)
    expect(t.app.guide).toBe(7n)
  })

  it('a typed angle rotates the guide exactly; a retyped one redoes it', () => {
    const t = makeTool({ selected: 7n })
    setPivotAndReference(t.tool)
    t.tool.onPointerMove(snapAt(0, 1), rayAt(0, 1))
    type(t.tool, '30')
    t.tool.onKey(key('Enter'))
    expect(thetaOf(t.scene.transform_guide.mock.calls[0][1])).toBeCloseTo(Math.PI / 6, 9)

    type(t.tool, '45')
    t.tool.onKey(key('Enter'))
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    const [g, affine] = t.scene.transform_guide.mock.calls[1]
    expect(g).toBe(7n)
    expect(thetaOf(affine)).toBeCloseTo(Math.PI / 4, 9)
  })

  it('a refused rotation toasts the kernel error', () => {
    const t = makeTool({ selected: 7n })
    t.scene.transform_guide.mockImplementationOnce(() => {
      throw new Error('UnknownGuide: no such guide in this document')
    })
    rotateQuarter(t.tool)
    expect(t.onToast).toHaveBeenCalledTimes(1)
  })
})

describe('RotateTool — copying a guide', () => {
  it('copy mode commits one duplicate_guide_array (count 1); the copy becomes the selected guide', () => {
    const t = makeTool({ selected: 7n })
    t.tool.onKey(key('Alt'))
    rotateQuarter(t.tool)
    expect(t.scene.transform_guide).not.toHaveBeenCalled()
    const [g, affine, count] = t.scene.duplicate_guide_array.mock.calls[0]
    expect([g, count]).toEqual([7n, 1])
    expect(thetaOf(affine)).toBeCloseTo(Math.PI / 2, 9)
    expect(t.app.guide).toBe(50n)
  })

  it('/6 retracts the copy and fans six copies from the ORIGINAL guide, one call on the step', () => {
    const t = makeTool({ selected: 7n })
    t.tool.onKey(key('Alt'))
    rotateQuarter(t.tool)
    type(t.tool, '/6')
    t.tool.onKey(key('Enter'))
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    const [g, affine, count] = t.scene.duplicate_guide_array.mock.calls[1]
    expect([g, count]).toEqual([7n, 6])
    expect(thetaOf(affine)).toBeCloseTo(Math.PI / 12, 9) // 90° / 6
    expect(t.app.guide).toBe(56n) // the last of the six
  })
})

describe('RotateTool — a pivot click on a guide with nothing selected takes it, for the gesture only', () => {
  it('the clicked guide rotates and the selection is empty afterwards', () => {
    const t = makeTool({ hit: 9n })
    t.tool.onPointerDown(snapAt(0, 0), rayAt(0, 0))
    expect(t.app.guide).toBe(9n) // highlighted while it rotates
    expect(t.nodeAcquire).not.toHaveBeenCalled()
    t.tool.onPointerDown(snapAt(1, 0), rayAt(1, 0))
    t.tool.onPointerMove(snapAt(0, 1), rayAt(0, 1))
    t.tool.onPointerDown(snapAt(0, 1), rayAt(0, 1))
    expect(t.scene.transform_guide.mock.calls[0][0]).toBe(9n)
    expect(t.app.guide).toBeNull()
  })

  it('Escape at the reference stage gives the selection back', () => {
    const t = makeTool({ hit: 9n })
    setPivotAndReference(t.tool)
    t.tool.onKey(key('Escape'))
    expect(t.scene.transform_guide).not.toHaveBeenCalled()
    expect(t.app.guide).toBeNull()
  })

  it('a zero sweep rotates nothing and gives the selection back', () => {
    const t = makeTool({ hit: 9n })
    setPivotAndReference(t.tool)
    t.tool.onPointerDown(snapAt(1, 0), rayAt(1, 0))
    expect(t.scene.transform_guide).not.toHaveBeenCalled()
    expect(t.app.guide).toBeNull()
  })

  it('a fan of a click-taken guide leaves nothing selected', () => {
    const t = makeTool({ hit: 9n })
    t.tool.onKey(key('Alt'))
    rotateQuarter(t.tool)
    type(t.tool, '3x')
    t.tool.onKey(key('Enter'))
    expect(t.scene.duplicate_guide_array.mock.calls[1][0]).toBe(9n)
    expect(t.app.guide).toBeNull()
  })

  it('with nodes selected, a pivot click on a guide is their pivot', () => {
    const t = makeTool({ hit: 9n, nodes: [{ kind: 'object', id: 1n }] })
    rotateQuarter(t.tool)
    expect(t.acquire).not.toHaveBeenCalled()
    expect(t.scene.transform_guide).not.toHaveBeenCalled()
    expect(t.scene.transform_selection).toHaveBeenCalledTimes(1)
  })

  it('a miss falls through to the node acquirer, then the hint', () => {
    const t = makeTool({ hit: null })
    t.tool.onPointerDown(snapAt(0, 0), rayAt(0, 0))
    expect(t.nodeAcquire).toHaveBeenCalledTimes(1)
    expect(t.onToast).toHaveBeenCalledWith('Click an object to rotate it')
  })
})

const OBJ: NodeRef = { kind: 'object', id: 1n }

/** A tool carrying `guides` with `nodes`, as the Viewport pushes them, and
 *  the app selection its commits (and arrays) land on. */
function makeCarrying(nodes: NodeRef[], guides: bigint[]) {
  const t = makeTool({ nodes, hit: 9n })
  t.tool.setCarriedGuides(guides)
  const landed: { nodes: NodeRef[]; guides: readonly bigint[] }[] = []
  t.onCommit.mockImplementation((n: NodeRef[], g: readonly bigint[]) => {
    landed.push({ nodes: n, guides: g })
    t.tool.setSelection(n)
    t.tool.setCarriedGuides(g)
  })
  return { ...t, landed }
}

describe('RotateTool — guides carried with the selection', () => {
  it('rotates the nodes and every carried guide in one transform_mixed_selection', () => {
    const t = makeCarrying([OBJ], [7n, 8n])
    setPivotAndReference(t.tool)
    // One overlay line per carried guide (no object meshes in this mock).
    const ghost = t.preview.children.find((c) => c instanceof THREE.Group && c.children.length === 2) as THREE.Group
    expect(ghost.children.every((c) => c instanceof THREE.LineSegments)).toBe(true)
    t.tool.onPointerMove(snapAt(0, 1), rayAt(0, 1))
    t.tool.onPointerDown(snapAt(0, 1), rayAt(0, 1))

    expect(t.scene.transform_selection).not.toHaveBeenCalled()
    expect(t.scene.transform_mixed_selection).toHaveBeenCalledTimes(1)
    const [kinds, ids, sketches, guides, affine] = t.scene.transform_mixed_selection.mock.calls[0]
    expect([...kinds]).toEqual([0])
    expect([...ids]).toEqual([1n])
    expect([...sketches]).toEqual([])
    expect([...guides]).toEqual([7n, 8n])
    expect(thetaOf(affine)).toBeCloseTo(Math.PI / 2, 9)
    expect(t.scene.history_bracket_open).not.toHaveBeenCalled() // one kernel step
    expect(t.landed).toEqual([{ nodes: [OBJ], guides: [7n, 8n] }])
    expect(t.scene.transform_guide).not.toHaveBeenCalled()
  })

  it('several guides alone rotate together about the clicked pivot; no guide or node is taken', () => {
    const t = makeCarrying([], [7n, 8n])
    expect(t.tool.clickAcquire()).toBeNull()
    rotateQuarter(t.tool)
    expect(t.acquire).not.toHaveBeenCalled()
    expect(t.nodeAcquire).not.toHaveBeenCalled()
    const [kinds, ids, , guides] = t.scene.transform_mixed_selection.mock.calls[0]
    expect([...kinds, ...ids]).toEqual([])
    expect([...guides]).toEqual([7n, 8n])
    expect(t.landed).toEqual([{ nodes: [], guides: [7n, 8n] }])
  })

  it('a retyped angle redoes the rotation with the same guides', () => {
    const t = makeCarrying([OBJ], [7n])
    rotateQuarter(t.tool)
    type(t.tool, '30')
    t.tool.onKey(key('Enter'))
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.scene.transform_mixed_selection).toHaveBeenCalledTimes(2)
    const [, , , guides, affine] = t.scene.transform_mixed_selection.mock.calls[1]
    expect([...guides]).toEqual([7n])
    expect(thetaOf(affine)).toBeCloseTo(Math.PI / 6, 9)
  })

  it('a refused rotation toasts the kernel error and lands nothing', () => {
    const t = makeCarrying([OBJ], [7n])
    t.scene.transform_mixed_selection.mockImplementationOnce(() => {
      throw new Error('UnknownGuide: no such guide in this document')
    })
    rotateQuarter(t.tool)
    expect(t.onToast).toHaveBeenCalledTimes(1)
    expect(t.landed).toEqual([])
  })

  it('inside a component the members rotate in the definition and the guides in the world, in one bracket', () => {
    const t = makeCarrying([OBJ], [7n])
    t.tool.setEditContext({ kind: 'instance', id: 40n, component: 41n })
    rotateQuarter(t.tool)
    expect(t.scene.transform_def_selection).toHaveBeenCalledTimes(1)
    expect(t.scene.transform_def_selection.mock.calls[0][0]).toBe(40n)
    const [kinds, ids, sketches, guides, affine] = t.scene.transform_mixed_selection.mock.calls[0]
    expect([...kinds, ...ids, ...sketches]).toEqual([])
    expect([...guides]).toEqual([7n])
    expect(thetaOf(affine)).toBeCloseTo(Math.PI / 2, 9)
    expect(t.scene.history_bracket_open).toHaveBeenCalledTimes(1)
    expect(t.scene.history_bracket_commit).toHaveBeenCalledTimes(1)
    expect(t.scene.history_bracket_abort).not.toHaveBeenCalled()
  })
})

describe('RotateTool — copying guides carried with the selection', () => {
  it('a copy lays the nodes and guides once in one mixed call; every copy becomes the selection', () => {
    const t = makeCarrying([OBJ], [7n, 8n])
    t.tool.onKey(key('Alt'))
    rotateQuarter(t.tool)
    expect(t.scene.duplicate_selection_array).not.toHaveBeenCalled()
    expect(t.scene.duplicate_guide_array).not.toHaveBeenCalled()
    const [, ids, guides, affine, count] = t.scene.duplicate_mixed_selection_array.mock.calls[0]
    expect([...ids]).toEqual([1n])
    expect([...guides]).toEqual([7n, 8n])
    expect(count).toBe(1)
    expect(thetaOf(affine)).toBeCloseTo(Math.PI / 2, 9)
    expect(t.landed).toEqual([{ nodes: [{ kind: 'object', id: 100n }], guides: [50n, 51n] }])
  })

  it('3x retracts the single copy and fans three from the ORIGINAL nodes and guides', () => {
    const t = makeCarrying([OBJ], [7n])
    t.tool.onKey(key('Alt'))
    rotateQuarter(t.tool)
    type(t.tool, '3x')
    t.tool.onKey(key('Enter'))
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    const [, ids, guides, affine, count] = t.scene.duplicate_mixed_selection_array.mock.calls[1]
    expect([...ids]).toEqual([1n])
    expect([...guides]).toEqual([7n])
    expect(count).toBe(3)
    expect(thetaOf(affine)).toBeCloseTo(Math.PI / 2, 9)
    expect(t.landed[1].guides).toEqual([51n, 52n, 53n])
  })

  it('/3 of guides alone divides the swept angle, all of them in one call', () => {
    const t = makeCarrying([], [7n, 8n])
    t.tool.onKey(key('Alt'))
    rotateQuarter(t.tool)
    type(t.tool, '/3')
    t.tool.onKey(key('Enter'))
    const [kinds, , guides, affine, count] = t.scene.duplicate_mixed_selection_array.mock.calls[1]
    expect([...kinds]).toEqual([])
    expect([...guides, count]).toEqual([7n, 8n, 3])
    expect(thetaOf(affine)).toBeCloseTo(Math.PI / 6, 9) // 90° / 3
    expect(t.landed[1]).toEqual({ nodes: [], guides: [52n, 53n, 54n, 55n, 56n, 57n] })
  })

  it('an angle retyped after 3x re-fans the nodes and guides at the new step', () => {
    const t = makeCarrying([OBJ], [7n])
    t.tool.onKey(key('Alt'))
    rotateQuarter(t.tool)
    type(t.tool, '3x')
    t.tool.onKey(key('Enter'))
    type(t.tool, '30')
    t.tool.onKey(key('Enter'))
    const [, ids, guides, affine, count] = t.scene.duplicate_mixed_selection_array.mock.calls[2]
    expect([...ids, ...guides, count]).toEqual([1n, 7n, 3])
    expect(thetaOf(affine)).toBeCloseTo(Math.PI / 6, 9)
    expect(t.landed[2].guides).toHaveLength(3)
  })
})
