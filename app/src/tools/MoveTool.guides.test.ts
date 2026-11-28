/**
 * MoveTool on construction guides: the guide target (`setGuideTarget`), the
 * click that takes a guide (`setGuideHooks`) and its ephemeral selection,
 * and the copy / ×N array / retype paths through `transform_guide` and
 * `duplicate_guide_array`; and the guides carried with nodes, or several at
 * once (`setCarriedGuides`), through the mixed transform and copy calls —
 * against a mocked WasmScene. `isMac` is mocked to true so a bare Alt tap
 * toggles copy mode (see MoveTool.test.ts).
 */

import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { MoveTool } from './MoveTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'

vi.mock('../platform', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../platform')>()
  return { ...actual, isMac: true, COPY_MODIFIER_KEY: 'Alt', COPY_MODIFIER_LABEL: 'Option' }
})

const ray: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }
const snapAt = (x: number, y = 0, z = 0): Snap => ({ x, y, z, kind: 'ground' })
const key = (k: string): KeyboardEvent =>
  ({ key: k, repeat: false, preventDefault: () => { /* no-op */ } }) as unknown as KeyboardEvent
const type = (tool: MoveTool, text: string): void => {
  for (const ch of text) tool.onKey(key(ch))
}
const translationOf = (affine: Float64Array): number[] => [affine[3], affine[7], affine[11]]

/** The members MoveTool calls on a guide gesture. Every commit, undo and
 *  redo moves the history generation, as the kernel's would. */
function makeScene() {
  const state = { gen: 1n, nextGuide: 50n }
  const bump = () => { state.gen += 1n }
  const scene = {
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    guide_kind: vi.fn(() => 'line'),
    guide_geometry: vi.fn(() => new Float64Array([0, 0, 0, 0, 1, 0])),
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
    /** Copies every listed node and guide once per placement, placement-major. */
    duplicate_mixed_selection_array: vi.fn(
      (_k: Uint8Array, ids: BigUint64Array, guides: BigUint64Array, _a: Float64Array, count: number, _l: boolean) => {
        bump()
        return mixedCopyJson(state, ids, guides, count)
      },
    ),
    duplicate_mixed_selection_placements: vi.fn(
      (_k: Uint8Array, ids: BigUint64Array, guides: BigUint64Array, affines: Float64Array, _l: boolean) => {
        bump()
        return mixedCopyJson(state, ids, guides, affines.length / 12)
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
  return scene
}

/** The mixed copy calls' JSON: object copies from 100, guide copies from
 *  `nextGuide`, placement-major. */
function mixedCopyJson(state: { nextGuide: bigint }, ids: BigUint64Array, guides: BigUint64Array, reps: number): string {
  const nodes: { kind: string; id: string }[] = []
  const copies: string[] = []
  for (let r = 0; r < reps; r += 1) {
    ids.forEach((_, i) => nodes.push({ kind: 'object', id: String(100 + r * ids.length + i) }))
    guides.forEach(() => copies.push(String(state.nextGuide++)))
  }
  return JSON.stringify({ nodes, linked: [], guides: copies })
}

/**
 * A tool with no nodes selected, `selected` as the app's selected guide,
 * and a guide acquirer that hits `hit` (null: a miss). The acquirer and the
 * commit hook mirror the Viewport: both select the guide they name in the
 * app, which pushes it back into the tool.
 */
function makeTool(opts: { selected?: bigint | null; hit?: bigint | null; nodes?: NodeRef[] } = {}) {
  const scene = makeScene()
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const tool = new MoveTool(scene as never, preview, null, opts.nodes ?? [], onCommit, onToast)
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
  const onGuide = vi.fn((g: bigint | null) => select(g))
  tool.setGuideHooks(acquire, onGuide)
  const nodeAcquire = vi.fn(() => null)
  tool.setSelectionAcquirer(nodeAcquire)
  return { tool, scene, preview, onCommit, onToast, app, acquire, onGuide, nodeAcquire }
}

/** Click a base point at the origin, then a destination at (x, 0, 0). */
function clickMove(tool: MoveTool, x: number): void {
  tool.onPointerDown(snapAt(0), ray)
  tool.onPointerMove(snapAt(x), ray)
  tool.onPointerDown(snapAt(x), ray)
}

describe('MoveTool — moving a selected guide', () => {
  it('commits one transform_guide by the clicked vector and keeps the guide selected', () => {
    const t = makeTool({ selected: 7n })
    t.tool.onPointerDown(snapAt(0), ray)
    // The guide's ghost, and the dashed base→destination line.
    expect(t.preview.children).toHaveLength(2)
    expect(t.preview.children.filter((c) => (c as THREE.LineSegments).material instanceof THREE.LineDashedMaterial))
      .toHaveLength(1)
    t.tool.onPointerMove(snapAt(2), ray)
    t.tool.onPointerDown(snapAt(2), ray)

    expect(t.scene.transform_guide).toHaveBeenCalledTimes(1)
    const [g, affine] = t.scene.transform_guide.mock.calls[0]
    expect(g).toBe(7n)
    expect(translationOf(affine)).toEqual([2, 0, 0])
    expect(t.onGuide).toHaveBeenLastCalledWith(7n)
    expect(t.app.guide).toBe(7n)
    expect(t.onCommit).not.toHaveBeenCalled() // no node selection churn
    expect(t.preview.children).toHaveLength(0)
  })

  it('a typed distance along a locked axis moves the guide exactly that far', () => {
    const t = makeTool({ selected: 7n })
    t.tool.onPointerDown(snapAt(0), ray)
    t.tool.onKey(key('ArrowLeft')) // lock green (Y)
    type(t.tool, '3')
    t.tool.onKey(key('Enter'))
    expect(translationOf(t.scene.transform_guide.mock.calls[0][1])).toEqual([0, 3, 0])
  })

  it('a click off any guide is the selected guide\'s base point — no node is acquired', () => {
    const t = makeTool({ selected: 7n, hit: null })
    clickMove(t.tool, 1)
    expect(t.nodeAcquire).not.toHaveBeenCalled()
    expect(t.scene.transform_guide.mock.calls[0][0]).toBe(7n)
  })

  it('a retyped distance redoes the guide move: one undo, the same guide moved again', () => {
    const t = makeTool({ selected: 7n })
    clickMove(t.tool, 2)
    type(t.tool, '5')
    t.tool.onKey(key('Enter'))
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.scene.transform_guide).toHaveBeenCalledTimes(2)
    const [g, affine] = t.scene.transform_guide.mock.calls[1]
    expect(g).toBe(7n)
    expect(translationOf(affine)).toEqual([5, 0, 0])
  })

  it('a refused move toasts the kernel error and lands nothing', () => {
    const t = makeTool({ selected: 7n })
    t.scene.transform_guide.mockImplementationOnce(() => {
      throw new Error('UnknownGuide: no such guide in this document')
    })
    clickMove(t.tool, 2)
    expect(t.onToast).toHaveBeenCalledTimes(1)
    expect(t.onGuide).not.toHaveBeenCalled()
  })
})

describe('MoveTool — copying a guide', () => {
  it('copy mode commits one duplicate_guide_array (count 1); the copy becomes the selected guide', () => {
    const t = makeTool({ selected: 7n })
    t.tool.onKey(key('Alt'))
    clickMove(t.tool, 1)
    expect(t.scene.transform_guide).not.toHaveBeenCalled()
    const [g, affine, count] = t.scene.duplicate_guide_array.mock.calls[0]
    expect([g, count]).toEqual([7n, 1])
    expect(translationOf(affine)).toEqual([1, 0, 0])
    expect(t.app.guide).toBe(50n)
  })

  it('3x retracts the single copy and lays three from the ORIGINAL guide in one call', () => {
    const t = makeTool({ selected: 7n })
    t.tool.onKey(key('Alt'))
    clickMove(t.tool, 1)
    type(t.tool, '3x')
    t.tool.onKey(key('Enter'))
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    const [g, affine, count] = t.scene.duplicate_guide_array.mock.calls[1]
    expect([g, count]).toEqual([7n, 3])
    expect(translationOf(affine)).toEqual([1, 0, 0])
    expect(t.app.guide).toBe(53n) // the last of the three
  })

  it('/4 divides the committed distance into four steps', () => {
    const t = makeTool({ selected: 7n })
    t.tool.onKey(key('Alt'))
    clickMove(t.tool, 2)
    type(t.tool, '/4')
    t.tool.onKey(key('Enter'))
    const [, affine, count] = t.scene.duplicate_guide_array.mock.calls[1]
    expect(count).toBe(4)
    expect(translationOf(affine)).toEqual([0.5, 0, 0])
  })
})

describe('MoveTool — a click on a guide takes it, for the gesture only', () => {
  it('with nothing selected, the clicked guide moves and the selection is empty afterwards', () => {
    const t = makeTool({ hit: 9n })
    t.tool.onPointerDown(snapAt(0), ray)
    expect(t.app.guide).toBe(9n) // highlighted while it moves
    expect(t.nodeAcquire).not.toHaveBeenCalled()
    t.tool.onPointerMove(snapAt(2), ray)
    t.tool.onPointerDown(snapAt(2), ray)
    expect(t.scene.transform_guide.mock.calls[0][0]).toBe(9n)
    expect(t.app.guide).toBeNull()
  })

  it('a click on ANOTHER guide moves that one, and the earlier selection comes back', () => {
    const t = makeTool({ selected: 7n, hit: 9n })
    clickMove(t.tool, 1)
    expect(t.scene.transform_guide.mock.calls[0][0]).toBe(9n)
    expect(t.app.guide).toBe(7n)
  })

  it('a click on the guide already selected keeps it selected', () => {
    const t = makeTool({ selected: 7n, hit: 7n })
    clickMove(t.tool, 1)
    expect(t.app.guide).toBe(7n)
  })

  it('Escape mid-gesture gives the selection back', () => {
    const t = makeTool({ hit: 9n })
    t.tool.onPointerDown(snapAt(0), ray)
    t.tool.onKey(key('Escape'))
    expect(t.scene.transform_guide).not.toHaveBeenCalled()
    expect(t.app.guide).toBeNull()
  })

  it('a second click on the base point moves nothing and gives the selection back', () => {
    const t = makeTool({ hit: 9n })
    t.tool.onPointerDown(snapAt(0), ray)
    t.tool.onPointerDown(snapAt(0), ray)
    expect(t.scene.transform_guide).not.toHaveBeenCalled()
    expect(t.app.guide).toBeNull()
  })

  it('a copy of a click-taken guide, and its array, leave nothing selected', () => {
    const t = makeTool({ hit: 9n })
    t.tool.onKey(key('Alt'))
    clickMove(t.tool, 1)
    expect(t.app.guide).toBeNull()
    type(t.tool, '2x')
    t.tool.onKey(key('Enter'))
    expect(t.scene.duplicate_guide_array.mock.calls[1][0]).toBe(9n)
    expect(t.app.guide).toBeNull()
  })

  it('with nodes selected, a click on a guide is their base point', () => {
    const t = makeTool({ hit: 9n, nodes: [{ kind: 'object', id: 1n }] })
    clickMove(t.tool, 1)
    expect(t.acquire).not.toHaveBeenCalled()
    expect(t.scene.transform_guide).not.toHaveBeenCalled()
    expect(t.scene.transform_selection).toHaveBeenCalledTimes(1)
  })

  it('a miss falls through to the node acquirer, then the hint', () => {
    const t = makeTool({ hit: null })
    t.tool.onPointerDown(snapAt(0), ray)
    expect(t.nodeAcquire).toHaveBeenCalledTimes(1)
    expect(t.onToast).toHaveBeenCalledWith('Click an object to move it')
  })
})

const OBJ: NodeRef = { kind: 'object', id: 1n }

/** A tool carrying `guides` with `nodes`, as the Viewport pushes them, and
 *  the app selection its commits land on. */
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

describe('MoveTool — guides carried with the selection', () => {
  it('moves the nodes and every carried guide in one transform_mixed_selection', () => {
    const t = makeCarrying([OBJ], [7n, 8n])
    t.tool.onPointerDown(snapAt(0), ray)
    // One overlay line per carried guide (no object meshes in this mock), and the dashed link.
    const ghost = t.preview.children.find((c) => c instanceof THREE.Group) as THREE.Group
    expect(ghost.children.filter((c) => c instanceof THREE.LineSegments)).toHaveLength(2)
    t.tool.onPointerMove(snapAt(2), ray)
    t.tool.onPointerDown(snapAt(2), ray)

    expect(t.scene.transform_selection).not.toHaveBeenCalled()
    expect(t.scene.transform_mixed_selection).toHaveBeenCalledTimes(1)
    const [kinds, ids, sketches, guides, affine] = t.scene.transform_mixed_selection.mock.calls[0]
    expect([...kinds]).toEqual([0])
    expect([...ids]).toEqual([1n])
    expect([...sketches]).toEqual([])
    expect([...guides]).toEqual([7n, 8n])
    expect(translationOf(affine)).toEqual([2, 0, 0])
    expect(t.scene.history_bracket_open).not.toHaveBeenCalled() // one kernel step
    expect(t.landed).toEqual([{ nodes: [OBJ], guides: [7n, 8n] }])
    expect(t.scene.transform_guide).not.toHaveBeenCalled()
  })

  it('several guides alone move together from the clicked base point; no guide or node is taken', () => {
    const t = makeCarrying([], [7n, 8n])
    expect(t.tool.clickAcquire()).toBeNull()
    clickMove(t.tool, 1)
    expect(t.acquire).not.toHaveBeenCalled()
    expect(t.nodeAcquire).not.toHaveBeenCalled()
    const [kinds, ids, , guides] = t.scene.transform_mixed_selection.mock.calls[0]
    expect([...kinds, ...ids]).toEqual([])
    expect([...guides]).toEqual([7n, 8n])
    expect(t.landed).toEqual([{ nodes: [], guides: [7n, 8n] }])
  })

  it('a retyped distance redoes the move with the same guides', () => {
    const t = makeCarrying([OBJ], [7n])
    clickMove(t.tool, 2)
    type(t.tool, '5')
    t.tool.onKey(key('Enter'))
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.scene.transform_mixed_selection).toHaveBeenCalledTimes(2)
    const [, , , guides, affine] = t.scene.transform_mixed_selection.mock.calls[1]
    expect([...guides]).toEqual([7n])
    expect(translationOf(affine)).toEqual([5, 0, 0])
  })

  it('a refused move toasts the kernel error and lands nothing', () => {
    const t = makeCarrying([OBJ], [7n])
    t.scene.transform_mixed_selection.mockImplementationOnce(() => {
      throw new Error('UnknownGuide: no such guide in this document')
    })
    clickMove(t.tool, 2)
    expect(t.onToast).toHaveBeenCalledTimes(1)
    expect(t.landed).toEqual([])
  })

  it('inside a component the members move in the definition and the guides in the world, in one bracket', () => {
    const t = makeCarrying([OBJ], [7n])
    t.tool.setEditContext({ kind: 'instance', id: 40n, component: 41n })
    clickMove(t.tool, 2)
    expect(t.scene.transform_def_selection).toHaveBeenCalledTimes(1)
    expect(t.scene.transform_def_selection.mock.calls[0][0]).toBe(40n)
    const [kinds, ids, sketches, guides, affine] = t.scene.transform_mixed_selection.mock.calls[0]
    expect([...kinds, ...ids, ...sketches]).toEqual([])
    expect([...guides]).toEqual([7n])
    expect(translationOf(affine)).toEqual([2, 0, 0])
    expect(t.scene.history_bracket_open).toHaveBeenCalledTimes(1)
    expect(t.scene.history_bracket_commit).toHaveBeenCalledTimes(1)
    expect(t.scene.history_bracket_abort).not.toHaveBeenCalled()
  })
})

describe('MoveTool — copying guides carried with the selection', () => {
  it('a copy lays the nodes and guides once in one mixed call; every copy becomes the selection', () => {
    const t = makeCarrying([OBJ], [7n, 8n])
    t.tool.onKey(key('Alt'))
    clickMove(t.tool, 1)
    expect(t.scene.duplicate_selection_array).not.toHaveBeenCalled()
    expect(t.scene.duplicate_guide_array).not.toHaveBeenCalled()
    const [, ids, guides, affine, count] = t.scene.duplicate_mixed_selection_array.mock.calls[0]
    expect([...ids]).toEqual([1n])
    expect([...guides]).toEqual([7n, 8n])
    expect([translationOf(affine), count]).toEqual([[1, 0, 0], 1])
    expect(t.landed).toEqual([{ nodes: [{ kind: 'object', id: 100n }], guides: [50n, 51n] }])
  })

  it('3x retracts the single copy and lays three from the ORIGINAL nodes and guides', () => {
    const t = makeCarrying([OBJ], [7n])
    t.tool.onKey(key('Alt'))
    clickMove(t.tool, 1)
    type(t.tool, '3x')
    t.tool.onKey(key('Enter'))
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    const [, ids, guides, , count] = t.scene.duplicate_mixed_selection_array.mock.calls[1]
    expect([...ids]).toEqual([1n])
    expect([...guides]).toEqual([7n])
    expect(count).toBe(3)
    expect(t.landed[1].guides).toEqual([51n, 52n, 53n])
  })

  it('a grid lays every carried guide at each slot through the mixed placements call', () => {
    const t = makeCarrying([OBJ], [7n])
    t.tool.onKey(key('Alt'))
    clickMove(t.tool, 1)
    type(t.tool, '2x,2x')
    t.tool.onKey(key('Enter'))
    const [, ids, guides, affines] = t.scene.duplicate_mixed_selection_placements.mock.calls[0]
    expect([...ids]).toEqual([1n])
    expect([...guides]).toEqual([7n])
    expect(affines.length / 12).toBe(8) // a 3 x 3 lattice less the original's slot
    expect(t.landed[1].guides).toHaveLength(8)
  })

  it('a grid of guides alone is refused: they copy along one direction only', () => {
    const t = makeCarrying([], [7n, 8n])
    t.tool.onKey(key('Alt'))
    clickMove(t.tool, 1)
    type(t.tool, '2x,2x')
    t.tool.onKey(key('Enter'))
    expect(t.onToast).toHaveBeenCalledWith('Guides copy along one direction only — type a single count.')
    expect(t.scene.scene_undo).not.toHaveBeenCalled()
    expect(t.scene.duplicate_mixed_selection_placements).not.toHaveBeenCalled()
  })

  it('a row of guides alone copies them all in one call', () => {
    const t = makeCarrying([], [7n, 8n])
    t.tool.onKey(key('Alt'))
    clickMove(t.tool, 1)
    type(t.tool, '2x')
    t.tool.onKey(key('Enter'))
    const [kinds, , guides, , count] = t.scene.duplicate_mixed_selection_array.mock.calls[1]
    expect([...kinds]).toEqual([])
    expect([...guides, count]).toEqual([7n, 8n, 2])
    expect(t.landed[1]).toEqual({ nodes: [], guides: [52n, 53n, 54n, 55n] })
  })
})
