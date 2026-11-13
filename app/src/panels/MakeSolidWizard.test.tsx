/**
 * MakeSolidWizard — the overview per scope, the recommended markers, Use
 * recommended choices (one transaction, or none), step by step with
 * non-default choices, choose-for-each ticks, the open-edge overlay and the
 * reason for an item that can't be closed, Ready sentences, the Done page,
 * Escape, resizing, isolation, and the long-list chunking. A plain object
 * stands in for the wasm Scene.
 */
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { MakeSolidWizard, LIST_CHUNK, WIZARD_DEFAULT_WIDTH, clampWizardWidth, resetWizardWidth } from './MakeSolidWizard'
import type { Scene as WasmScene } from '../wasm/loader'
import { taggedDispatch } from '../test/taggedDispatch'

const RIM = { role: 'outer', planar: true, closed: true, cappable: true, sheet_edge: false, shared: false, perimeter: 4, extent: 1, edges: 4 }
const SEAM = { ...RIM, shared: true }
const WARPED = { ...RIM, planar: false, cappable: false }

function row(over: Record<string, unknown>): Record<string, unknown> {
  return {
    object: '1',
    name: 'Thing',
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

const COUNTS = { objects: 13, solids: 6, flat_sheets: 2, bent_sheets: 1, cappable: 3, holes: 0, mixed: 0, non_manifold: 1, seam_families: 0 }

const ROWS = [
  row({ object: '1', name: 'Wall' }),
  row({ object: '2', name: 'Roof' }),
  row({ object: '3', name: 'Chair leg', definition: '9', definition_name: 'Chair', placements: 4, instances: ['201'] }),
  row({ object: '4', name: 'Glass', verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] }),
  row({ object: '5', name: 'Sign', verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] }),
  row({ object: '6', name: 'Hill', verdict: { kind: 'bent_sheet' }, rims: [WARPED] }),
  row({ object: '7', name: 'Knot', verdict: { kind: 'non_manifold' }, branching_vertices: 2 }),
]

interface Sent {
  method: string
  params: { label?: string; commands?: Array<{ method: string; params: Record<string, unknown> }> }
}

function makeScene(
  rows: Record<string, unknown>[] = ROWS,
  opts: { refuse?: (call: number) => { index: number; explanation: string; refusal?: string } | null; solidsAfter?: number; families?: unknown[] } = {},
) {
  const sent: Sent[] = []
  let transacts = 0
  let applied = false
  const scene = {
    health_report_json: vi.fn(() =>
      JSON.stringify({
        rows,
        families: opts.families ?? [],
        counts: { ...COUNTS, solids: applied ? (opts.solidsAfter ?? 9) : COUNTS.solids },
      }),
    ),
    last_import_object_ids: vi.fn(() => new BigUint64Array([1n, 2n])),
    api_connection_open: vi.fn(() => 3),
    api_connection_close: vi.fn(),
    object_public_ids: vi.fn((ids: BigUint64Array) => Array.from(ids, (id) => `obj_${id}`)),
    api_dispatch: vi.fn(taggedDispatch((_c: number, raw: string) => {
      const req = JSON.parse(raw) as { id: number } & Sent
      sent.push(req)
      if (req.method !== 'hew.doc.transact') return JSON.stringify({ jsonrpc: '2.0', id: req.id, result: {} })
      const r = opts.refuse?.(transacts++) ?? null
      if (r === null) {
        applied = true
        return JSON.stringify({ jsonrpc: '2.0', id: req.id, result: {} })
      }
      return JSON.stringify({
        jsonrpc: '2.0',
        id: req.id,
        error: { code: -32000, message: 'refused', data: { refusal: r.refusal ?? 'X', failed_index: r.index, explanation: r.explanation } },
      })
    })),
  }
  return { scene: scene as unknown as WasmScene & typeof scene, sent, transacts: () => sent.filter((s) => s.method === 'hew.doc.transact') }
}

function setup(over: { scene?: ReturnType<typeof makeScene>['scene']; scope?: 'import' | 'selection' | 'export' | 'model'; objectIds?: bigint[] } = {}) {
  const made = over.scene === undefined ? makeScene() : null
  const scene = over.scene ?? made!.scene
  const props = {
    scene,
    scope: over.scope ?? ('model' as const),
    objectIds: over.objectIds,
    onClose: vi.fn(),
    onIsolate: vi.fn(),
    onClearIsolation: vi.fn(),
    onHealthOverlay: vi.fn(),
    onGeometryChanged: vi.fn(),
    onToast: vi.fn(),
  }
  const utils = render(<MakeSolidWizard {...props} />)
  return { ...utils, props, scene, made }
}

const btn = (name: string | RegExp) => screen.getByRole('button', { name })
const page = () => document.querySelector('[data-make-solid-wizard]')!.getAttribute('data-wizard-page')

beforeEach(() => resetWizardWidth())

const overlayCalls = (props: { onHealthOverlay: ReturnType<typeof vi.fn> }) => props.onHealthOverlay.mock.calls.map((c) => c[0])
const lastOverlay = (props: { onHealthOverlay: ReturnType<typeof vi.fn> }) => overlayCalls(props).at(-1)

describe('MakeSolidWizard — overview', () => {
  it('a model: says what it checked and what Hew recommends for each page', () => {
    setup()
    expect(screen.getByRole('dialog', { name: 'Make Solid' })).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'Make this model solid' })).toBeInTheDocument()
    expect(
      screen.getByText(
        'Hew checked the 13 objects in this model. 6 are solid. Open shapes work for most things; a solid can also be pushed, pulled, combined, and 3D printed.',
      ),
    ).toBeInTheDocument()
    expect(screen.getByText('3 shapes have openings: leave them open.')).toBeInTheDocument()
    expect(screen.getByText('2 flat panels: leave as they are.')).toBeInTheDocument()
    expect(screen.getByText("1 curved sheet: Hew can't make it solid yet.")).toBeInTheDocument()
    expect(screen.getByText("1 shape can't be closed with flat faces.")).toBeInTheDocument()
    expect(screen.queryByText(/split into pieces/)).not.toBeInTheDocument() // no split shapes: no line
  })

  it('an import: says "this import" and reads the last import', () => {
    const { scene } = setup({ scope: 'import' })
    expect(scene.last_import_object_ids).toHaveBeenCalled()
    expect(scene.health_report_json).toHaveBeenCalledWith(BigUint64Array.from([1n, 2n]))
    expect(screen.getByRole('heading', { name: 'Make this import solid' })).toBeInTheDocument()
    expect(screen.getByText(/in this import\./)).toBeInTheDocument()
    expect(screen.getByText('3 shapes have openings: leave them open.')).toBeInTheDocument()
  })

  it('a selection: reads the selected objects and recommends closing the openings', () => {
    const { scene } = setup({ scope: 'selection', objectIds: [4n, 5n] })
    expect(scene.health_report_json).toHaveBeenCalledWith(BigUint64Array.from([4n, 5n]))
    expect(scene.last_import_object_ids).not.toHaveBeenCalled()
    expect(screen.getByRole('heading', { name: 'Make the selection solid' })).toBeInTheDocument()
    expect(screen.getByText(/you selected\./)).toBeInTheDocument()
    expect(screen.getByText('3 shapes have openings: close them.')).toBeInTheDocument()
  })

  it("an export warning's objects: reads them, recommends closing, and never says the person selected them (KB-6)", () => {
    const { scene } = setup({ scope: 'export', objectIds: [4n, 5n] })
    expect(scene.health_report_json).toHaveBeenCalledWith(BigUint64Array.from([4n, 5n]))
    expect(screen.getByRole('heading', { name: 'Make these objects solid' })).toBeInTheDocument()
    expect(screen.getByText(/objects that are not solid\./)).toBeInTheDocument()
    expect(screen.queryByText(/you selected/)).not.toBeInTheDocument()
    expect(screen.getByText('3 shapes have openings: close them.')).toBeInTheDocument()
  })

  it('the model scope reads every object', () => {
    const { scene } = setup()
    expect(scene.health_report_json).toHaveBeenCalledWith(null)
  })

  it('a model with nothing on any page says so and offers Done', () => {
    const { props } = setup({ scene: makeScene([]).scene })
    expect(screen.getByText('Every object here is solid.')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Use recommended choices' })).not.toBeInTheDocument()
    fireEvent.click(btn('Done'))
    expect(props.onClose).toHaveBeenCalled()
  })
})

describe('MakeSolidWizard — Use recommended choices', () => {
  it('on a model with only openings and panels: nothing runs and the Done page says nothing changed', async () => {
    const rows = ROWS.slice(0, 5)
    const made = makeScene(rows)
    const { props } = setup({ scene: made.scene })
    fireEvent.click(btn('Use recommended choices'))
    await waitFor(() => expect(page()).toBe('done'))
    expect(made.transacts()).toHaveLength(0)
    expect(props.onGeometryChanged).not.toHaveBeenCalled()
    expect(screen.getByRole('heading', { name: 'Nothing changed' })).toBeInTheDocument()
    expect(screen.getByText('Nothing was changed, so there is nothing to undo.')).toBeInTheDocument()
    expect(screen.getByText('5 objects were left as you chose.')).toBeInTheDocument()
  })

  it('on a selection: ONE transaction labeled Make Solid closes the openings', async () => {
    const made = makeScene()
    const { props } = setup({ scene: made.scene, scope: 'selection', objectIds: [1n, 2n, 3n] })
    fireEvent.click(btn('Use recommended choices'))
    await waitFor(() => expect(page()).toBe('done'))
    expect(made.transacts()).toHaveLength(1)
    expect(made.transacts()[0].params.label).toBe('Make Solid')
    expect(made.transacts()[0].params.commands).toEqual([
      { method: 'hew.solid.cap', params: { object: 'obj_1' } },
      { method: 'hew.solid.cap', params: { object: 'obj_2' } },
      { method: 'hew.solid.cap', params: { object: 'obj_3' } },
    ])
    expect(props.onGeometryChanged).toHaveBeenCalledTimes(1)
    expect(screen.getByRole('heading', { name: 'Done' })).toBeInTheDocument()
    expect(screen.getByText('3 objects are solid now. To undo all of it, choose Edit ▸ Undo.')).toBeInTheDocument()
    expect(screen.getByText('2 objects were left as you chose.')).toBeInTheDocument()
    // The item that can't be closed is listed with its short reason.
    expect(screen.getByText("1 shape can't be closed:")).toBeInTheDocument()
    const still = document.querySelector('[data-wizard-list="still-open"]') as HTMLElement
    expect(within(still).getByText('Knot')).toBeInTheDocument()
    expect(within(still).getByText('Openings tangled together')).toBeInTheDocument()
  })

  it('Done closes the window', async () => {
    const { props } = setup({ scope: 'selection', objectIds: [1n] })
    fireEvent.click(btn('Use recommended choices'))
    await waitFor(() => expect(page()).toBe('done'))
    fireEvent.click(btn('Done'))
    expect(props.onClose).toHaveBeenCalled()
  })

  it('a refusal drops that item, keeps the rest in the same single step, and shows its reason on the Done page', async () => {
    const made = makeScene(ROWS, {
      refuse: (call) => (call === 0 ? { index: 1, explanation: 'raw words', refusal: 'rim_not_planar' } : null),
      solidsAfter: 8,
    })
    setup({ scene: made.scene, scope: 'selection', objectIds: [1n, 2n, 3n] })
    fireEvent.click(btn('Use recommended choices'))
    await waitFor(() => expect(page()).toBe('done'))
    expect(made.transacts()).toHaveLength(2)
    expect(made.transacts()[1].params.commands!.map((c) => c.params.object)).toEqual(['obj_1', 'obj_3'])
    expect(screen.getByText("Hew couldn't make this one solid:")).toBeInTheDocument()
    const list = document.querySelector('[data-wizard-list="couldnt-fix"]') as HTMLElement
    expect(within(list).getByText('Roof')).toBeInTheDocument()
    expect(within(list).getByText(/This opening isn't flat/)).toBeInTheDocument()
  })

  it('everything refused: nothing changed, and the refused item is listed', async () => {
    const made = makeScene([row({ object: '1', name: 'A' })], { refuse: () => ({ index: 0, explanation: 'raw', refusal: 'rim_not_planar' }) })
    setup({ scene: made.scene, scope: 'selection', objectIds: [1n] })
    fireEvent.click(btn('Use recommended choices'))
    await waitFor(() => expect(page()).toBe('done'))
    expect(screen.getByRole('heading', { name: 'Nothing changed' })).toBeInTheDocument()
    expect(screen.getByText("Hew couldn't make this one solid:")).toBeInTheDocument()
  })
})

describe('MakeSolidWizard — step by step', () => {
  it('walks the pages that have items; openings default to Leave on a model, marked Recommended and listed first', () => {
    setup()
    fireEvent.click(btn('Go step by step'))
    expect(page()).toBe('openings')
    expect(screen.getByRole('heading', { name: 'Openings' })).toBeInTheDocument()
    const radios = screen.getAllByRole('radio')
    expect(radios.map((r) => r.parentElement!.textContent)).toEqual(['Leave them open(Recommended)', 'Close the openings'])
    expect(screen.getByRole('radio', { name: /Leave them open/ })).toBeChecked()
    expect(screen.getByRole('radio', { name: 'Close the openings' })).not.toBeChecked()
    const list = document.querySelector('[data-wizard-list="openings"]') as HTMLElement
    expect(within(list).getByText('Wall')).toBeInTheDocument()
    expect(within(list).getByText('Chair')).toBeInTheDocument()
    expect(within(list).getByText('4 copies')).toBeInTheDocument()
    fireEvent.click(btn('Accept'))
    expect(page()).toBe('flatPanels') // no split shapes: that page is skipped
    expect(screen.getByRole('radio', { name: /Leave them as panels/ })).toBeChecked()
    expect(screen.getByText('(Recommended)').parentElement).toHaveTextContent('Leave them as panels')
    // Thickness is the other choice; sheets have only the one.
    fireEvent.click(btn('Accept'))
    expect(page()).toBe('curvedSheets')
    expect(screen.getAllByRole('radio')).toHaveLength(1)
    expect(screen.getByRole('radio', { name: /Leave them as sheets/ })).toBeChecked()
    expect(screen.queryByRole('button', { name: 'Choose for each item' })).not.toBeInTheDocument()
    fireEvent.click(btn('Accept'))
    expect(page()).toBe('closerLook')
    expect(screen.getByRole('heading', { name: "Can't be closed" })).toBeInTheDocument()
    expect(screen.queryByRole('radio')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Choose for each item' })).not.toBeInTheDocument()
    expect(screen.getByText('Openings tangled together')).toBeInTheDocument()
    fireEvent.click(btn('Accept'))
    expect(page()).toBe('ready')
    expect(screen.getByRole('heading', { name: 'Ready' })).toBeInTheDocument()
    fireEvent.click(btn('Back'))
    expect(page()).toBe('closerLook')
    fireEvent.click(btn('Back'))
    fireEvent.click(btn('Back'))
    fireEvent.click(btn('Back'))
    expect(page()).toBe('openings')
    fireEvent.click(btn('Back'))
    expect(page()).toBe('overview')
  })

  it('a selection: Close is the recommended opening choice and is listed first', () => {
    setup({ scope: 'selection', objectIds: [1n, 2n, 3n] })
    fireEvent.click(btn('Go step by step'))
    const radios = screen.getAllByRole('radio')
    expect(radios.map((r) => r.parentElement!.textContent)).toEqual(['Close the openings(Recommended)', 'Leave them open'])
    expect(screen.getByRole('radio', { name: /Close the openings/ })).toBeChecked()
  })

  it('non-default choices: close the openings, add a centered 4 mm thickness; Ready says so; Make Solid sends those commands', async () => {
    const made = makeScene()
    setup({ scene: made.scene })
    fireEvent.click(btn('Go step by step'))
    fireEvent.click(screen.getByRole('radio', { name: 'Close the openings' }))
    fireEvent.click(btn('Accept'))
    expect(screen.queryByRole('textbox', { name: 'Thickness' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('radio', { name: 'Add thickness' }))
    const field = screen.getByRole('textbox', { name: 'Thickness' }) as HTMLInputElement
    expect(field.value).toBe('3 mm')
    expect(screen.getByRole('radio', { name: 'In front' })).toBeChecked()
    fireEvent.change(field, { target: { value: '4 mm' } })
    fireEvent.click(screen.getByRole('radio', { name: 'Centered' }))
    fireEvent.click(btn('Accept'))
    fireEvent.click(btn('Accept')) // curved sheets
    fireEvent.click(btn('Accept')) // can't be closed
    expect(page()).toBe('ready')
    const ready = document.querySelector('[data-wizard-ready]') as HTMLElement
    expect(within(ready).getAllByRole('listitem').map((li) => li.textContent)).toEqual([
      'Close the openings in 3 shapes.',
      'Give 2 flat panels a thickness of 4 mm.',
      'Leave 2 objects as they are.',
    ])
    expect(made.transacts()).toHaveLength(0) // nothing changes until Make Solid
    fireEvent.click(btn('Make Solid'))
    await waitFor(() => expect(page()).toBe('done'))
    expect(made.transacts()).toHaveLength(1)
    expect(made.transacts()[0].params.label).toBe('Make Solid')
    expect(made.transacts()[0].params.commands).toEqual([
      { method: 'hew.solid.cap', params: { object: 'obj_1' } },
      { method: 'hew.solid.cap', params: { object: 'obj_2' } },
      { method: 'hew.solid.cap', params: { object: 'obj_3' } },
      { method: 'hew.solid.thicken', params: { object: 'obj_4', distance: 0.004, centered: true } },
      { method: 'hew.solid.thicken', params: { object: 'obj_5', distance: 0.004, centered: true } },
    ])
  })

  it('a thickness that does not parse blocks Accept with the plain message', () => {
    setup()
    fireEvent.click(btn('Go step by step'))
    fireEvent.click(btn('Accept'))
    fireEvent.click(screen.getByRole('radio', { name: 'Add thickness' }))
    fireEvent.change(screen.getByRole('textbox', { name: 'Thickness' }), { target: { value: 'lots' } })
    fireEvent.click(btn('Accept'))
    expect(screen.getByRole('alert')).toHaveTextContent('Enter a thickness, such as 3 mm.')
    expect(page()).toBe('flatPanels')
  })

  it('Choose for each item: ticking one opening under Leave closes exactly that one', async () => {
    const made = makeScene()
    setup({ scene: made.scene })
    fireEvent.click(btn('Go step by step'))
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument()
    fireEvent.click(btn('Choose for each item'))
    const roof = screen.getByRole('checkbox', { name: 'Make solid Roof' })
    expect(roof).not.toBeChecked()
    expect(screen.getByRole('checkbox', { name: 'Make solid Wall' })).not.toBeChecked()
    fireEvent.click(roof)
    expect(roof).toBeChecked()
    for (let i = 0; i < 4; i++) fireEvent.click(btn('Accept'))
    const ready = document.querySelector('[data-wizard-ready]') as HTMLElement
    expect(within(ready).getAllByRole('listitem').map((li) => li.textContent)).toEqual([
      'Close the openings in 1 shape.',
      'Leave 6 objects as they are.',
    ])
    fireEvent.click(btn('Make Solid'))
    await waitFor(() => expect(page()).toBe('done'))
    expect(made.transacts()[0].params.commands).toEqual([{ method: 'hew.solid.cap', params: { object: 'obj_2' } }])
  })

  it('Choose for each item: unticking one under Close skips it', async () => {
    const made = makeScene()
    setup({ scene: made.scene, scope: 'selection', objectIds: [1n, 2n, 3n] })
    fireEvent.click(btn('Go step by step'))
    fireEvent.click(btn('Choose for each item'))
    const roof = screen.getByRole('checkbox', { name: 'Make solid Roof' })
    expect(roof).toBeChecked()
    fireEvent.click(roof)
    for (let i = 0; i < 4; i++) fireEvent.click(btn('Accept'))
    fireEvent.click(btn('Make Solid'))
    await waitFor(() => expect(page()).toBe('done'))
    expect(made.transacts()[0].params.commands).toEqual([
      { method: 'hew.solid.cap', params: { object: 'obj_1' } },
      { method: 'hew.solid.cap', params: { object: 'obj_3' } },
    ])
  })

  it('changing the page radio clears that page\'s ticks', () => {
    setup()
    fireEvent.click(btn('Go step by step'))
    fireEvent.click(btn('Choose for each item'))
    fireEvent.click(screen.getByRole('checkbox', { name: 'Make solid Roof' }))
    expect(screen.getByRole('checkbox', { name: 'Make solid Roof' })).toBeChecked()
    fireEvent.click(screen.getByRole('radio', { name: 'Close the openings' }))
    for (const name of ['Wall', 'Roof', 'Chair']) expect(screen.getByRole('checkbox', { name: `Make solid ${name}` })).toBeChecked()
    fireEvent.click(screen.getByRole('radio', { name: /Leave them open/ }))
    for (const name of ['Wall', 'Roof', 'Chair']) expect(screen.getByRole('checkbox', { name: `Make solid ${name}` })).not.toBeChecked()
  })

  it('the thickness field shows when any panel item is ticked, even under Leave', () => {
    setup()
    fireEvent.click(btn('Go step by step'))
    fireEvent.click(btn('Accept'))
    expect(screen.queryByRole('textbox', { name: 'Thickness' })).not.toBeInTheDocument()
    fireEvent.click(btn('Choose for each item'))
    fireEvent.click(screen.getByRole('checkbox', { name: 'Make solid Glass' }))
    expect(screen.getByRole('textbox', { name: 'Thickness' })).toBeInTheDocument()
    expect(screen.getByRole('radio', { name: 'Centered' })).toBeInTheDocument()
  })

  it('every page is skipped when nothing is on it: only an item that can not be closed goes straight there', () => {
    setup({ scene: makeScene([ROWS[6]]).scene })
    fireEvent.click(btn('Go step by step'))
    expect(page()).toBe('closerLook')
    fireEvent.click(btn('Accept'))
    expect(page()).toBe('ready')
    expect(screen.getByText('Nothing will change. Go back to make a choice, or close this window.')).toBeInTheDocument()
  })
})

describe('MakeSolidWizard — split shapes', () => {
  const rows = [
    row({ object: '1', name: 'Group#18', family: 0, rims: [SEAM, RIM] }),
    row({ object: '2', name: 'Group#18', family: 0, rims: [SEAM] }),
    row({ object: '3', name: 'Group#18', family: 0, verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] }),
  ]
  const families = [{ members: ['1', '2', '3'], shared_rims: 1 }]

  it('offers three choices; caps only the seams, and keeps the walls when asked', async () => {
    const made = makeScene(rows, { families })
    setup({ scene: made.scene })
    expect(screen.getByText('1 shape was split into pieces: rejoin them.')).toBeInTheDocument()
    fireEvent.click(btn('Go step by step'))
    expect(screen.getByRole('heading', { name: 'Rejoin split shapes' })).toBeInTheDocument()
    expect(screen.getAllByRole('radio')).toHaveLength(4)
    expect(screen.getByRole('radio', { name: /Rejoin the pieces and remove the inside walls/ })).toBeChecked()
    expect(screen.getByRole('radio', { name: /Rejoin as one solid/ })).not.toBeChecked()
    fireEvent.click(screen.getByRole('radio', { name: 'Rejoin the pieces and keep the inside walls' }))
    fireEvent.click(btn('Accept'))
    expect(screen.getByText('Rejoin 1 split shape and keep the inside walls.')).toBeInTheDocument()
    fireEvent.click(btn('Make Solid'))
    await waitFor(() => expect(page()).toBe('done'))
    expect(made.transacts()[0].params.commands).toEqual([
      { method: 'hew.solid.cap', params: { object: 'obj_1', rims: [0] } },
      { method: 'hew.solid.cap', params: { object: 'obj_2', rims: [0] } },
    ])
  })

  it('Rejoin as one solid sends one rejoin command carrying the whole family, and the Ready page says so', async () => {
    const made = makeScene(rows, { families })
    setup({ scene: made.scene })
    fireEvent.click(btn('Go step by step'))
    fireEvent.click(screen.getByRole('radio', { name: /Rejoin as one solid/ }))
    fireEvent.click(btn('Accept'))
    expect(screen.getByText('Rejoin 1 split shape as one solid.')).toBeInTheDocument()
    fireEvent.click(btn('Make Solid'))
    await waitFor(() => expect(page()).toBe('done'))
    expect(made.transacts()).toHaveLength(1)
    expect(made.transacts()[0].params.commands).toEqual([
      { method: 'hew.solid.rejoin', params: { family: 0, as: 'one_solid' } },
    ])
  })

  it('Use recommended choices rejoins split shapes on a model, deleting the wall', async () => {
    const made = makeScene(rows, { families })
    setup({ scene: made.scene })
    fireEvent.click(btn('Use recommended choices'))
    await waitFor(() => expect(page()).toBe('done'))
    expect(made.transacts()[0].params.commands!.map((c) => [c.method, c.params.object])).toEqual([
      ['hew.solid.cap', 'obj_1'],
      ['hew.solid.cap', 'obj_2'],
      ['hew.solid.delete_faces', 'obj_3'],
    ])
  })
})

describe('MakeSolidWizard — isolation and open edges', () => {
  it('clicking an item isolates and frames it and draws its open edges; Show all does the whole page; closing clears both', () => {
    const { props, unmount } = setup()
    fireEvent.click(btn('Go step by step'))
    fireEvent.click(btn('Roof'))
    expect(props.onIsolate).toHaveBeenLastCalledWith([{ kind: 'object', id: 2n }])
    expect(lastOverlay(props)).toEqual({ ids: [2n], warnIds: [], current: null, rev: 0, capPreview: null })
    fireEvent.click(btn('Chair'))
    expect(props.onIsolate).toHaveBeenLastCalledWith([{ kind: 'member', object: 3n, instance: 201n }])
    expect(lastOverlay(props)).toMatchObject({ ids: [3n], warnIds: [] })
    fireEvent.click(btn('Show all'))
    expect(props.onIsolate).toHaveBeenLastCalledWith([
      { kind: 'object', id: 1n },
      { kind: 'object', id: 2n },
      { kind: 'member', object: 3n, instance: 201n },
    ])
    expect(lastOverlay(props)).toMatchObject({ ids: [1n, 2n, 3n], warnIds: [] })
    expect(props.onClearIsolation).not.toHaveBeenCalled()
    unmount()
    expect(props.onClearIsolation).toHaveBeenCalled()
    expect(lastOverlay(props)).toBeNull()
  })

  it('selecting an item that can not be closed draws it in the warning color and says why', () => {
    const { props } = setup()
    fireEvent.click(btn('Go step by step'))
    for (let i = 0; i < 3; i++) fireEvent.click(btn('Accept'))
    expect(page()).toBe('closerLook')
    expect(document.querySelector('[data-wizard-reason-callout]')).toBeNull()
    fireEvent.click(btn('Knot'))
    expect(props.onIsolate).toHaveBeenLastCalledWith([{ kind: 'object', id: 7n }])
    // Warning color only: a row in both lists would draw twice in two colors.
    expect(lastOverlay(props)).toEqual({ ids: [], warnIds: [7n], current: null, rev: 0, capPreview: null })
    const callout = document.querySelector('[data-wizard-reason-callout]') as HTMLElement
    expect(callout).toHaveTextContent(
      "The highlighted edges are open. Several openings run together or stop short, so Hew can't tell where each one ends. Shapes like this are usually open on purpose, like a pipe fitting or an electrical box, and work as they are.",
    )
  })

  it('moving to another page clears the isolation, the overlay and the callout', () => {
    const { props } = setup()
    fireEvent.click(btn('Go step by step'))
    for (let i = 0; i < 3; i++) fireEvent.click(btn('Accept'))
    fireEvent.click(btn('Knot'))
    fireEvent.click(btn('Back'))
    expect(props.onClearIsolation).toHaveBeenCalledTimes(1)
    expect(lastOverlay(props)).toBeNull()
    fireEvent.click(btn('Accept'))
    expect(document.querySelector('[data-wizard-reason-callout]')).toBeNull()
  })

  it('Done-page items isolate and highlight too, with the reason', async () => {
    const { props } = setup({ scope: 'selection', objectIds: [1n] })
    fireEvent.click(btn('Use recommended choices'))
    await waitFor(() => expect(page()).toBe('done'))
    fireEvent.click(btn('Knot'))
    expect(props.onIsolate).toHaveBeenLastCalledWith([{ kind: 'object', id: 7n }])
    expect(lastOverlay(props)).toMatchObject({ ids: [], warnIds: [7n] })
    expect(document.querySelector('[data-wizard-reason-callout]')).toHaveTextContent('The highlighted edges are open.')
  })

  it('closing without having isolated anything leaves the viewport alone', () => {
    const { props, unmount } = setup()
    fireEvent.click(btn('Go step by step'))
    unmount()
    expect(props.onClearIsolation).not.toHaveBeenCalled()
    expect(props.onHealthOverlay).not.toHaveBeenCalled()
  })

  it('isolation ending elsewhere drops the selection and the reason; closing then clears nothing', () => {
    const { props, rerender, unmount } = setup()
    fireEvent.click(btn('Go step by step'))
    for (let i = 0; i < 3; i++) fireEvent.click(btn('Accept'))
    fireEvent.click(btn('Knot'))
    expect(document.querySelector('[data-wizard-reason-callout]')).not.toBeNull()
    rerender(<MakeSolidWizard {...props} isolationLost={1} />)
    expect(document.querySelector('[data-wizard-reason-callout]')).toBeNull()
    unmount()
    expect(props.onClearIsolation).not.toHaveBeenCalled()
  })

  it('an edit re-reads what is isolated and draws it at the new revision', () => {
    const { props, rerender } = setup()
    fireEvent.click(btn('Go step by step'))
    fireEvent.click(btn('Roof'))
    rerender(<MakeSolidWizard {...props} docRev={5} />)
    expect(lastOverlay(props)).toEqual({ ids: [2n], warnIds: [], current: null, rev: 5, capPreview: null })
  })

  it('the lip reason reads as a lip', () => {
    const lip = { ...RIM, cappable: false, sheet_edge: true }
    setup({ scene: makeScene([row({ object: '9', name: 'Tray', verdict: { kind: 'holes', planar: 1, warped: 0 }, rims: [lip] })]).scene })
    fireEvent.click(btn('Go step by step'))
    fireEvent.click(btn('Tray'))
    expect(document.querySelector('[data-wizard-reason-callout]')).toHaveTextContent('They run along a flat lip')
  })
})

describe('MakeSolidWizard — cancelling', () => {
  it('Escape that belongs to a viewport gesture or a dialog leaves the window open', () => {
    const { props, rerender } = setup()
    rerender(<MakeSolidWizard {...props} escapeBusy={() => true} />)
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(props.onClose).not.toHaveBeenCalled()
    rerender(<MakeSolidWizard {...props} escapeBusy={() => false} />)
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(props.onClose).toHaveBeenCalledTimes(1)
  })

  it('Escape closes it with no change', () => {
    const made = makeScene()
    const { props } = setup({ scene: made.scene })
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(props.onClose).toHaveBeenCalledTimes(1)
    expect(made.sent).toHaveLength(0)
    expect(props.onGeometryChanged).not.toHaveBeenCalled()
  })

  it('the close button closes it with no change', () => {
    const made = makeScene()
    const { props } = setup({ scene: made.scene })
    fireEvent.click(screen.getByRole('button', { name: 'Close' }))
    expect(props.onClose).toHaveBeenCalledTimes(1)
    expect(made.sent).toHaveLength(0)
  })

  it('Escape in the thickness field leaves the window alone', () => {
    const { props } = setup()
    fireEvent.click(btn('Go step by step'))
    fireEvent.click(btn('Accept'))
    fireEvent.click(screen.getByRole('radio', { name: 'Add thickness' }))
    const field = screen.getByRole('textbox', { name: 'Thickness' })
    field.focus()
    fireEvent.keyDown(field, { key: 'Escape' })
    expect(props.onClose).not.toHaveBeenCalled()
    fireEvent.keyDown(document.body, { key: 'Escape' })
    expect(props.onClose).toHaveBeenCalledTimes(1)
  })

  it('is not modal: no backdrop, aria-modal off', () => {
    setup()
    expect(screen.getByRole('dialog', { name: 'Make Solid' })).toHaveAttribute('aria-modal', 'false')
  })

  it('Escape before the deferred run changes nothing', () => {
    vi.useFakeTimers()
    try {
      const made = makeScene()
      const { props } = setup({ scene: made.scene, scope: 'selection', objectIds: [1n] })
      fireEvent.click(btn('Use recommended choices'))
      fireEvent.keyDown(document, { key: 'Escape' })
      vi.runAllTimers()
      expect(props.onClose).toHaveBeenCalledTimes(1)
      expect(made.sent).toHaveLength(0)
      expect(props.onGeometryChanged).not.toHaveBeenCalled()
    } finally {
      vi.useRealTimers()
    }
  })

  it('closing the window unmounted before the timer fires changes nothing', () => {
    vi.useFakeTimers()
    try {
      const made = makeScene()
      const { props, unmount } = setup({ scene: made.scene, scope: 'selection', objectIds: [1n] })
      fireEvent.click(btn('Use recommended choices'))
      unmount()
      vi.runAllTimers()
      expect(made.sent).toHaveLength(0)
      expect(props.onGeometryChanged).not.toHaveBeenCalled()
    } finally {
      vi.useRealTimers()
    }
  })
})

describe('MakeSolidWizard — window width', () => {
  it('clamps to 420 px and 80 % of the viewport', () => {
    expect(clampWizardWidth(100, 1000)).toBe(420)
    expect(clampWizardWidth(560, 1000)).toBe(560)
    expect(clampWizardWidth(5000, 1000)).toBe(800)
    expect(clampWizardWidth(5000, 400)).toBe(420) // a tiny viewport still gets the minimum
  })

  const handle = () => document.querySelector('[data-wizard-resize]') as HTMLElement
  const widthOf = () => (screen.getByRole('dialog', { name: 'Make Solid' }) as HTMLElement).style.width

  it('opens at 560 px; dragging the left edge resizes within the limits, and the width is remembered', () => {
    const first = setup()
    expect(widthOf()).toBe(`${WIZARD_DEFAULT_WIDTH}px`)
    const viewport = window.innerWidth
    fireEvent.pointerDown(handle(), { clientX: 400, pointerId: 1 })
    fireEvent.pointerMove(handle(), { clientX: 300, pointerId: 1 }) // 100 px wider
    expect(widthOf()).toBe('660px')
    fireEvent.pointerMove(handle(), { clientX: -5000, pointerId: 1 })
    expect(widthOf()).toBe(`${Math.floor(viewport * 0.8)}px`)
    fireEvent.pointerMove(handle(), { clientX: 5000, pointerId: 1 })
    expect(widthOf()).toBe('420px')
    fireEvent.pointerMove(handle(), { clientX: 350, pointerId: 1 })
    expect(widthOf()).toBe('610px')
    fireEvent.pointerUp(handle(), { pointerId: 1 })
    fireEvent.pointerMove(handle(), { clientX: 0, pointerId: 1 }) // no drag in progress: ignored
    expect(widthOf()).toBe('610px')
    first.unmount()
    setup()
    expect(widthOf()).toBe('610px')
  })

  it('clamps the remembered width to the viewport when it opens', () => {
    const before = window.innerWidth
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 600 })
    try {
      setup()
      expect(widthOf()).toBe('480px')
    } finally {
      Object.defineProperty(window, 'innerWidth', { configurable: true, value: before })
    }
  })
})

describe('MakeSolidWizard — long lists', () => {
  it('shows the first 200 items and "Show 200 more" for the next chunk', () => {
    const many = Array.from({ length: 450 }, (_, i) => row({ object: String(i + 1), name: `Box ${i + 1}` }))
    setup({ scene: makeScene(many).scene })
    fireEvent.click(btn('Go step by step'))
    const list = document.querySelector('[data-wizard-list="openings"]') as HTMLElement
    expect(within(list).getAllByRole('listitem')).toHaveLength(LIST_CHUNK)
    fireEvent.click(btn('Show 200 more'))
    expect(within(list).getAllByRole('listitem')).toHaveLength(2 * LIST_CHUNK)
    fireEvent.click(btn('Show 200 more'))
    expect(within(list).getAllByRole('listitem')).toHaveLength(450)
    expect(screen.queryByRole('button', { name: 'Show 200 more' })).not.toBeInTheDocument()
  }, 20_000) // renders 450 rows; the 5 s default is tight under a loaded full-suite run

  it('tells repeated names apart in the lists', () => {
    const { scene } = makeScene([row({ object: '1', name: 'Group#9' }), row({ object: '2', name: 'Group#9' }), row({ object: '3', name: 'Group#9' })])
    setup({ scene })
    fireEvent.click(btn('Go step by step'))
    expect(screen.getAllByRole('listitem').map((n) => n.textContent)).toEqual(['Group#9', 'Group#9 (2)', 'Group#9 (3)'])
  })
})

describe('MakeSolidWizard — a throw', () => {
  it('shows the problem and stays on the page instead of the Done page', async () => {
    const made = makeScene()
    made.scene.api_dispatch.mockImplementation(() => {
      throw new Error('boom')
    })
    const { props } = setup({ scene: made.scene, scope: 'selection', objectIds: [1n] })
    fireEvent.click(btn('Use recommended choices'))
    await act(async () => {
      await new Promise((r) => setTimeout(r, 20))
    })
    expect(props.onToast).toHaveBeenCalledWith('boom', undefined)
    expect(page()).toBe('overview')
    expect(props.onGeometryChanged).not.toHaveBeenCalled()
  })
})

describe('MakeSolidWizard — freshness', () => {
  const baseProps = (scene: WasmScene) => ({
    scene,
    scope: 'model' as const,
    onClose: vi.fn(),
    onIsolate: vi.fn(),
    onClearIsolation: vi.fn(),
    onHealthOverlay: vi.fn(),
    onGeometryChanged: vi.fn(),
  })

  it('reads the plan again when the document revision changes, keeping choices and ticks', () => {
    const rows = [row({ object: '1', name: 'Wall' }), row({ object: '2', name: 'Roof' }), row({ object: '3', name: 'Door' })]
    const { scene } = makeScene(rows)
    const props = baseProps(scene)
    const { rerender } = render(<MakeSolidWizard {...props} docRev={0} />)
    expect(screen.getByText('3 shapes have openings: leave them open.')).toBeInTheDocument()
    fireEvent.click(btn('Go step by step'))
    fireEvent.click(screen.getByRole('button', { name: 'Choose for each item' }))
    fireEvent.click(screen.getByRole('checkbox', { name: 'Make solid Roof' }))
    // The person closes the wall by hand: it no longer needs anything.
    rows.splice(0, 1)
    rerender(<MakeSolidWizard {...props} docRev={1} />)
    expect(screen.getByText('Has openings (2)')).toBeInTheDocument()
    expect(screen.getByRole('checkbox', { name: 'Make solid Roof' })).toBeChecked()
    expect(screen.getByRole('checkbox', { name: 'Make solid Door' })).not.toBeChecked()
    expect(screen.getByRole('radio', { name: /Leave them open/ })).toBeChecked()
  })

  it('goes back to the overview when the page it was on has nothing left', () => {
    const rows = [row({ object: '1', name: 'Wall' }), row({ object: '4', name: 'Glass', verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] })]
    const { scene } = makeScene(rows)
    const props = baseProps(scene)
    const { rerender } = render(<MakeSolidWizard {...props} docRev={0} />)
    fireEvent.click(btn('Go step by step'))
    expect(page()).toBe('openings')
    rows.splice(0, 1)
    rerender(<MakeSolidWizard {...props} docRev={1} />)
    expect(page()).toBe('overview')
  })

  it('Make Solid re-reads the model first: an item closed by hand sends no command', async () => {
    const rows = [row({ object: '1', name: 'Wall' }), row({ object: '2', name: 'Roof' })]
    const made = makeScene(rows)
    setup({ scene: made.scene, scope: 'selection', objectIds: [1n, 2n] })
    rows.splice(0, 1) // closed by hand, and the wizard has not been told yet
    fireEvent.click(btn('Use recommended choices'))
    await waitFor(() => expect(page()).toBe('done'))
    expect(made.transacts()[0].params.commands).toEqual([{ method: 'hew.solid.cap', params: { object: 'obj_2' } }])
  })

  it('closes itself when a different document is loaded', () => {
    const { scene } = makeScene()
    const props = baseProps(scene)
    const { rerender } = render(<MakeSolidWizard {...props} docGeneration={4} />)
    rerender(<MakeSolidWizard {...props} docGeneration={4} docRev={9} />)
    expect(props.onClose).not.toHaveBeenCalled()
    rerender(<MakeSolidWizard {...props} docGeneration={5} />)
    expect(props.onClose).toHaveBeenCalledTimes(1)
  })
})

describe('MakeSolidWizard — Needs mending', () => {
  const WIRE = {
    weld: 0.000002,
    weld_vertices: 312,
    weld_max_gap: 0,
    folds: 0,
    branching_vertices: 2,
    islands: 1,
    hole_rims: 7,
    hole_rims_planar: 3,
    hole_rims_warped: 4,
    outer_rims: 0,
    fill_rims: [0, 1, 2, 3, 4, 5, 6],
    filled_rims: 7,
    fill_curved: 4,
    pieces: 0,
  }
  function mendSetup(onBusy?: (message: string, work: () => void) => boolean) {
    const made = makeScene()
    const scene = made.scene as typeof made.scene & { mend_census_json: ReturnType<typeof vi.fn> }
    scene.mend_census_json = vi.fn(() => JSON.stringify(WIRE))
    const props = {
      scene,
      scope: 'model' as const,
      onClose: vi.fn(),
      onIsolate: vi.fn(),
      onClearIsolation: vi.fn(),
      onHealthOverlay: vi.fn(),
      onGeometryChanged: vi.fn(),
      onToast: vi.fn(),
      onBusy,
    }
    render(<MakeSolidWizard {...props} />)
    return { ...made, props }
  }

  it('the page offers Mend, on by default, with what it will do for each item', () => {
    mendSetup()
    expect(screen.getByText(/1 shape needs mending/)).toBeInTheDocument()
    fireEvent.click(btn('Go step by step'))
    while (page() !== 'closerLook') fireEvent.click(btn('Accept'))
    expect(screen.getByRole('heading', { name: 'Needs mending' })).toBeInTheDocument()
    expect(screen.queryByRole('heading', { name: "Can't be closed" })).not.toBeInTheDocument()
    expect(screen.getByRole('radio', { name: /Mend what can be mended/ })).toBeChecked()
    expect(screen.getByRole('radio', { name: /Leave them as they are/ })).not.toBeChecked()
    expect(document.querySelector('[data-wizard-detail]')!.textContent).toBe(
      'Merge 312 nearby points, separate 2 spots where parts touch at a single point and fill 7 small gaps (3 flat, 4 curved)',
    )
    fireEvent.click(btn('Knot'))
    expect(document.querySelector('[data-wizard-reason-callout]')!.textContent).toContain('Mend can merge nearby points')
    fireEvent.click(btn('Accept'))
    expect(page()).toBe('ready')
    expect(within(document.querySelector('[data-wizard-ready]') as HTMLElement).getByText('Mend 1 object.')).toBeInTheDocument()
  })

  it('Use recommended choices mends inside the one transaction, behind the busy card', async () => {
    const onBusy = vi.fn((_m: string, work: () => void) => {
      work()
      return true
    })
    const { transacts, props } = mendSetup(onBusy)
    fireEvent.click(btn('Use recommended choices'))
    await waitFor(() => expect(page()).toBe('done'))
    expect(onBusy).toHaveBeenCalledWith('Mending…', expect.any(Function))
    expect(transacts()).toHaveLength(1)
    expect(transacts()[0].params.label).toBe('Make Solid')
    expect(transacts()[0].params.commands).toEqual([
      { method: 'hew.solid.mend', params: { object: 'obj_7', weld: true, folds: true, split: true, fill: 'holes' } },
    ])
    expect(props.onGeometryChanged).toHaveBeenCalled()
  })

  it('says so, and runs nothing, when the busy card is taken', () => {
    const { transacts, props } = mendSetup(() => false)
    fireEvent.click(btn('Use recommended choices'))
    expect(props.onToast).toHaveBeenCalledWith('Another change is still running. Press Enter or click again when it finishes.')
    expect(transacts()).toHaveLength(0)
    expect(page()).toBe('overview')
    expect(btn('Use recommended choices')).toBeEnabled()
  })

  it('choosing to leave them writes nothing', async () => {
    const { transacts } = mendSetup(() => true)
    fireEvent.click(btn('Go step by step'))
    while (page() !== 'closerLook') fireEvent.click(btn('Accept'))
    fireEvent.click(screen.getByRole('radio', { name: /Leave them as they are/ }))
    fireEvent.click(btn('Accept'))
    expect(within(document.querySelector('[data-wizard-ready]') as HTMLElement).getByText('Nothing will change. Go back to make a choice, or close this window.')).toBeInTheDocument()
    expect(transacts()).toHaveLength(0)
  })
})
