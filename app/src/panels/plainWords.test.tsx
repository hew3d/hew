/**
 * Plain words: none of the kernel's words (leaky, rim, planar, manifold,
 * seam, verdict, cappable, partition, surface, accepted) or the old
 * "needs fixing" words (need fixing, fix model, closer look, watertight)
 * appear anywhere a person can read in the Model Health panel, the Make
 * Solid wizard (every page, with the reason callout), the import report, the
 * STL/3MF warning, or the Add thickness dialog. (Object Info's lines and the
 * badge are checked in scenePanels.test.tsx and App.test.tsx beside their
 * other health tests.)
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { MakeSolidWizard } from './MakeSolidWizard'
import { ModelHealthPanel } from './ModelHealthPanel'
import { ImportReportDialog } from './ImportReportDialog'
import { StlExportDialog } from './StlExportDialog'
import { ThickenDialog } from './ThickenDialog'
import { BANNED_RE, readableText } from '../health/bannedWords'
import type { Scene as WasmScene } from '../wasm/loader'
import type { ImportReport } from '../io/fileHost'
import { taggedDispatch } from '../test/taggedDispatch'

const RIM = { role: 'outer', planar: true, closed: true, cappable: true, sheet_edge: false, shared: false, perimeter: 4, extent: 1, edges: 4 }
const SEAM = { ...RIM, shared: true }
const LIP = { ...RIM, cappable: false, sheet_edge: true }
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

const ROWS = [
  row({ object: '1', name: 'Wall' }),
  row({ object: '2', name: 'Chair leg', definition: '9', definition_name: 'Chair', placements: 3, instances: ['201'] }),
  row({ object: '3', name: 'Glass', verdict: { kind: 'flat_sheet', holes: 1 }, rims: [WARPED] }),
  row({ object: '4', name: 'Hill', verdict: { kind: 'bent_sheet' }, rims: [WARPED] }),
  row({ object: '5', name: 'Knot', verdict: { kind: 'non_manifold' }, branching_vertices: 2 }),
  row({ object: '6', name: 'Box', verdict: { kind: 'mixed' }, rims: [RIM, WARPED] }),
  row({ object: '7', name: 'Cube', family: 0, rims: [SEAM] }),
  row({ object: '10', name: 'Tray', verdict: { kind: 'holes', planar: 1, warped: 0 }, rims: [LIP] }),
  row({ object: '11', name: 'Bent', verdict: { kind: 'holes', planar: 0, warped: 1 }, rims: [WARPED] }),
  row({ object: '8', name: 'Cube', family: 0, verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] }),
  row({ object: '9', name: 'Kept glass', accepted: true, verdict: { kind: 'flat_sheet', holes: 0 }, rims: [WARPED] }),
]
const COUNTS = { objects: 16, solids: 5, flat_sheets: 3, bent_sheets: 1, cappable: 3, holes: 0, mixed: 1, non_manifold: 1, seam_families: 1 }

function makeScene(refuse = false) {
  let applied = false
  return {
    health_report_json: vi.fn(() =>
      JSON.stringify({ rows: ROWS, families: [{ members: ['7', '8'], shared_rims: 1 }], counts: { ...COUNTS, solids: applied ? 8 : 5 } }),
    ),
    health_rim_highlight: vi.fn(() => new Float32Array([0, 0, 0, 1, 1, 1])),
    health_rim_segments: vi.fn(() => new Float32Array()),
    last_import_object_ids: vi.fn(() => new BigUint64Array([1n])),
    group_members: () => [],
    instance_expanded_members: vi.fn(() => new BigUint64Array()),
    mend_census_json: vi.fn(() =>
      JSON.stringify({ weld: 0.000002, weld_vertices: 312, weld_max_gap: 0, folds: 4, branching_vertices: 2, islands: 1, hole_rims: 7, hole_rims_planar: 3, hole_rims_warped: 4, outer_rims: 0, fill_rims: [0, 1, 2, 3, 4, 5, 6], filled_rims: 7, fill_curved: 4, pieces: 0 }),
    ),
    mend_census_points: vi.fn(() => new Float32Array([0, 0, 0])),
    weld_range: vi.fn(() => new Float64Array([1e-9, 1e-4])),
    default_weld_distance: vi.fn(() => 0.0000019),
    api_connection_open: vi.fn(() => 3),
    api_connection_close: vi.fn(),
    object_public_ids: vi.fn((ids: BigUint64Array) => Array.from(ids, (id) => `obj_${id}`)),
    api_dispatch: vi.fn(taggedDispatch((_c: number, raw: string) => {
      const req = JSON.parse(raw) as { id: number; method: string }
      if (req.method !== 'hew.doc.transact') return JSON.stringify({ jsonrpc: '2.0', id: req.id, result: {} })
      if (refuse && !applied) {
        applied = true
        return JSON.stringify({
          jsonrpc: '2.0',
          id: req.id,
          error: { code: -32000, message: 'refused', data: { refusal: 'cap_would_be_coplanar', failed_index: 0, explanation: 'x' } },
        })
      }
      applied = true
      return JSON.stringify({ jsonrpc: '2.0', id: req.id, result: {} })
    })),
  } as unknown as WasmScene
}

const noBanned = (root: ParentNode): void => {
  const text = readableText(root)
  expect(text, 'the readable text must not contain a kernel word').not.toMatch(BANNED_RE)
  expect(text.length).toBeGreaterThan(20)
}

describe('plain words on screen', () => {
  it('the Model Health panel: groups, rows, buttons, tooltips, with the kept rows shown', () => {
    const { container } = render(
      <ModelHealthPanel
        scene={makeScene()}
        docRev={0}
        selectedIds={[]}
        scope="document"
        onScopeChange={vi.fn()}
        objectFilter={null}
        onClearObjectFilter={vi.fn()}
        onSelectNode={vi.fn()}
        onIsolate={vi.fn()}
        onClearIsolation={vi.fn()}
        onFrameBounds={vi.fn()}
        onOverlayChange={vi.fn()}
        onGeometryChanged={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByRole('checkbox', { name: 'Show kept panels and sheets' }))
    noBanned(container)
    // Select a row so its action buttons and the opening stepper show too.
    fireEvent.click(document.querySelector('[data-health-row="6"]') as HTMLElement)
    noBanned(container)
    fireEvent.click(document.querySelector('[data-health-row="3"]') as HTMLElement)
    noBanned(container)
    // A row Mend can act on: the button, the preview line, the reason, and the choices.
    fireEvent.click(document.querySelector('[data-health-row="5"]') as HTMLElement)
    expect(screen.getByRole('button', { name: 'Mend' })).toBeInTheDocument()
    expect(document.querySelector('[data-health-mend-preview]')).not.toBeNull()
    noBanned(container)
    fireEvent.click(document.querySelector('[data-health-mend-choose]') as HTMLElement)
    expect(document.querySelector('[data-health-mend-options]')).not.toBeNull()
    noBanned(container)
    fireEvent.mouseEnter(screen.getByRole('button', { name: 'Mend' }))
    noBanned(container)
    // The scope menu.
    noBanned(screen.getByRole('combobox', { name: 'Scope' }))
  })

  it('every page of the Make Solid wizard, Ready, and Done (with a refusal and every reason)', async () => {
    const scene = makeScene(true)
    const props = { onClose: vi.fn(), onIsolate: vi.fn(), onClearIsolation: vi.fn(), onHealthOverlay: vi.fn(), onGeometryChanged: vi.fn() }
    const { container } = render(<MakeSolidWizard scene={scene} scope="model" {...props} />)
    noBanned(container)
    fireEvent.click(screen.getByRole('button', { name: 'Go step by step' }))
    const seen: string[] = []
    for (let i = 0; i < 8; i++) {
      const page = document.querySelector('[data-make-solid-wizard]')!.getAttribute('data-wizard-page')!
      seen.push(page)
      if (page === 'flatPanels') {
        fireEvent.click(screen.getByRole('radio', { name: 'Add thickness' }))
        noBanned(container)
      }
      if (page === 'openings' || page === 'split' || page === 'flatPanels') {
        fireEvent.click(screen.getByRole('button', { name: 'Choose for each item' }))
      }
      if (page === 'closerLook') {
        // Each item's open edges and its reason, for every kind of reason.
        for (const name of ['Knot', 'Tray', 'Bent']) {
          fireEvent.click(screen.getByRole('button', { name }))
          expect(document.querySelector('[data-wizard-reason-callout]')).not.toBeNull()
          noBanned(container)
        }
      }
      noBanned(container)
      if (page === 'ready') break
      fireEvent.click(screen.getByRole('button', { name: 'Accept' }))
    }
    expect(seen).toEqual(['openings', 'split', 'flatPanels', 'curvedSheets', 'closerLook', 'ready'])
    fireEvent.click(screen.getByRole('button', { name: 'Back' }))
    fireEvent.click(screen.getByRole('button', { name: 'Accept' }))
    // Close the openings by hand-picking them, so a refusal reaches the Done page.
    fireEvent.click(screen.getByRole('button', { name: 'Back' }))
    for (let i = 0; i < 4; i++) fireEvent.click(screen.getByRole('button', { name: 'Back' }))
    fireEvent.click(screen.getByRole('radio', { name: 'Close the openings' }))
    for (let i = 0; i < 5; i++) fireEvent.click(screen.getByRole('button', { name: 'Accept' }))
    fireEvent.click(screen.getByRole('button', { name: 'Make Solid' }))
    await waitFor(() => expect(document.querySelector('[data-wizard-page="done"]')).not.toBeNull())
    noBanned(container)
    expect(document.querySelector('[data-wizard-reason]')).not.toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Tray' }))
    noBanned(container)
    expect(document.querySelector('[data-wizard-reason-callout]')).not.toBeNull()
  })

  it('the overview and Done page reached with Use recommended choices, per scope', async () => {
    for (const scope of ['import', 'selection', 'export', 'model'] as const) {
      const props = { onClose: vi.fn(), onIsolate: vi.fn(), onClearIsolation: vi.fn(), onHealthOverlay: vi.fn(), onGeometryChanged: vi.fn() }
      const { container, unmount } = render(<MakeSolidWizard scene={makeScene()} scope={scope} objectIds={[1n]} {...props} />)
      noBanned(container)
      fireEvent.click(screen.getByRole('button', { name: 'Use recommended choices' }))
      await waitFor(() => expect(document.querySelector('[data-wizard-page="done"]')).not.toBeNull())
      noBanned(container)
      unmount()
    }
  })

  it('the import report, for every mix of results', () => {
    const base: ImportReport = {
      objects_created: 20,
      lines_added: 0,
      watertight: 5,
      leaky: 15,
      health: { objects: 20, solids: 5, flat_sheets: 6, bent_sheets: 2, cappable: 3, holes: 2, mixed: 1, non_manifold: 1, seam_families: 0 },
      skipped: [],
      textures_missing: [],
      warnings: [],
    }
    for (const opened of [false, true]) {
      const { container, unmount } = render(<ImportReportDialog report={base} opened={opened} onClose={vi.fn()} onMakeSolid={vi.fn()} />)
      noBanned(container)
      unmount()
    }
    const panelsOnly = { ...base, health: { ...base.health, cappable: 0, holes: 0, mixed: 0, non_manifold: 0 } }
    const { container } = render(<ImportReportDialog report={panelsOnly} onClose={vi.fn()} onMakeSolid={vi.fn()} />)
    noBanned(container)
  })

  it('the STL/3MF warning', () => {
    for (const formatLabel of ['STL', '3MF']) {
      const { container, unmount } = render(
        <StlExportDialog offenders={['Roof', 'Object 7']} formatLabel={formatLabel} onExport={vi.fn()} onMakeSolid={vi.fn()} onCancel={vi.fn()} />,
      )
      noBanned(container)
      unmount()
    }
  })

  it('the Add thickness dialog', () => {
    const { container } = render(<ThickenDialog count={3} onConfirm={vi.fn()} onCancel={vi.fn()} />)
    noBanned(container)
  })

  it('the checker itself catches a banned word', () => {
    const { container } = render(<div title="Non-manifold">All good, a leaky pipe near the rim of the sink.</div>)
    expect(readableText(container)).toMatch(BANNED_RE)
    for (const w of ['leaky', 'rim', 'planar', 'manifold', 'seam', 'verdict', 'cappable', 'partition', 'surface', 'accepted', 'need fixing', 'Needs fixing', 'Fix Model', 'a closer look', 'watertight']) {
      expect(`a ${w} b`).toMatch(BANNED_RE)
    }
    expect('primary trim').not.toMatch(BANNED_RE)
  })
})
