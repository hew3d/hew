/**
 * Component tests for the phone Print sheet (design_handoff_printing
 * SPEC.md §4). Mirrors `panels/PrintDialog.test.tsx`'s harness: a fake
 * `ViewportApi` exposing the minimal `renderPrintPages`/`getPrintView`/
 * `computePrintExtent` surface `usePrintController` reads (jsdom has no
 * canvas/WebGL), `../print/pdf`'s `buildPrintPdf` mocked (wasm), and
 * `../print/cutList`'s `cutListRows` mocked (needs a real kernel scene).
 */
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { PrintSheet, type PrintSheetProps } from './PrintSheet'
import type { ViewportApi } from '../viewport/Viewport'
import type { PrintHost } from '../print/printHost'
import { getPrintPrefs, setPrintPrefs, DEFAULT_PRINT_PREFS } from '../settings/print'
import { buildPrintPdf } from '../print/pdf'

vi.mock('../print/pdf', () => ({
  buildPrintPdf: vi.fn().mockResolvedValue(new Uint8Array([0x25, 0x50, 0x44, 0x46])),
}))

const mockCutListRows = vi.hoisted(() => vi.fn(() => [] as { label: string; section: string; size: string; depth: number }[]))
vi.mock('../print/cutList', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../print/cutList')>()
  return { ...actual, cutListRows: mockCutListRows }
})

function createFakeApi() {
  return {
    getPrintView: vi.fn(() => ({
      dir: [0, 0, -1] as [number, number, number],
      up: [0, 1, 0] as [number, number, number],
      target: [0, 0, 0] as [number, number, number],
      projection: 'perspective' as const,
      orthoSize: { w: 1, h: 1 },
      aspect: 4 / 3,
    })),
    computePrintExtent: vi.fn(() => ({
      center: [0, 0, 0] as [number, number, number],
      rect: { x: -0.05, y: -0.05, w: 0.1, h: 0.1 },
      depth: { min: -0.05, max: 0.05 },
      empty: false,
    })),
    getSelectedIds: vi.fn(() => ({ objects: [] as bigint[], instances: [] as bigint[] })),
    getHiddenIds: vi.fn(() => ({ objects: [] as bigint[], instances: [] as bigint[] })),
    getSectionState: vi.fn(() => null as { origin: [number, number, number]; normal: [number, number, number]; active: boolean } | null),
    collectAnnotationDrawing: vi.fn(() => ({
      segments: [] as number[],
      labels: [] as { position: [number, number, number]; text: string; detached: boolean }[],
    })),
    renderPrintPages: vi.fn(async (pages: { widthPx: number; heightPx: number }[], opts: { format: string }) =>
      pages.map(() => new Blob([new Uint8Array(4)], { type: opts.format })),
    ),
  }
}
type FakeApi = ReturnType<typeof createFakeApi>

function renderSheet(overrides: Partial<PrintSheetProps> = {}, api: FakeApi = createFakeApi()) {
  const onClose = vi.fn()
  const printPrint = vi.fn().mockResolvedValue(undefined)
  const printHost: PrintHost = { print: printPrint }
  const savePdf = vi.fn().mockResolvedValue(true)
  const props: PrintSheetProps = {
    open: true,
    orientation: 'portrait',
    getViewportApi: () => api as unknown as ViewportApi,
    getScene: () => null,
    getScenes: () => [],
    documentName: 'bridge.hew',
    sceneName: null,
    printHost,
    savePdf,
    onClose,
    ...overrides,
  }
  const utils = render(<PrintSheet {...props} />)
  return { ...utils, api, onClose, printHost, printPrint, savePdf }
}

beforeAll(() => {
  URL.createObjectURL = vi.fn(() => 'blob:x')
  URL.revokeObjectURL = vi.fn()
  Object.defineProperty(HTMLImageElement.prototype, 'decode', {
    configurable: true,
    value: () => Promise.resolve(),
  })
  Object.defineProperty(window.navigator, 'language', { value: 'en-US', configurable: true })
})

beforeEach(() => {
  localStorage.clear()
  setPrintPrefs(structuredClone(DEFAULT_PRINT_PREFS))
  mockCutListRows.mockReset()
  mockCutListRows.mockReturnValue([])
})

describe('PrintSheet', () => {
  it('renders null when closed', () => {
    renderSheet({ open: false })
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('opens with the mode switch, Paper/Style rows, and the action buttons', () => {
    renderSheet()
    expect(screen.getByRole('dialog', { name: 'Print Layout' })).toBeInTheDocument()

    const modeGroup = screen.getByRole('group', { name: 'Mode' })
    expect(modeGroup).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Standard' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByRole('button', { name: 'Scaled' })).toHaveAttribute('aria-pressed', 'false')

    expect(screen.getByRole('button', { name: 'Paper' })).toBeInTheDocument()
    expect(screen.getByRole('group', { name: 'Style' })).toBeInTheDocument()
    // Standard mode: no Scale/Extent rows.
    expect(screen.queryByRole('button', { name: 'Scale' })).not.toBeInTheDocument()
    expect(screen.queryByRole('group', { name: 'Extent' })).not.toBeInTheDocument()

    expect(screen.getByRole('button', { name: 'Save PDF…' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Print…' })).toBeInTheDocument()
  })

  it('switching to Scaled shows the Scale/Extent rows; Fit updates the scale label', () => {
    renderSheet()
    fireEvent.click(screen.getByRole('button', { name: 'Scaled' }))

    const scaleBtn = screen.getByRole('button', { name: 'Scale' })
    expect(scaleBtn).toBeInTheDocument()
    expect(screen.getByRole('group', { name: 'Extent' })).toBeInTheDocument()
    const before = scaleBtn.textContent

    fireEvent.click(screen.getByRole('button', { name: 'Fit' }))
    // Fit picks the largest preset that fits the fake's 0.1m extent — for
    // Letter/normal-margin/title-on that is not the 1:1 default, so the
    // scale row's label text changes.
    expect(screen.getByRole('button', { name: 'Scale' }).textContent).not.toBe(before)
  })

  it('opening the Paper list and picking A4 updates the row', () => {
    renderSheet()
    fireEvent.click(screen.getByRole('button', { name: 'Paper' }))
    fireEvent.click(screen.getByRole('button', { name: 'A4' }))
    expect(screen.getByRole('button', { name: 'Paper' })).toHaveTextContent('A4')
  })

  it('Save PDF… calls the savePdf prop and stays quiet afterwards (the share sheet was the confirmation)', async () => {
    const { savePdf } = renderSheet()
    fireEvent.click(screen.getByRole('button', { name: 'Save PDF…' }))

    await waitFor(() => expect(savePdf).toHaveBeenCalledTimes(1))
    expect(buildPrintPdf).toHaveBeenCalledTimes(1)
    await waitFor(() => expect(screen.getByRole('dialog', { name: 'Print Layout' })).toHaveAttribute('data-status', 'saved'))
    expect(screen.queryByTestId('print-sheet-summary')).not.toBeInTheDocument()
  })

  it('Print… hands off to the print host and stays quiet afterwards', async () => {
    const { printPrint } = renderSheet()
    fireEvent.click(screen.getByRole('button', { name: 'Print…' }))

    await waitFor(() => expect(printPrint).toHaveBeenCalledTimes(1))
    await waitFor(() => expect(screen.getByRole('dialog', { name: 'Print Layout' })).toHaveAttribute('data-status', 'sent'))
    expect(document.getElementById('hew-print-root')).not.toBeNull()
    expect(screen.queryByTestId('print-sheet-summary')).not.toBeInTheDocument()
  })

  it('carries no summary line and no AirPrint blurb at rest — the strip and rows say it all', () => {
    renderSheet()
    expect(screen.queryByTestId('print-sheet-summary')).not.toBeInTheDocument()
    expect(screen.queryByTestId('print-sheet-note')).not.toBeInTheDocument()
    expect(screen.queryByText(/AirPrint/)).not.toBeInTheDocument()
  })

  it('renders the centered card in landscape', () => {
    renderSheet({ orientation: 'landscape' })
    expect(screen.getByTestId('print-sheet-card')).toBeInTheDocument()
    expect(screen.queryByTestId('print-sheet-sheet')).not.toBeInTheDocument()
  })

  it('renders the bottom sheet in portrait', () => {
    renderSheet({ orientation: 'portrait' })
    expect(screen.getByTestId('print-sheet-sheet')).toBeInTheDocument()
    expect(screen.queryByTestId('print-sheet-card')).not.toBeInTheDocument()
  })

  it('Escape and the scrim both close the sheet', () => {
    const { onClose } = renderSheet()
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(onClose).toHaveBeenCalledTimes(1)
    fireEvent.click(screen.getByTestId('shop-print-scrim'))
    expect(onClose).toHaveBeenCalledTimes(2)
  })

  it('disables both actions while nothing is visible to print (Scaled, empty extent)', () => {
    const api = createFakeApi()
    api.computePrintExtent.mockReturnValue({ center: [0, 0, 0], rect: { x: 0, y: 0, w: 0, h: 0 }, depth: { min: 0, max: 0 }, empty: true })
    renderSheet({}, api)
    // Standard mode always prints whatever's on screen (`empty: false`
    // unconditionally, printJob.ts) — only Scaled mode can be empty.
    fireEvent.click(screen.getByRole('button', { name: 'Scaled' }))
    expect(screen.getByRole('button', { name: 'Save PDF…' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Print…' })).toBeDisabled()
  })
})
