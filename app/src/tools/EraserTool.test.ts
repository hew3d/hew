/**
 * EraserTool unit tests: a click erases a drawn line, a solid edge only
 * explains itself until the crease modifier is held, Ctrl-click toggles a
 * crease, a soft seam refuses, and a drag sweeps several in one undo step.
 * A fake scene and host stand in for wasm and the viewport.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { EraserTool, CLICK_SLOP_PX, type EraserHost } from './EraserTool'
import type { Snap } from './types'
import type { NodeRef } from '../panels/treeModel'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

const edgeSnap = (object: bigint, element: bigint, instance?: bigint): Snap => ({
  x: 0, y: 0, z: 0, kind: 'edge', object, element, elementKind: 'edge', instance,
})
const drawn = (key: string): NodeRef => ({ kind: 'draft-edge', id: 1n, ctx: '{"plane":[0,0,0,0,0,1]}', key })

interface EdgeInfo { smooth: number; group: bigint[]; ring?: bigint[]; creased?: boolean }

function makeTool(edges: Record<string, EdgeInfo> = {}, opts: { setCreaseThrows?: string } = {}) {
  const creased = new Map<string, boolean>()
  const setCreaseCalls: unknown[][] = []
  const scene = {
    edge_smoothness: (o: bigint, e: bigint) => edges[`${o}:${e}`]?.smooth ?? 0,
    logical_edge: (o: bigint, e: bigint, propagate: boolean) => {
      const info = edges[`${o}:${e}`]
      const ids = (propagate ? info?.ring : undefined) ?? info?.group ?? []
      return { edges: () => new BigUint64Array(ids), free: vi.fn() }
    },
    edge_is_creased: (o: bigint, e: bigint) => creased.get(`${o}:${e}`) ?? edges[`${o}:${e}`]?.creased ?? false,
    edge_endpoints: () => new Float64Array([0, 0, 0, 1, 0, 0]),
    plane_units_json: () => '{"units":[],"regions":[]}',
    set_crease: vi.fn((o: bigint, ids: BigUint64Array, on: boolean) => {
      setCreaseCalls.push([o, Array.from(ids), on])
      if (opts.setCreaseThrows !== undefined) throw new Error(opts.setCreaseThrows)
      for (const id of ids) creased.set(`${o}:${id}`, on)
      return {
        objects_touched: () => new BigUint64Array([o]),
        components_touched: () => new BigUint64Array(),
        instances_touched: () => new BigUint64Array(),
        free: vi.fn(),
      }
    }),
    history_bracket_open: vi.fn(),
    history_bracket_commit: vi.fn(() => true),
    history_bracket_abort: vi.fn(),
  }
  let ref: NodeRef | null = null
  const host: EraserHost = {
    resolveRef: vi.fn(() => ref),
    runDelete: vi.fn(),
    onCreaseCommit: vi.fn(),
  }
  const onToast = vi.fn()
  const tool = new EraserTool(scene as unknown as WasmScene, new THREE.Group(), host, onToast)
  return {
    tool, scene, host, onToast, setCreaseCalls,
    setRef: (r: NodeRef | null) => { ref = r },
  }
}

/** A click: press at (x, y), release at (x + dx, y). */
function click(tool: EraserTool, snap: Snap | null, opts: { ctrl?: boolean; dx?: number } = {}) {
  tool.setCreaseModifier(opts.ctrl === true)
  tool.onPointerMove(snap, RAY)
  tool.onPointerRawDown(100, 100)
  tool.onPointerDown(snap, RAY)
  if (opts.dx !== undefined) tool.onPointerScreenMove(100 + opts.dx, 100)
  tool.onPointerUp(snap, RAY)
}

describe('EraserTool', () => {
  it('erases a drawn line with a plain click', () => {
    const t = makeTool()
    const ref = drawn('a')
    t.setRef(ref)
    click(t.tool, null)
    expect(t.host.runDelete).toHaveBeenCalledWith([ref])
  })

  it('a solid edge only explains itself on a plain click', () => {
    const t = makeTool({ '7:5': { smooth: 0, group: [5n, 6n] } })
    click(t.tool, edgeSnap(7n, 5n))
    expect(t.host.runDelete).not.toHaveBeenCalled()
    expect(t.scene.set_crease).not.toHaveBeenCalled()
    expect(t.tool.statusHint()).toContain("Edges of a solid can't be erased.")
    expect(t.tool.statusHint()).toContain('to crease (keeps it sharp under Smooth)')
  })

  it('Ctrl-click on one facet of a faceted round creases the whole base ring', () => {
    const t = makeTool({ '7:5': { smooth: 0, group: [5n], ring: [5n, 6n, 7n, 8n] } })
    click(t.tool, edgeSnap(7n, 5n), { ctrl: true })
    expect(t.setCreaseCalls[0]).toEqual([7n, [5n, 6n, 7n, 8n], true])
  })

  it('Ctrl-click creases the whole logical edge, and again lets it soften', () => {
    const t = makeTool({ '7:5': { smooth: 0, group: [5n, 6n] } })
    click(t.tool, edgeSnap(7n, 5n), { ctrl: true })
    expect(t.setCreaseCalls[0]).toEqual([7n, [5n, 6n], true])
    expect(t.host.onCreaseCommit).toHaveBeenCalledWith({ objectIds: [7n], componentIds: [], instanceIds: [] })
    expect(t.onToast).toHaveBeenLastCalledWith('Creased the edge. It stays sharp under Smooth.')
    click(t.tool, edgeSnap(7n, 5n), { ctrl: true })
    expect(t.setCreaseCalls[1]).toEqual([7n, [5n, 6n], false])
    expect(t.onToast).toHaveBeenLastCalledWith('Uncreased the edge. It softens under Smooth again.')
  })

  it('shows the crease status while the modifier is held', () => {
    const t = makeTool({ '7:5': { smooth: 0, group: [5n] }, '7:9': { smooth: 0, group: [9n], creased: true } })
    t.tool.setCreaseModifier(true)
    t.tool.onPointerMove(edgeSnap(7n, 5n), RAY)
    expect(t.tool.statusHint()).toBe('Crease: keeps this edge sharp under Smooth')
    t.tool.onPointerMove(edgeSnap(7n, 9n), RAY)
    expect(t.tool.statusHint()).toBe('Uncrease: lets this edge soften under Smooth')
  })

  it('a tangent edge is creased on its own', () => {
    const t = makeTool({ '7:5': { smooth: 1, group: [] } })
    click(t.tool, edgeSnap(7n, 5n), { ctrl: true })
    expect(t.setCreaseCalls[0]).toEqual([7n, [5n], true])
  })

  it('a soft seam says so, and Ctrl-click shows the kernel refusal', () => {
    const t = makeTool({ '7:5': { smooth: 2, group: [] } }, { setCreaseThrows: 'CreaseOnSoftEdge: seam' })
    click(t.tool, edgeSnap(7n, 5n))
    expect(t.tool.statusHint()).toBe('That edge is a soft seam.')
    expect(t.scene.set_crease).not.toHaveBeenCalled()
    click(t.tool, edgeSnap(7n, 5n), { ctrl: true })
    expect(t.onToast).toHaveBeenCalledWith("Soft seams can't be creased.", 'CreaseOnSoftEdge')
    expect(t.host.onCreaseCommit).not.toHaveBeenCalled()
  })

  it('a hard edge that bends nowhere is a drawn line on a face', () => {
    const t = makeTool({ '7:5': { smooth: 0, group: [] } })
    const ref = drawn('imprint')
    t.setRef(ref)
    click(t.tool, edgeSnap(7n, 5n))
    expect(t.host.runDelete).toHaveBeenCalledWith([ref])
  })

  it('does not erase the inside of a shape', () => {
    const t = makeTool()
    t.setRef({ kind: 'draft-region', id: 2n, ctx: 'c', key: 'r' })
    click(t.tool, null)
    expect(t.host.runDelete).not.toHaveBeenCalled()
    expect(t.tool.statusHint()).toContain('Click a line to erase it')
  })

  it('an edge of a component copy points to editing the component', () => {
    const t = makeTool({ '7:5': { smooth: 0, group: [5n] } })
    click(t.tool, edgeSnap(7n, 5n, 9n), { ctrl: true })
    expect(t.scene.set_crease).not.toHaveBeenCalled()
    expect(t.tool.statusHint()).toContain('Open the component first')
  })

  it('a small wobble is still a click', () => {
    const t = makeTool()
    t.setRef(drawn('a'))
    click(t.tool, null, { dx: CLICK_SLOP_PX })
    expect(t.host.runDelete).toHaveBeenCalledOnce()
  })

  it('dragging across lines erases them all as one undo step', () => {
    const t = makeTool()
    const a = drawn('a')
    const b = drawn('b')
    const c = drawn('c')
    t.tool.setCreaseModifier(false)
    t.setRef(a)
    t.tool.onPointerMove(null, RAY)
    t.tool.onPointerRawDown(100, 100)
    t.tool.onPointerDown(null, RAY)
    t.tool.onPointerScreenMove(130, 100) // past the slop: a drag
    t.setRef(b)
    t.tool.onPointerMove(null, RAY)
    t.setRef(null)
    t.tool.onPointerMove(null, RAY) // empty space is skipped
    t.setRef(c)
    t.tool.onPointerMove(null, RAY)
    t.setRef(a)
    t.tool.onPointerMove(null, RAY) // already collected
    t.tool.onPointerUp(null, RAY)
    expect(t.host.runDelete).toHaveBeenCalledOnce()
    expect(t.host.runDelete).toHaveBeenCalledWith([a, b, c])
    expect(t.scene.history_bracket_open).toHaveBeenCalledOnce()
    expect(t.scene.history_bracket_commit).toHaveBeenCalledWith('Erase')
  })

  it('a plain drag skips solid edges; a Ctrl drag skips drawn lines', () => {
    const t = makeTool({ '7:5': { smooth: 0, group: [5n] }, '7:8': { smooth: 0, group: [8n], creased: true } })
    const line = drawn('a')
    // Plain drag over a solid edge and a line: only the line goes.
    t.tool.setCreaseModifier(false)
    t.setRef(line)
    t.tool.onPointerMove(null, RAY)
    t.tool.onPointerRawDown(0, 0)
    t.tool.onPointerDown(null, RAY)
    t.tool.onPointerScreenMove(40, 0)
    t.tool.onPointerMove(edgeSnap(7n, 5n), RAY)
    t.tool.onPointerUp(null, RAY)
    expect(t.host.runDelete).toHaveBeenCalledWith([line])
    expect(t.scene.set_crease).not.toHaveBeenCalled()

    // Ctrl drag over two solid edges and a line: each edge toggles from its own flag.
    t.tool.setCreaseModifier(true)
    t.tool.onPointerMove(edgeSnap(7n, 5n), RAY)
    t.tool.onPointerRawDown(0, 0)
    t.tool.onPointerDown(edgeSnap(7n, 5n), RAY)
    t.tool.onPointerScreenMove(40, 0)
    t.tool.onPointerMove(edgeSnap(7n, 8n), RAY)
    t.setRef(line)
    t.tool.onPointerMove(null, RAY)
    t.tool.onPointerUp(null, RAY)
    expect(t.setCreaseCalls).toEqual([[7n, [5n], true], [7n, [8n], false]])
    expect(t.scene.history_bracket_commit).toHaveBeenLastCalledWith('Crease')
  })

  it('Escape during a drag changes nothing', () => {
    const t = makeTool()
    t.setRef(drawn('a'))
    t.tool.onPointerMove(null, RAY)
    t.tool.onPointerRawDown(0, 0)
    t.tool.onPointerDown(null, RAY)
    t.tool.onPointerScreenMove(40, 0)
    t.tool.onKey({ key: 'Escape' } as KeyboardEvent)
    t.tool.onPointerUp(null, RAY)
    expect(t.host.runDelete).not.toHaveBeenCalled()
  })

  it('aborts the bracket when a crease in a drag is refused', () => {
    const t = makeTool(
      { '7:5': { smooth: 0, group: [5n] }, '7:8': { smooth: 0, group: [8n] } },
      { setCreaseThrows: 'CreaseOnBoundary: opening' },
    )
    t.tool.setCreaseModifier(true)
    t.tool.onPointerMove(edgeSnap(7n, 5n), RAY)
    t.tool.onPointerRawDown(0, 0)
    t.tool.onPointerDown(edgeSnap(7n, 5n), RAY)
    t.tool.onPointerScreenMove(40, 0)
    t.tool.onPointerMove(edgeSnap(7n, 8n), RAY)
    t.tool.onPointerUp(null, RAY)
    expect(t.scene.history_bracket_abort).toHaveBeenCalledOnce()
    expect(t.onToast).toHaveBeenCalledWith("An edge on an opening can't be creased.", 'CreaseOnBoundary')
  })
  it('a soft seam inside a Ctrl drag creases the rest and toasts once', () => {
    const t = makeTool({
      '7:5': { smooth: 0, group: [5n] },
      '7:6': { smooth: 2, group: [] },
      '7:8': { smooth: 0, group: [8n] },
    })
    t.tool.setCreaseModifier(true)
    t.tool.onPointerMove(edgeSnap(7n, 5n), RAY)
    t.tool.onPointerRawDown(0, 0)
    t.tool.onPointerDown(edgeSnap(7n, 5n), RAY)
    t.tool.onPointerScreenMove(40, 0)
    t.tool.onPointerMove(edgeSnap(7n, 6n), RAY) // the seam
    t.tool.onPointerMove(edgeSnap(7n, 8n), RAY)
    t.tool.onPointerUp(null, RAY)
    expect(t.setCreaseCalls).toEqual([[7n, [5n], true], [7n, [8n], true]])
    expect(t.scene.history_bracket_abort).not.toHaveBeenCalled()
    expect(t.scene.history_bracket_commit).toHaveBeenLastCalledWith('Crease')
    // One seam warning, then the one confirmation for the rest.
    expect(t.onToast).toHaveBeenCalledTimes(2)
    expect(t.onToast).toHaveBeenNthCalledWith(1, "Soft seams can't be creased.", 'CreaseOnSoftEdge')
    expect(t.onToast).toHaveBeenLastCalledWith('Creased 2 edges. They stay sharp under Smooth.')
  })

  it('a Ctrl drag over a creased and an uncreased edge says both', () => {
    const t = makeTool({
      '7:5': { smooth: 0, group: [5n], creased: true },
      '7:8': { smooth: 0, group: [8n] },
    })
    t.tool.setCreaseModifier(true)
    t.tool.onPointerMove(edgeSnap(7n, 5n), RAY)
    t.tool.onPointerRawDown(0, 0)
    t.tool.onPointerDown(edgeSnap(7n, 5n), RAY)
    t.tool.onPointerScreenMove(40, 0)
    t.tool.onPointerMove(edgeSnap(7n, 8n), RAY)
    t.tool.onPointerUp(null, RAY)
    expect(t.setCreaseCalls).toEqual([[7n, [5n], false], [7n, [8n], true]])
    expect(t.onToast).toHaveBeenLastCalledWith('Creased one edge and uncreased one.')
  })

  it('a component member is refused in a Ctrl drag too: nothing is creased', () => {
    const t = makeTool({ '7:5': { smooth: 0, group: [5n] } })
    t.tool.setCreaseModifier(true)
    t.tool.onPointerMove(edgeSnap(7n, 5n, 9n), RAY)
    t.tool.onPointerRawDown(0, 0)
    t.tool.onPointerDown(edgeSnap(7n, 5n, 9n), RAY)
    t.tool.onPointerScreenMove(40, 0)
    t.tool.onPointerMove(edgeSnap(7n, 5n, 9n), RAY)
    t.tool.onPointerUp(null, RAY)
    expect(t.scene.set_crease).not.toHaveBeenCalled()
    expect(t.host.onCreaseCommit).not.toHaveBeenCalled()
  })
})
