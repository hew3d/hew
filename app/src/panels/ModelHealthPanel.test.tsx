/**
 * ModelHealthPanel — grouping, counts, search, accepted toggle, row click
 * (select + frame), rim stepping, empty state, and the overlay it reports.
 *
 * A plain object stands in for the wasm Scene (the panel imports the type
 * only), and `worldBoundsForSelection` is mocked: framing math has its own
 * tests in objectBounds.test.ts.
 */
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ModelHealthPanel } from './ModelHealthPanel'
import type { Scene as WasmScene } from '../wasm/loader'
import type { HealthOverlay, HealthScope } from '../health/healthReport'
import type { NodeRef } from './treeModel'

vi.mock('./objectBounds', () => ({
  worldBoundsForSelection: vi.fn(() => ({ min: [0, 0, 0], max: [2, 2, 2] })),
}))
import { worldBoundsForSelection } from './objectBounds'
import { taggedDispatch } from '../test/taggedDispatch'

const RIM = { role: 'outer', planar: true, closed: true, cappable: true, perimeter: 4, extent: 1, edges: 4 }

function row(over: Record<string, unknown>): Record<string, unknown> {
  return {
    object: '1',
    name: 'Roof',
    definition: null,
    definition_name: null,
    placements: 1,
    instances: [],
    accepted: false,
    family: null,
    verdict: { kind: 'cappable' },
    rims: [RIM],
    branching_vertices: 0,
    islands: 1,
    ...over,
  }
}

const COUNTS = { objects: 10, solids: 5, flat_sheets: 1, bent_sheets: 0, cappable: 2, holes: 1, mixed: 0, non_manifold: 1, seam_families: 0 }

const ROWS = [
  row({ object: '1', name: 'Roof', rims: [RIM, RIM, RIM] }),
  row({ object: '2', name: 'Wall' }),
  row({ object: '3', name: 'Floor', verdict: { kind: 'flat_sheet', holes: 0 }, rims: [RIM] }),
  row({ object: '4', name: 'Pipe', verdict: { kind: 'holes', planar: 2, warped: 0 }, rims: [RIM, RIM] }),
  row({ object: '5', name: 'Knot', verdict: { kind: 'non_manifold' }, branching_vertices: 2 }),
  row({
    object: '6',
    name: 'Leg',
    definition: '77',
    definition_name: 'Chair',
    placements: 4,
    instances: ['201', '202'],
    verdict: { kind: 'cappable' },
  }),
  row({ object: '7', name: 'Quiet', accepted: true, verdict: { kind: 'bent_sheet' } }),
]

function makeScene(rows = ROWS, extra: Record<string, unknown> = {}) {
  const scene = {
    health_report_json: vi.fn(() => JSON.stringify({ rows, families: [], counts: COUNTS })),
    health_rim_highlight: vi.fn((_id: bigint, rim: number) => new Float32Array([rim, 0, 0, rim + 1, 1, 1])),
    health_rim_segments: vi.fn(() => new Float32Array()),
    last_import_object_ids: vi.fn(() => new BigUint64Array([1n, 2n])),
    group_members: () => [],
    instance_expanded_members: vi.fn(() => new BigUint64Array([4n])),
    ...extra,
  }
  return scene as unknown as WasmScene & typeof scene
}

function setup(over: { scene?: ReturnType<typeof makeScene>; scope?: HealthScope; selectedIds?: NodeRef[]; objectFilter?: bigint | null } = {}) {
  const scene = over.scene ?? makeScene()
  const props = {
    scene,
    docRev: 0,
    selectedIds: over.selectedIds ?? [],
    scope: over.scope ?? 'document',
    onScopeChange: vi.fn(),
    objectFilter: over.objectFilter ?? null,
    onClearObjectFilter: vi.fn(),
    onSelectNode: vi.fn(),
    onIsolate: vi.fn(),
    onClearIsolation: vi.fn(),
    onFrameBounds: vi.fn(),
    onOverlayChange: vi.fn(),
    onGeometryChanged: vi.fn(),
    onToast: vi.fn(),
  }
  const utils = render(<ModelHealthPanel {...props} />)
  return { ...utils, scene, props }
}

const rowEl = (id: string) => document.querySelector(`[data-health-row="${id}"]`) as HTMLElement
/** Type into the search box and let its debounce elapse. */
async function search(value: string): Promise<void> {
  fireEvent.change(screen.getByRole('searchbox', { name: 'Search' }), { target: { value } })
  await act(async () => {
    await new Promise((r) => setTimeout(r, 200))
  })
}
const lastOverlay = (fn: ReturnType<typeof vi.fn>): HealthOverlay | null => fn.mock.calls[fn.mock.calls.length - 1][0]

beforeEach(() => {
  vi.mocked(worldBoundsForSelection).mockClear()
})

describe('ModelHealthPanel — grouping', () => {
  it('lists non-empty groups with counts, in kind order, with the plain words', () => {
    setup()
    const headers = screen.getAllByRole('button', { expanded: true }).map((b) => b.textContent)
    expect(headers).toEqual([
      '▾Has openings (4)',
      '▾Flat panels (1)',
      "▾Can't be closed (1)",
    ])
    // No curved-sheet group: the only curved sheet is kept, so it is hidden.
    expect(screen.queryByText(/Curved sheets/)).not.toBeInTheDocument()
    expect(screen.queryByText(/leaky/i)).not.toBeInTheDocument()
  })

  it('shows name, verdict detail, and ×N for a shared definition member', () => {
    setup()
    const leg = rowEl('6')
    expect(within(leg).getByText('Chair')).toBeInTheDocument() // definition name
    expect(within(leg).getByText('×4')).toBeInTheDocument()
    expect(within(leg).getByText('1 opening')).toBeInTheDocument()
    expect(within(rowEl('4')).getByText('2 openings')).toBeInTheDocument()
    expect(within(rowEl('3')).getByText('Flat panel')).toBeInTheDocument()
  })

  it('a group header collapses its rows', () => {
    setup()
    fireEvent.click(screen.getByRole('button', { name: /Flat panels \(1\)/ }))
    expect(rowEl('3')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: /Flat panels \(1\)/ }))
    expect(rowEl('3')).not.toBeNull()
  })

  it('groups split shapes by family under "Split shapes"', () => {
    const rows = [
      row({ object: '1', name: 'Group#18', family: 0 }),
      row({ object: '2', name: 'Group#18', family: 0, verdict: { kind: 'flat_sheet', holes: 0 } }),
    ]
    const scene = makeScene(rows, {
      health_report_json: vi.fn(() =>
        JSON.stringify({ rows, families: [{ members: ['1', '2', '3'], shared_rims: 1 }], counts: COUNTS }),
      ),
    })
    setup({ scene })
    expect(screen.getByText(/Split shapes \(1\)/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Group#18 → 3 pieces \(2\)/ })).toBeInTheDocument()
  })
})

describe('ModelHealthPanel — filters', () => {
  it('search matches names', async () => {
    setup()
    await search('roo')
    expect(rowEl('1')).not.toBeNull()
    expect(rowEl('2')).toBeNull()
    expect(screen.queryByText(/Can't be closed/)).not.toBeInTheDocument()
  })

  it('search matches the detail text', async () => {
    setup()
    await search('2 openings')
    expect(rowEl('4')).not.toBeNull()
    expect(rowEl('2')).toBeNull()
    expect(rowEl('5')).toBeNull()
  })

  it('says so when nothing matches', async () => {
    setup()
    await search('zzz')
    expect(screen.getByText('Nothing matches your search.')).toBeInTheDocument()
  })

  it('hides kept sheets until "Show kept panels and sheets" is checked', () => {
    setup()
    expect(rowEl('7')).toBeNull()
    fireEvent.click(screen.getByRole('checkbox', { name: 'Show kept panels and sheets' }))
    expect(rowEl('7')).not.toBeNull()
    expect(screen.getByRole('button', { name: /Curved sheets \(1\)/ })).toBeInTheDocument()
    expect(within(rowEl('7')).getByText('Curved sheet, kept')).toBeInTheDocument()
  })

  it('the scope select reports changes', () => {
    const { props } = setup()
    fireEvent.change(screen.getByRole('combobox', { name: 'Scope' }), { target: { value: 'last' } })
    expect(props.onScopeChange).toHaveBeenCalledWith('last')
  })

  it('reads the Last import scope with the last import ids', () => {
    const { scene } = setup({ scope: 'last' })
    expect(scene.last_import_object_ids).toHaveBeenCalled()
    expect(scene.health_report_json).toHaveBeenLastCalledWith(BigUint64Array.from([1n, 2n]))
  })

  it('reads the Selection scope from the selection (instances expand to their members)', () => {
    const { scene } = setup({
      scope: 'selection',
      selectedIds: [
        { kind: 'object', id: 9n },
        { kind: 'instance', id: 50n },
      ],
    })
    expect(scene.health_report_json).toHaveBeenLastCalledWith(BigUint64Array.from([9n, 4n]))
  })

  it('an empty selection says nothing is selected', () => {
    setup({ scope: 'selection' })
    expect(screen.getByText('Select something in the model first.')).toBeInTheDocument()
  })

  it('the Whole document scope asks for every object', () => {
    const { scene } = setup()
    expect(scene.health_report_json).toHaveBeenLastCalledWith(null)
  })

  it('an object filter lists one object and can be cleared', () => {
    const { scene, props } = setup({ objectFilter: 5n })
    expect(scene.health_report_json).toHaveBeenLastCalledWith(BigUint64Array.from([5n]))
    fireEvent.click(screen.getByRole('button', { name: 'Show all objects' }))
    expect(props.onClearObjectFilter).toHaveBeenCalled()
  })
})

describe('ModelHealthPanel — empty state', () => {
  it('says every object is a solid when there are no rows', () => {
    setup({ scene: makeScene([]) })
    expect(screen.getByText('Every object in the model is solid.')).toBeInTheDocument()
  })
})

describe('ModelHealthPanel — row click', () => {
  it('a world object row selects its object node and frames it', () => {
    const { props } = setup()
    fireEvent.click(rowEl('2'))
    expect(props.onSelectNode).toHaveBeenCalledWith({ kind: 'object', id: 2n })
    expect(worldBoundsForSelection).toHaveBeenCalledWith(expect.anything(), [{ kind: 'object', id: 2n }])
    expect(props.onFrameBounds).toHaveBeenCalledWith([0, 0, 0], [2, 2, 2])
  })

  it('a definition member row selects its first placing instance', () => {
    const { props } = setup()
    fireEvent.click(rowEl('6'))
    expect(props.onSelectNode).toHaveBeenCalledWith({ kind: 'instance', id: 201n })
    expect(worldBoundsForSelection).toHaveBeenCalledWith(expect.anything(), [{ kind: 'instance', id: 201n }])
  })

  it('a row isolates its world object and stepping rims keeps the isolation', () => {
    const { props } = setup()
    fireEvent.click(rowEl('1'))
    expect(props.onIsolate).toHaveBeenCalledWith([{ kind: 'object', id: 1n }], { frame: false })
    props.onIsolate.mockClear()
    fireEvent.click(screen.getByRole('button', { name: 'Next opening' }))
    expect(props.onIsolate).not.toHaveBeenCalled()
    expect(props.onClearIsolation).not.toHaveBeenCalled()
  })

  it('a definition member row isolates the member through the selected instance', () => {
    const { props } = setup()
    fireEvent.click(rowEl('6'))
    expect(props.onIsolate).toHaveBeenCalledWith([{ kind: 'member', object: 6n, instance: 201n }], { frame: false })
    // Another row replaces it.
    fireEvent.click(rowEl('2'))
    expect(props.onIsolate).toHaveBeenLastCalledWith([{ kind: 'object', id: 2n }], { frame: false })
  })

  it('switching the scope clears isolation', () => {
    const { props, rerender } = setup()
    expect(props.onClearIsolation).not.toHaveBeenCalled()
    rerender(<ModelHealthPanel {...props} scope="last" />)
    expect(props.onClearIsolation).toHaveBeenCalledTimes(1)
  })

  it('collapsing the panel (unmounting it) clears isolation', () => {
    const { props, unmount } = setup()
    unmount()
    expect(props.onClearIsolation).toHaveBeenCalledTimes(1)
  })

  it('Enter selects the focused row', () => {
    const { props } = setup()
    fireEvent.keyDown(rowEl('2'), { key: 'Enter' })
    expect(props.onSelectNode).toHaveBeenCalledWith({ kind: 'object', id: 2n })
  })
})

describe('ModelHealthPanel — stepping through rims', () => {
  it('shows ◀ ▶ and the rim position only on the selected row', () => {
    setup()
    expect(screen.queryByRole('button', { name: 'Next opening' })).not.toBeInTheDocument()
    fireEvent.click(rowEl('1'))
    expect(within(rowEl('1')).getByRole('button', { name: 'Previous opening' })).toBeInTheDocument()
    expect(within(rowEl('1')).getByText('1/3')).toBeInTheDocument()
  })

  it('▶ and ◀ step (wrapping), framing each rim from its segments', () => {
    const { scene, props } = setup()
    fireEvent.click(rowEl('1'))
    props.onFrameBounds.mockClear()
    fireEvent.click(screen.getByRole('button', { name: 'Next opening' }))
    expect(scene.health_rim_highlight).toHaveBeenLastCalledWith(1n, 1)
    expect(props.onFrameBounds).toHaveBeenLastCalledWith([1, 0, 0], [2, 1, 1])
    expect(within(rowEl('1')).getByText('2/3')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Previous opening' }))
    fireEvent.click(screen.getByRole('button', { name: 'Previous opening' }))
    expect(scene.health_rim_highlight).toHaveBeenLastCalledWith(1n, 2) // wrapped to the last rim
    expect(within(rowEl('1')).getByText('3/3')).toBeInTheDocument()
  })

  it('stepping does not change the selection', () => {
    const { props } = setup()
    fireEvent.click(rowEl('1'))
    props.onSelectNode.mockClear()
    fireEvent.click(screen.getByRole('button', { name: 'Next opening' }))
    expect(props.onSelectNode).not.toHaveBeenCalled()
  })

  it('Tab / Shift+Tab on the selected row walk its rims', () => {
    const { scene } = setup()
    fireEvent.click(rowEl('1'))
    fireEvent.keyDown(rowEl('1'), { key: 'Tab' })
    expect(scene.health_rim_highlight).toHaveBeenLastCalledWith(1n, 1)
    fireEvent.keyDown(rowEl('1'), { key: 'Tab', shiftKey: true })
    expect(scene.health_rim_highlight).toHaveBeenLastCalledWith(1n, 0)
  })

  it('Tab is left alone on a row with a single rim', () => {
    setup()
    fireEvent.click(rowEl('2'))
    const notPrevented = fireEvent.keyDown(rowEl('2'), { key: 'Tab' })
    expect(notPrevented).toBe(true)
  })
})

describe('ModelHealthPanel — overlay', () => {
  it('reports the listed rows (non-manifold apart) and the current finding', () => {
    const { props } = setup()
    const first = lastOverlay(props.onOverlayChange)!
    expect(first.ids).toEqual([1n, 2n, 4n, 6n, 3n])
    expect(first.warnIds).toEqual([5n])
    expect(first.current).toBeNull()
    fireEvent.click(rowEl('1'))
    fireEvent.click(screen.getByRole('button', { name: 'Next opening' }))
    expect(lastOverlay(props.onOverlayChange)!.current).toEqual({ id: 1n, rim: 1 })
  })

  it('follows the search and accepted filters', async () => {
    const { props } = setup()
    await search('roo')
    expect(lastOverlay(props.onOverlayChange)!.ids).toEqual([1n])
    expect(lastOverlay(props.onOverlayChange)!.warnIds).toEqual([])
    await search('')
    expect(lastOverlay(props.onOverlayChange)!.ids).not.toContain(7n)
    fireEvent.click(screen.getByRole('checkbox', { name: 'Show kept panels and sheets' }))
    expect(lastOverlay(props.onOverlayChange)!.ids).toContain(7n)
  })

  it('clears the overlay when the panel unmounts (section collapsed)', () => {
    const { props, unmount } = setup()
    unmount()
    expect(lastOverlay(props.onOverlayChange)).toBeNull()
  })
})

describe('ModelHealthPanel — Stage 2 repairs', () => {
  const capResult = (closed: boolean) => JSON.stringify({ faces: 1, closed, reversed: false })
  const repairScene = (closed = true) =>
    makeScene(ROWS, {
      cap_rims: vi.fn(() => capResult(closed)),
      set_accepted_surface: vi.fn(),
    })
  const btn = (name: string | RegExp) => screen.getByRole('button', { name })

  it('shows actions only on the selected row', () => {
    setup({ scene: repairScene() })
    expect(screen.queryByRole('button', { name: 'Close' })).not.toBeInTheDocument()
    fireEvent.click(rowEl('1'))
    expect(btn('Close')).toBeInTheDocument()
    expect(btn('Keep as is')).toBeInTheDocument()
  })

  it('Close caps every rim, refreshes, and clears the current finding', () => {
    const scene = repairScene(true)
    const { props } = setup({ scene })
    fireEvent.click(rowEl('1'))
    fireEvent.click(btn('Close'))
    expect(scene.cap_rims).toHaveBeenCalledWith(1n, null)
    expect(props.onGeometryChanged).toHaveBeenCalledTimes(1)
    expect(lastOverlay(props.onOverlayChange)!.current).toBeNull()
  })

  it('Close this opening passes the current rim only, then restarts at the first rim', () => {
    const scene = repairScene(false)
    const { props } = setup({ scene })
    fireEvent.click(rowEl('1'))
    fireEvent.click(btn('Next opening'))
    fireEvent.click(btn('Close this opening'))
    expect(scene.cap_rims).toHaveBeenCalledWith(1n, Uint32Array.of(1))
    expect(props.onGeometryChanged).toHaveBeenCalledTimes(1)
    expect(lastOverlay(props.onOverlayChange)!.current).toEqual({ id: 1n, rim: 0 })
  })

  it('offers Close this opening only for a planar closed rim', () => {
    const warped = { ...RIM, planar: false, cappable: false }
    setup({ scene: makeScene([row({ object: '1', rims: [warped] })]) as never })
    fireEvent.click(rowEl('1'))
    expect(screen.queryByRole('button', { name: 'Close this opening' })).not.toBeInTheDocument()
    expect(btn('Close')).toBeInTheDocument()
  })

  it('Keep as is and Don\'t keep call set_accepted_surface', () => {
    const scene = repairScene()
    const { props } = setup({ scene })
    fireEvent.click(rowEl('2'))
    fireEvent.click(btn('Keep as is'))
    expect(scene.set_accepted_surface).toHaveBeenCalledWith(2n, true)
    fireEvent.click(screen.getByRole('checkbox', { name: 'Show kept panels and sheets' }))
    fireEvent.click(rowEl('7'))
    expect(screen.queryByRole('button', { name: 'Close' })).not.toBeInTheDocument()
    fireEvent.click(btn("Don't keep"))
    expect(scene.set_accepted_surface).toHaveBeenCalledWith(7n, false)
    expect(props.onGeometryChanged).toHaveBeenCalledTimes(2)
  })

  it('a closer-look row, and a flat panel, offer Keep as is but no Close', () => {
    setup({ scene: repairScene() })
    fireEvent.click(rowEl('5'))
    expect(screen.queryByRole('button', { name: 'Close' })).not.toBeInTheDocument()
    expect(btn('Keep as is')).toBeInTheDocument()
    fireEvent.click(rowEl('3'))
    expect(screen.queryByRole('button', { name: 'Close' })).not.toBeInTheDocument()
    expect(btn('Keep as is')).toBeInTheDocument()
  })

  it('names the shared copies in the tooltip of a definition member', () => {
    setup({ scene: repairScene() })
    fireEvent.click(rowEl('6'))
    expect(btn('Close')).toHaveAttribute('title', expect.stringContaining('in all 4 copies'))
  })

  it('a refusal toasts the kernel copy, calls no refresh, and keeps the selection', () => {
    const scene = makeScene(ROWS, {
      cap_rims: vi.fn(() => {
        throw new Error('RimNotPlanar: rim 0 is not planar')
      }),
    })
    const { props } = setup({ scene })
    fireEvent.click(rowEl('1'))
    fireEvent.click(btn('Close'))
    expect(props.onToast).toHaveBeenCalledTimes(1)
    expect(props.onToast.mock.calls[0][0]).toMatch(/opening isn't flat/)
    expect(props.onToast.mock.calls[0][1]).toBe('RimNotPlanar')
    expect(props.onGeometryChanged).not.toHaveBeenCalled()
    expect(rowEl('1')).toHaveAttribute('aria-pressed', 'true')
  })
})

describe('ModelHealthPanel — keyboard and selection fixes', () => {
  it('Enter / Space on the ◀ ▶ buttons keep their own behavior (the row does not hijack them)', () => {
    const { props } = setup()
    fireEvent.click(rowEl('1'))
    props.onSelectNode.mockClear()
    const next = screen.getByRole('button', { name: 'Next opening' })
    expect(fireEvent.keyDown(next, { key: 'Enter' })).toBe(true) // not preventDefault-ed
    expect(fireEvent.keyDown(next, { key: ' ' })).toBe(true)
    expect(props.onSelectNode).not.toHaveBeenCalled()
    // …and Tab from a child button is not trapped either.
    expect(fireEvent.keyDown(next, { key: 'Tab' })).toBe(true)
    expect(within(rowEl('1')).getByText('1/3')).toBeInTheDocument()
  })

  it('Tab stops at the last rim and Shift+Tab at the first, so focus can leave the row', () => {
    setup()
    fireEvent.click(rowEl('1')) // 3 rims, at 1/3
    expect(fireEvent.keyDown(rowEl('1'), { key: 'Tab', shiftKey: true })).toBe(true) // first: not trapped
    expect(within(rowEl('1')).getByText('1/3')).toBeInTheDocument()
    expect(fireEvent.keyDown(rowEl('1'), { key: 'Tab' })).toBe(false) // 2/3
    expect(fireEvent.keyDown(rowEl('1'), { key: 'Tab' })).toBe(false) // 3/3
    expect(within(rowEl('1')).getByText('3/3')).toBeInTheDocument()
    expect(fireEvent.keyDown(rowEl('1'), { key: 'Tab' })).toBe(true) // last: let the browser move on
    expect(within(rowEl('1')).getByText('3/3')).toBeInTheDocument()
  })

  it('a definition member row keeps the instance the viewport already selected', () => {
    const { props } = setup({ selectedIds: [{ kind: 'instance', id: 202n }] })
    fireEvent.click(rowEl('6'))
    expect(props.onSelectNode).toHaveBeenCalledWith({ kind: 'instance', id: 202n })
    expect(worldBoundsForSelection).toHaveBeenCalledWith(expect.anything(), [{ kind: 'instance', id: 202n }])
  })

  it('…but an unrelated selection still falls back to the first placing instance', () => {
    const { props } = setup({ selectedIds: [{ kind: 'instance', id: 999n }] })
    fireEvent.click(rowEl('6'))
    expect(props.onSelectNode).toHaveBeenCalledWith({ kind: 'instance', id: 201n })
  })
})

describe('ModelHealthPanel — Stage 3 batches', () => {
  interface Sent {
    method: string
    params: { label?: string; commands?: Array<{ method: string; params: Record<string, unknown> }> }
  }

  /** `refuse(callIndex)` → refuse that transact at an index, else succeed. */
  function batchScene(
    rows: Record<string, unknown>[] = ROWS as Record<string, unknown>[],
    refuse: (call: number) => { index: number; explanation: string } | null = () => null,
    families: unknown[] = [],
  ) {
    const sent: Sent[] = []
    let transacts = 0
    const scene = makeScene(rows, {
      health_report_json: vi.fn(() => JSON.stringify({ rows, families, counts: COUNTS })),
      api_connection_open: vi.fn(() => 3),
      api_connection_close: vi.fn(),
      object_public_ids: vi.fn((ids: BigUint64Array) => Array.from(ids, (id) => `obj_${id}`)),
      api_dispatch: vi.fn(taggedDispatch((_c: number, raw: string) => {
        const req = JSON.parse(raw) as { id: number } & Sent
        sent.push(req)
        if (req.method !== 'hew.doc.transact') return JSON.stringify({ jsonrpc: '2.0', id: req.id, result: {} })
        const r = refuse(transacts++)
        return JSON.stringify(
          r === null
            ? { jsonrpc: '2.0', id: req.id, result: {} }
            : { jsonrpc: '2.0', id: req.id, error: { code: -32000, message: 'refused', data: { refusal: 'X', failed_index: r.index, explanation: r.explanation } } },
        )
      })),
    })
    const typed = scene as typeof scene & { api_dispatch: ReturnType<typeof vi.fn> }
    return { scene: typed, sent, transacts: () => sent.filter((s) => s.method === 'hew.doc.transact') }
  }
  const group = (key: string) => document.querySelector(`[data-health-group="${key}"]`) as HTMLElement
  const batchBtn = (key: string, name: string | RegExp) => within(group(key)).getByRole('button', { name })

  it('puts the right batch buttons on each group header', () => {
    setup({ scene: batchScene().scene })
    expect(batchBtn('openings', 'Close all')).toBeInTheDocument()
    expect(within(group('openings')).queryByRole('button', { name: /Keep all/ })).not.toBeInTheDocument()
    expect(batchBtn('flatPanels', 'Add thickness…')).toBeEnabled()
    expect(batchBtn('flatPanels', 'Keep all')).toBeInTheDocument()
    expect(within(group('flatPanels')).queryByRole('button', { name: 'Close all' })).not.toBeInTheDocument()
    expect(batchBtn('closerLook', 'Keep all')).toBeInTheDocument()
    expect(within(group('closerLook')).queryByRole('button', { name: 'Add thickness…' })).not.toBeInTheDocument()
  })

  it('curved sheets: Keep all, and Add thickness… disabled with the reason', () => {
    const rows = [row({ object: '9', name: 'Hill', verdict: { kind: 'bent_sheet' } })]
    setup({ scene: batchScene(rows).scene })
    const thicken = batchBtn('curvedSheets', 'Add thickness…')
    expect(thicken).toBeDisabled()
    expect(thicken).toHaveAttribute('title', "Hew can't add thickness to curved sheets yet")
    expect(batchBtn('curvedSheets', 'Keep all')).toBeEnabled()
  })

  it('Close all sends ONE labeled transaction over the listed closable rows, then refreshes and toasts', async () => {
    const { scene, transacts } = batchScene()
    const { props } = setup({ scene })
    fireEvent.click(batchBtn('openings', 'Close all'))
    await waitFor(() => expect(props.onGeometryChanged).toHaveBeenCalledTimes(1))
    expect(transacts()).toHaveLength(1)
    expect(transacts()[0].params.label).toBe('Close 4 objects')
    expect(transacts()[0].params.commands).toEqual([
      { method: 'hew.solid.cap', params: { object: 'obj_1' } },
      { method: 'hew.solid.cap', params: { object: 'obj_2' } },
      { method: 'hew.solid.cap', params: { object: 'obj_4' } },
      { method: 'hew.solid.cap', params: { object: 'obj_6' } },
    ])
    expect(props.onToast).toHaveBeenLastCalledWith('Closed 4 objects.')
  })

  it('only the rows currently listed are in the batch (search filter)', async () => {
    const { scene, transacts } = batchScene()
    const { props } = setup({ scene })
    await search('wall')
    fireEvent.click(batchBtn('openings', 'Close all'))
    await waitFor(() => expect(props.onGeometryChanged).toHaveBeenCalled())
    expect(transacts()[0].params.commands).toEqual([{ method: 'hew.solid.cap', params: { object: 'obj_2' } }])
    expect(transacts()[0].params.label).toBe('Close 1 object')
  })

  it('a refusal drops that row, warns on it, re-runs the rest, and the toast counts both', async () => {
    const { scene, transacts } = batchScene(ROWS as Record<string, unknown>[], (call) =>
      call === 0 ? { index: 1, explanation: 'its edge crosses itself' } : null,
    )
    const { props } = setup({ scene })
    fireEvent.click(batchBtn('openings', 'Close all'))
    await waitFor(() => expect(props.onGeometryChanged).toHaveBeenCalled())
    expect(transacts()).toHaveLength(2)
    expect(transacts()[1].params.label).toBe('Close 3 objects')
    expect(transacts()[1].params.commands!.map((c) => c.params.object)).toEqual(['obj_1', 'obj_4', 'obj_6'])
    expect(props.onToast).toHaveBeenLastCalledWith("Closed 3 objects. 1 couldn't be closed; see the list.")
    expect(within(rowEl('2')).getByText("Couldn't close: its edge crosses itself")).toBeInTheDocument()
    expect(within(rowEl('1')).queryByText(/Couldn't/)).not.toBeInTheDocument()
  })

  it('a row warning clears once the report no longer lists the row (and does not come back)', async () => {
    const { scene } = batchScene(ROWS as Record<string, unknown>[], (call) =>
      call === 0 ? { index: 1, explanation: 'nope' } : null,
    )
    const { props, rerender } = setup({ scene })
    fireEvent.click(batchBtn('openings', 'Close all'))
    await waitFor(() => expect(within(rowEl('2')).getByText("Couldn't close: nope")).toBeInTheDocument())
    // Row 2 repaired elsewhere: a new report without it.
    scene.health_report_json.mockImplementation(() =>
      JSON.stringify({ rows: (ROWS as Record<string, unknown>[]).filter((r) => r.object !== '2'), families: [], counts: COUNTS }),
    )
    rerender(<ModelHealthPanel {...props} scene={scene} docRev={1} />)
    expect(rowEl('2')).toBeNull()
    // It comes back (undo): no stale warning.
    scene.health_report_json.mockImplementation(() => JSON.stringify({ rows: ROWS, families: [], counts: COUNTS }))
    rerender(<ModelHealthPanel {...props} scene={scene} docRev={2} />)
    expect(within(rowEl('2')).queryByText(/Couldn't/)).not.toBeInTheDocument()
  })

  it('shows a busy state on the button while the batch runs', async () => {
    const { scene } = batchScene()
    const { props } = setup({ scene })
    fireEvent.click(batchBtn('openings', 'Close all'))
    expect(batchBtn('openings', 'Working…')).toBeDisabled()
    expect(batchBtn('closerLook', 'Keep all')).toBeDisabled()
    await waitFor(() => expect(props.onGeometryChanged).toHaveBeenCalled())
    expect(batchBtn('closerLook', 'Keep all')).toBeEnabled()
  })

  it('Keep all keeps every listed row of the group in one transaction', async () => {
    const { scene, transacts } = batchScene()
    const { props } = setup({ scene })
    fireEvent.click(batchBtn('closerLook', 'Keep all'))
    await waitFor(() => expect(props.onGeometryChanged).toHaveBeenCalled())
    expect(transacts()[0].params.label).toBe('Keep 1 object as is')
    expect(transacts()[0].params.commands).toEqual([
      { method: 'hew.solid.mark_surface', params: { object: 'obj_5', accepted: true } },
    ])
  })

  it('a thrown (non-refusal) error toasts and refreshes nothing', async () => {
    const { scene } = batchScene()
    scene.api_dispatch.mockImplementation(() => {
      throw new Error('boom')
    })
    const { props } = setup({ scene })
    fireEvent.click(batchBtn('openings', 'Close all'))
    await waitFor(() => expect(props.onToast).toHaveBeenCalledWith('boom', undefined))
    expect(props.onGeometryChanged).not.toHaveBeenCalled()
    expect(batchBtn('openings', 'Close all')).toBeEnabled()
  })

  describe('Thicken…', () => {
    const thickenRows = [
      row({ object: '3', name: 'Floor', verdict: { kind: 'flat_sheet', holes: 0 } }),
      row({ object: '8', name: 'Deck', verdict: { kind: 'flat_sheet', holes: 1 } }),
    ]

    it('the group button asks for a thickness, then runs one thicken transaction (Centered)', async () => {
      const { scene, transacts } = batchScene(thickenRows)
      const { props } = setup({ scene })
      fireEvent.click(batchBtn('flatPanels', 'Add thickness…'))
      const dialog = screen.getByRole('dialog', { name: 'Add thickness to 2 panels' })
      fireEvent.change(within(dialog).getByRole('textbox', { name: 'Thickness' }), { target: { value: '4cm' } })
      fireEvent.click(within(dialog).getByRole('radio', { name: 'Centered' }))
      fireEvent.click(within(dialog).getByRole('button', { name: 'Add thickness' }))
      await waitFor(() => expect(props.onGeometryChanged).toHaveBeenCalled())
      expect(transacts()).toHaveLength(1)
      expect(transacts()[0].params.label).toBe('Add thickness to 2 panels')
      expect(transacts()[0].params.commands).toEqual([
        { method: 'hew.solid.thicken', params: { object: 'obj_3', distance: 0.04, centered: true } },
        { method: 'hew.solid.thicken', params: { object: 'obj_8', distance: 0.04, centered: true } },
      ])
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })

    it('Escape cancels without running anything', () => {
      const { scene, sent } = batchScene(thickenRows)
      setup({ scene })
      fireEvent.click(batchBtn('flatPanels', 'Add thickness…'))
      fireEvent.keyDown(document, { key: 'Escape' })
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
      expect(sent).toHaveLength(0)
    })

    it('a selected flat-sheet row offers Thicken… for itself (Behind = negative)', async () => {
      const { scene, transacts } = batchScene(thickenRows)
      const { props } = setup({ scene })
      fireEvent.click(rowEl('3'))
      fireEvent.click(within(rowEl('3')).getByRole('button', { name: 'Add thickness…' }))
      const dialog = screen.getByRole('dialog', { name: 'Add thickness' })
      fireEvent.change(within(dialog).getByRole('textbox', { name: 'Thickness' }), { target: { value: '1cm' } })
      fireEvent.click(within(dialog).getByRole('radio', { name: 'Behind' }))
      fireEvent.click(within(dialog).getByRole('button', { name: 'Add thickness' }))
      await waitFor(() => expect(props.onGeometryChanged).toHaveBeenCalled())
      expect(transacts()[0].params.label).toBe('Add thickness to 1 panel')
      expect(transacts()[0].params.commands).toEqual([
        { method: 'hew.solid.thicken', params: { object: 'obj_3', distance: -0.01 } },
      ])
    })

    it('a refused surface is warned on its row', async () => {
      const { scene } = batchScene(thickenRows, (call) => (call === 0 ? { index: 0, explanation: 'too thin' } : null))
      const { props } = setup({ scene })
      fireEvent.click(batchBtn('flatPanels', 'Add thickness…'))
      fireEvent.click(screen.getByRole('button', { name: 'Add thickness' }))
      await waitFor(() => expect(props.onGeometryChanged).toHaveBeenCalled())
      expect(within(rowEl('3')).getByText("Couldn't add thickness: too thin")).toBeInTheDocument()
      expect(props.onToast).toHaveBeenLastCalledWith("Added thickness to 1 object. 1 couldn't be given a thickness; see the list.")
    })
  })

  describe('preview coloring', () => {
    const WARPED = { ...RIM, planar: false, cappable: false }
    const OPEN = { ...RIM, closed: false, cappable: false }
    const previewRows = [
      row({ object: '1', rims: [RIM, RIM, RIM] }),
      row({ object: '2', rims: [RIM] }),
      row({ object: '3', rims: [RIM, WARPED, RIM] }),
      row({ object: '4', rims: [OPEN] }),
      row({ object: '5', verdict: { kind: 'non_manifold' }, rims: [RIM] }),
    ]

    it('hovering Close all previews the planar closed rims of the batch; leaving clears it', () => {
      const { props } = setup({ scene: batchScene(previewRows).scene })
      expect(lastOverlay(props.onOverlayChange)!.capPreview).toBeNull()
      const button = batchBtn('openings', 'Close all')
      fireEvent.mouseEnter(button)
      expect(lastOverlay(props.onOverlayChange)!.capPreview).toEqual([
        { id: 1n, rim: 0 },
        { id: 1n, rim: 1 },
        { id: 1n, rim: 2 },
        { id: 2n, rim: 0 },
      ])
      // Only the group's rows are previewed; the rest keep the usual overlay.
      expect(lastOverlay(props.onOverlayChange)!.warnIds).toEqual([5n])
      expect(lastOverlay(props.onOverlayChange)!.ids).toEqual([1n, 2n, 3n, 4n])
      fireEvent.mouseLeave(button)
      expect(lastOverlay(props.onOverlayChange)!.capPreview).toBeNull()
    })

    it('keyboard focus previews too, and blur clears', () => {
      const { props } = setup({ scene: batchScene(previewRows).scene })
      const button = batchBtn('openings', 'Close all')
      fireEvent.focus(button)
      expect(lastOverlay(props.onOverlayChange)!.capPreview).toHaveLength(4)
      fireEvent.blur(button)
      expect(lastOverlay(props.onOverlayChange)!.capPreview).toBeNull()
    })

    it('a row Close previews its own rims; Close this opening previews just the framed one', () => {
      const { props } = setup({ scene: batchScene(previewRows).scene })
      fireEvent.click(rowEl('3'))
      const close = within(rowEl('3')).getByRole('button', { name: /^Close$/ })
      fireEvent.mouseEnter(close)
      expect(lastOverlay(props.onOverlayChange)!.capPreview).toEqual([{ id: 3n, rim: 0 }, { id: 3n, rim: 2 }])
      fireEvent.mouseLeave(close)
      fireEvent.click(within(rowEl('3')).getByRole('button', { name: 'Next opening' })) // the warped one: no Close this opening
      fireEvent.click(within(rowEl('3')).getByRole('button', { name: 'Next opening' }))
      fireEvent.mouseEnter(within(rowEl('3')).getByRole('button', { name: 'Close this opening' }))
      expect(lastOverlay(props.onOverlayChange)!.capPreview).toEqual([{ id: 3n, rim: 2 }])
    })

    it('running the batch clears the preview', async () => {
      const { props } = setup({ scene: batchScene(previewRows).scene })
      const button = batchBtn('openings', 'Close all')
      fireEvent.mouseEnter(button)
      fireEvent.click(button)
      expect(lastOverlay(props.onOverlayChange)!.capPreview).toBeNull()
      await waitFor(() => expect(props.onGeometryChanged).toHaveBeenCalled())
    })
  })

  describe('split shapes', () => {
    const famRows = [
      row({ object: '1', name: 'Group#18', family: 0 }),
      row({ object: '2', name: 'Group#18', family: 0 }),
      row({ object: '3', name: 'Group#18', family: 0, verdict: { kind: 'flat_sheet', holes: 0 } }),
    ]
    const families = [{ members: ['1', '2', '3'], shared_rims: 1 }]

    it('Rejoin is one transaction: closes the pieces and deletes the inside wall', async () => {
      const { scene, transacts } = batchScene(famRows, () => null, families)
      const { props } = setup({ scene })
      fireEvent.click(batchBtn('family-0', 'Rejoin'))
      await waitFor(() => expect(props.onGeometryChanged).toHaveBeenCalled())
      expect(transacts()).toHaveLength(1)
      expect(transacts()[0].params.label).toBe('Rejoin 1 split shape')
      expect(transacts()[0].params.commands).toEqual([
        { method: 'hew.solid.cap', params: { object: 'obj_1' } },
        { method: 'hew.solid.cap', params: { object: 'obj_2' } },
        { method: 'hew.solid.delete_faces', params: { object: 'obj_3' } },
      ])
    })

    it('"Keep the inside walls" keeps the wall instead of deleting it', async () => {
      const { scene, transacts } = batchScene(famRows, () => null, families)
      const { props } = setup({ scene })
      fireEvent.click(within(group('family-0')).getByRole('checkbox', { name: 'Keep the inside walls' }))
      fireEvent.click(batchBtn('family-0', 'Rejoin'))
      await waitFor(() => expect(props.onGeometryChanged).toHaveBeenCalled())
      expect(transacts()[0].params.commands![2]).toEqual({
        method: 'hew.solid.mark_surface',
        params: { object: 'obj_3', accepted: true },
      })
    })

    it('a refused cap warns on its row and keeps the inside wall', async () => {
      const { scene, transacts } = batchScene(famRows, (call) => (call === 0 ? { index: 0, explanation: 'bad rim' } : null), families)
      const { props } = setup({ scene })
      fireEvent.click(batchBtn('family-0', 'Rejoin'))
      await waitFor(() => expect(props.onGeometryChanged).toHaveBeenCalled())
      expect(transacts()).toHaveLength(2)
      expect(transacts()[1].params.commands).toEqual([{ method: 'hew.solid.cap', params: { object: 'obj_2' } }])
      expect(within(rowEl('1')).getByText("Couldn't close: bad rim")).toBeInTheDocument()
      expect(within(rowEl('3')).getByText(/^Couldn't close: Left as is, because another piece/)).toBeInTheDocument()
    })
  })
})

describe('ModelHealthPanel — search debounce and overlay granularity', () => {
  it('the search box changes the listed set only after the debounce', async () => {
    const { props } = setup()
    const before = props.onOverlayChange.mock.calls.length
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search' }), { target: { value: 'r' } })
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search' }), { target: { value: 'ro' } })
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search' }), { target: { value: 'roo' } })
    // Still the full list straight after the keystrokes; the box shows what was typed.
    expect(rowEl('2')).not.toBeNull()
    expect(screen.getByRole('searchbox', { name: 'Search' })).toHaveValue('roo')
    expect(props.onOverlayChange.mock.calls.length).toBe(before)
    await waitFor(() => expect(rowEl('2')).toBeNull())
    expect(lastOverlay(props.onOverlayChange)!.ids).toEqual([1n])
  })

  it('stepping a rim keeps the listed ids and revision, changing only the current finding', () => {
    const { props } = setup()
    const first = lastOverlay(props.onOverlayChange)!
    fireEvent.click(rowEl('1'))
    fireEvent.click(screen.getByRole('button', { name: 'Next opening' }))
    const stepped = lastOverlay(props.onOverlayChange)!
    expect(stepped.ids).toBe(first.ids)
    expect(stepped.warnIds).toBe(first.warnIds)
    expect(stepped.rev).toBe(first.rev)
    expect(stepped.current).toEqual({ id: 1n, rim: 1 })
  })
})

describe('ModelHealthPanel — Selection scope snapshot', () => {
  it('a row click changes the selection but not the listed objects', () => {
    const { scene, props, rerender } = setup({ scope: 'selection', selectedIds: [{ kind: 'object', id: 1n }, { kind: 'object', id: 2n }] })
    const calls = scene.health_report_json.mock.calls.length
    expect(scene.health_report_json).toHaveBeenLastCalledWith(BigUint64Array.from([1n, 2n]))
    fireEvent.click(rowEl('1'))
    // The app now reports the clicked object as the (only) selection.
    rerender(<ModelHealthPanel {...props} scope="selection" selectedIds={[{ kind: 'object', id: 1n }]} />)
    expect(scene.health_report_json.mock.calls.length).toBe(calls)
    expect(rowEl('2')).not.toBeNull()
    expect(rowEl('4')).not.toBeNull()
  })

  it('"Use current selection" re-reads the selection', () => {
    const { scene, props, rerender } = setup({ scope: 'selection', selectedIds: [{ kind: 'object', id: 1n }] })
    rerender(<ModelHealthPanel {...props} scope="selection" selectedIds={[{ kind: 'object', id: 7n }]} />)
    expect(scene.health_report_json).toHaveBeenLastCalledWith(BigUint64Array.from([1n]))
    fireEvent.click(screen.getByRole('button', { name: 'Use current selection' }))
    expect(scene.health_report_json).toHaveBeenLastCalledWith(BigUint64Array.from([7n]))
  })

  it('choosing the scope snapshots the selection at that moment', () => {
    const { scene, props, rerender } = setup({ scope: 'document', selectedIds: [{ kind: 'object', id: 3n }] })
    expect(screen.queryByRole('button', { name: 'Use current selection' })).not.toBeInTheDocument()
    rerender(<ModelHealthPanel {...props} scope="selection" selectedIds={[{ kind: 'object', id: 3n }]} />)
    expect(scene.health_report_json).toHaveBeenLastCalledWith(BigUint64Array.from([3n]))
    rerender(<ModelHealthPanel {...props} scope="selection" selectedIds={[{ kind: 'object', id: 4n }]} />)
    expect(scene.health_report_json).toHaveBeenLastCalledWith(BigUint64Array.from([3n]))
  })
})

describe('ModelHealthPanel — rim.cappable decides closability', () => {
  const FLAT_LIP = { ...RIM, planar: true, closed: true, cappable: false } // a basin's lip
  const OFFSET_OK = { ...RIM, planar: false, closed: true, cappable: true }
  const btn = (name: string) => screen.queryByRole('button', { name })

  it('Close this opening follows cappable, not planar && closed', () => {
    setup({ scene: makeScene([row({ object: '1', rims: [FLAT_LIP, OFFSET_OK] })]) as never })
    fireEvent.click(rowEl('1'))
    expect(btn('Close this opening')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Next opening' }))
    expect(btn('Close this opening')).toBeInTheDocument()
  })

  it('the Close preview lists only cappable rims', () => {
    const { props } = setup({ scene: makeScene([row({ object: '1', rims: [FLAT_LIP, OFFSET_OK, RIM] })]) as never })
    fireEvent.click(rowEl('1'))
    fireEvent.mouseEnter(screen.getByRole('button', { name: 'Close' }))
    expect(lastOverlay(props.onOverlayChange)!.capPreview).toEqual([
      { id: 1n, rim: 1 },
      { id: 1n, rim: 2 },
    ])
  })
})

describe('ModelHealthPanel — repeated names and families that are not split shapes', () => {
  it('tells rows with the same imported name apart', () => {
    setup({ scene: makeScene([row({ object: '1', name: 'Group#9' }), row({ object: '2', name: 'Group#9' }), row({ object: '3', name: 'Group#9' })]) })
    expect(within(rowEl('1')).getByText('Group#9')).toBeInTheDocument()
    expect(within(rowEl('2')).getByText('Group#9 (2)')).toBeInTheDocument()
    expect(within(rowEl('3')).getByText('Group#9 (3)')).toBeInTheDocument()
  })

  it('a family with a piece that cannot be closed is not a split shape and offers no Rejoin', () => {
    const warped = { ...RIM, planar: false, cappable: false }
    const rows = [
      row({ object: '1', name: 'G', family: 0, verdict: { kind: 'holes', planar: 0, warped: 1 }, rims: [warped] }),
      row({ object: '2', name: 'G', family: 0, verdict: { kind: 'flat_sheet', holes: 0 }, rims: [warped] }),
    ]
    const scene = makeScene(rows)
    scene.health_report_json.mockImplementation(() => JSON.stringify({ rows, families: [{ members: ['1', '2'], shared_rims: 1 }], counts: COUNTS }))
    setup({ scene })
    expect(screen.queryByText(/Split shapes/)).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Flat panels \(1\)/ })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Can't be closed \(1\)/ })).toBeInTheDocument()
  })
})

describe("ModelHealthPanel — reasons for items that can't be closed", () => {
  it('the row shows the short reason, and the selected row also shows the sentence and why it is usually fine', () => {
    setup()
    // Knot: branching open edges.
    expect(within(rowEl('5')).getByText('Open edges branch')).toBeInTheDocument()
    expect(rowEl('5').querySelector('[data-health-reason]')).toBeNull()
    fireEvent.click(rowEl('5'))
    expect(rowEl('5').querySelector('[data-health-reason]')).toHaveTextContent(
      "The highlighted edges are open. They branch or stop short, so Hew can't tell where each opening ends. Shapes like this are usually open on purpose, like a pipe fitting or an electrical box, and work as they are.",
    )
    // A row that can be closed has no such line.
    fireEvent.click(rowEl('2'))
    expect(rowEl('2').querySelector('[data-health-reason]')).toBeNull()
    expect(rowEl('5').querySelector('[data-health-reason]')).toBeNull()
  })

  it('the lip reason reads as a lip', () => {
    const lip = { ...RIM, cappable: false, sheet_edge: true }
    setup({ scene: makeScene([row({ object: '9', name: 'Tray', verdict: { kind: 'holes', planar: 1, warped: 0 }, rims: [lip] })]) })
    expect(within(rowEl('9')).getByText('Open edges run along a lip')).toBeInTheDocument()
    fireEvent.click(rowEl('9'))
    expect(rowEl('9').querySelector('[data-health-reason]')).toHaveTextContent('They run along a flat lip')
  })
})
