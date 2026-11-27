/**
 * RotateTool on construction guides: the guide target, the pivot click on a
 * guide (taken for the gesture only with nothing selected; just a pivot
 * with a guide already selected — SketchUp's rule), and the copy / ×N fan /
 * retype paths through `transform_guide` and `duplicate_guide_array`,
 * against a mocked WasmScene. `isMac` is mocked to true so a bare Alt tap
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
    history_generation: vi.fn(() => state.gen),
    max_array_count: vi.fn(() => 1000),
    scene_undo: vi.fn(() => { bump(); return { free: () => { /* no-op */ } } }),
    scene_redo: vi.fn(() => { bump(); return { free: () => { /* no-op */ } } }),
  }
}

/** A tool with `selected` as the app's guide and a guide acquirer hitting
 *  `hit`; both hooks select what they name in the app, which pushes it back
 *  into the tool, as the Viewport does. */
function makeTool(opts: { selected?: bigint | null; hit?: bigint | null; nodes?: NodeRef[] } = {}) {
  const scene = makeScene()
  const preview = new THREE.Group()
  const onToast = vi.fn()
  const tool = new RotateTool(scene as never, preview, null, opts.nodes ?? [], vi.fn(), onToast)
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
  return { tool, scene, preview, onToast, app, acquire, nodeAcquire }
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
