/**
 * Component tests for the Print dialog (docs/design/printing.md §4,
 * design_handoff_printing/SPEC.md §2).
 *
 * jsdom has no canvas/WebGL, so `getViewportApi()` is replaced wholesale by a
 * fake exposing the minimal `ViewportApi` subset the dialog reads —
 * `renderPrintPages` IS the renderer here, returning a tiny placeholder Blob
 * per requested page instead of an actual GPU capture. `getScene()` defaults
 * to null so the vector Line-art path (which needs a real kernel `Scene`) is
 * skipped and the raster/preview code paths run; one test supplies a fake
 * scene with a `line_drawing()` method to exercise the vector path directly.
 *
 * `Save PDF…` goes through `wasm/pkg/wasm_api.js`'s `build_pdf`, which jsdom
 * cannot load — `../print/pdf` is mocked so `buildPrintPdf` never touches
 * wasm. `../print/cutList`'s `cutListRows` needs a real kernel scene shape
 * (`buildPartsSheetSections`) that isn't worth faking here, so it is mocked
 * too; everything else in that module (furniture layout, `blankTile`) is the
 * real implementation.
 *
 * Segmented controls (Orientation/Margins/Extent/Style/mode) render as real
 * `<button aria-pressed>`s inside a `role="group"` — `getByRole('button',
 * {name})` finds them the same way it would a native toggle button; Paper /
 * View / Scale / Pages stay native `<select>`s.
 */
import { render, screen, within, fireEvent, waitFor } from '@testing-library/react'
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { PrintDialog, type PrintDialogProps } from './PrintDialog'
import type { ViewportApi } from '../viewport/Viewport'
import type { Scene as WasmScene } from '../wasm/loader'
import type { SceneSource } from '../print/printJob'
import type { PrintHost } from '../print/printHost'
import { getPrintPrefs, setPrintPrefs, DEFAULT_PRINT_PREFS } from '../settings/print'
import { regionFromLocale } from '../shop/localeUnits'
import { paperSize, MARGIN_MM, defaultPaperForRegion } from '../print/paper'
import { fitRatioFor, DEFAULT_TITLE_BLOCK_MM } from '../print/layout'
import { fitScale, sameScale, scalePresetsFor } from '../print/scale'
import { buildPrintPdf } from '../print/pdf'
import { setLengthUnit } from '../settings/units'

vi.mock('../print/pdf', () => ({
  buildPrintPdf: vi.fn().mockResolvedValue(new Uint8Array([0x25, 0x50, 0x44, 0x46])),
}))

// `cutListRows` needs a real wasm `Scene` (`buildPartsSheetSections`); the
// rest of the module (page layout, `blankTile`) is real so the cut-list page
// still exercises real furniture/layout code.
const mockCutListRows = vi.hoisted(() => vi.fn(() => [] as { label: string; qty: number; l: string; w: string; h: string }[]))
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

function renderDialog(overrides: Partial<PrintDialogProps> = {}, api: FakeApi = createFakeApi()) {
  const onClose = vi.fn()
  const printPrint = vi.fn().mockResolvedValue(undefined)
  const printHost: PrintHost = { print: printPrint }
  const savePdf = vi.fn().mockResolvedValue(true)
  const onPagesReady = vi.fn()
  const props: PrintDialogProps = {
    getViewportApi: () => api as unknown as ViewportApi,
    getScene: () => null,
    getScenes: () => [],
    documentName: 'bridge.hew',
    sceneName: null,
    printHost,
    savePdf,
    onClose,
    onPagesReady,
    ...overrides,
  }
  const utils = render(<PrintDialog {...props} />)
  return { ...utils, api, onClose, printHost, printPrint, savePdf, onPagesReady }
}

/** The dialog root — carries data-status / data-summary (the footer is
 * quiet at rest; the summary text lives on the root for the harness). */
const dialogRoot = () => screen.getByRole('dialog', { name: 'Print Layout' })
const summaryAttr = () => dialogRoot().getAttribute('data-summary') ?? ''

/** Click a segmented-control chip by its group testid + visible label
 * ("print-orientation" -> "Portrait"). */
function clickChip(testId: string, label: string) {
  const group = screen.getByTestId(testId)
  fireEvent.click(within(group).getByRole('button', { name: label }))
}

beforeAll(() => {
  // jsdom has neither — the component reads/writes object URLs for preview
  // and print-resolution page bitmaps.
  URL.createObjectURL = vi.fn(() => 'blob:x')
  URL.revokeObjectURL = vi.fn()
  // jsdom never fetches `blob:`/`data:` <img> sources, so `complete` stays
  // false and load/error never fire — `waitForPrintImages()`'s fallback path
  // would hang forever. Stubbing `decode()` (which IS in the real browser
  // API, just absent from jsdom) makes it take the fast path instead.
  Object.defineProperty(HTMLImageElement.prototype, 'decode', {
    configurable: true,
    value: () => Promise.resolve(),
  })
  // Pin the locale so `seededPrefs()`'s one-time paper seed
  // (`defaultPaperForRegion(regionFromLocale(navigator.language))`) is
  // deterministic across machines/CI instead of whatever jsdom defaults to.
  Object.defineProperty(window.navigator, 'language', { value: 'en-US', configurable: true })
})

beforeEach(() => {
  localStorage.clear()
  setPrintPrefs(structuredClone(DEFAULT_PRINT_PREFS))
  mockCutListRows.mockReset()
  mockCutListRows.mockReturnValue([])
})

describe('PrintDialog', () => {
  it('renders the dialog with the mode toggle, action buttons, and a one-page summary', () => {
    renderDialog()
    expect(screen.getByRole('dialog', { name: 'Print Layout' })).toBeInTheDocument()

    const modeGroup = screen.getByTestId('print-mode')
    const standardBtn = within(modeGroup).getByRole('button', { name: 'Standard' })
    const scaledBtn = within(modeGroup).getByRole('button', { name: 'Scaled' })
    expect(standardBtn).toHaveAttribute('aria-pressed', 'true')
    expect(scaledBtn).toHaveAttribute('aria-pressed', 'false')

    expect(screen.getByTestId('print-confirm')).toHaveTextContent('Print…')
    expect(screen.getByTestId('print-save-pdf')).toHaveTextContent('Save PDF…')
    expect(summaryAttr()).toContain('1 page')
  })

  it('calls onClose on Escape', () => {
    const { onClose } = renderDialog()
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('calls onClose on an overlay click, but not on a click inside the dialog', () => {
    const { onClose } = renderDialog()
    fireEvent.click(screen.getByRole('dialog'))
    expect(onClose).not.toHaveBeenCalled()
    fireEvent.click(screen.getByTestId('print-dialog-overlay'))
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('shows Scaled-only controls only in Scaled mode (and never a projection note)', () => {
    renderDialog()
    expect(screen.queryByLabelText('View')).not.toBeInTheDocument()
    expect(screen.queryByLabelText('Scale')).not.toBeInTheDocument()
    expect(screen.queryByTestId('print-extent')).not.toBeInTheDocument()

    clickChip('print-mode', 'Scaled')
    expect(screen.getByLabelText('View')).toBeInTheDocument()
    expect(screen.getByLabelText('Scale')).toBeInTheDocument()
    expect(screen.getByTestId('print-extent')).toBeInTheDocument()
    // The fake viewport reports `projection: 'perspective'` — the print
    // simply uses parallel projection; the dialog does not say so.
    expect(screen.queryByTestId('print-projection-note')).not.toBeInTheDocument()

    clickChip('print-mode', 'Standard')
    expect(screen.queryByLabelText('View')).not.toBeInTheDocument()
    expect(screen.queryByTestId('print-projection-note')).not.toBeInTheDocument()
  })

  it('disables the Selection extent when nothing is selected', () => {
    renderDialog()
    clickChip('print-mode', 'Scaled')
    const extentGroup = screen.getByTestId('print-extent')
    const selectionBtn = within(extentGroup).getByRole('button', { name: 'Selection' })
    expect(selectionBtn).toBeDisabled()
    // The reason rides on the chip itself, not a line of exposition.
    expect(selectionBtn).toHaveAttribute('title', 'Nothing is selected.')
    expect(screen.queryByText('Nothing is selected.')).not.toBeInTheDocument()
  })

  it('Current view extent is always available and pulls View to Current view; a standard view drops it back to Model', () => {
    renderDialog()
    clickChip('print-mode', 'Scaled')
    const viewSelect = screen.getByLabelText('View') as HTMLSelectElement
    fireEvent.change(viewSelect, { target: { value: 'top' } })
    const cv = within(screen.getByTestId('print-extent')).getByRole('button', { name: 'Current view' })
    expect(cv).not.toBeDisabled()
    fireEvent.click(cv)
    expect(viewSelect.value).toBe('current')
    expect(cv).toHaveAttribute('aria-pressed', 'true')
    fireEvent.change(viewSelect, { target: { value: 'front' } })
    expect(within(screen.getByTestId('print-extent')).getByRole('button', { name: 'Model' })).toHaveAttribute('aria-pressed', 'true')
  })

  it('enables the Selection extent once something is selected', () => {
    // `hasSelection` is read once at mount (see the suspected bug noted in
    // the effort's final report) — a fresh instance is the only way to
    // exercise the "something is selected" branch.
    const api = createFakeApi()
    api.getSelectedIds.mockReturnValue({ objects: [1n], instances: [] })
    renderDialog({}, api)
    clickChip('print-mode', 'Scaled')
    const extentGroup = screen.getByTestId('print-extent')
    const selectionBtn = within(extentGroup).getByRole('button', { name: 'Selection' })
    expect(selectionBtn).not.toBeDisabled()
    expect(screen.queryByText('Nothing is selected.')).not.toBeInTheDocument()
  })

  it('Fit picks the exact scale that fills one page to the margins (a custom scale, shown as a ratio)', () => {
    renderDialog()
    clickChip('print-mode', 'Scaled')
    fireEvent.click(screen.getByRole('button', { name: 'Fit' }))

    // Mirror the dialog's own math (docs/design/printing.md §4) with the
    // real pure helpers instead of hard-coding an expected scale: locale
    // seeds the paper to Letter, defaults are orientation auto / normal
    // margin / title block on / overlap on, and the fake's extent is a
    // 100 mm (0.1 m) square.
    const paper = paperSize(defaultPaperForRegion(regionFromLocale('en-US')))
    const fitRatio = fitRatioFor({
      paper,
      orientation: 'auto',
      marginMm: MARGIN_MM.normal,
      titleBlockMm: DEFAULT_TITLE_BLOCK_MM,
      overlapMm: 10,
      dpi: 300,
      extent: { x: -0.05, y: -0.05, w: 0.1, h: 0.1 },
    })
    const expected = fitScale(fitRatio, 'metric')
    // Not a ladder step: the exact ratio (a 100 mm square on Letter → ~1:0.41 = 2.44:1).
    expect(scalePresetsFor('m').some((p) => sameScale(p, expected))).toBe(false)
    const scaleSelect = screen.getByLabelText('Scale') as HTMLSelectElement
    expect(scaleSelect.value).toBe('fit')
    expect(scaleSelect.selectedOptions[0].textContent).toBe(`${expected.label} (fit)`)
    expect(summaryAttr()).toContain('1 page')
    // The summary/title carry the bare ratio, once.
    expect(summaryAttr()).toContain(`· ${expected.label} ·`)
  })

  it('Print… renders every page, hands off to the print host, and reports completion', async () => {
    const { api, printPrint, onPagesReady } = renderDialog()
    fireEvent.click(screen.getByTestId('print-confirm'))

    await waitFor(() => expect(printPrint).toHaveBeenCalledTimes(1))
    expect(api.renderPrintPages).toHaveBeenCalled()

    const setup = printPrint.mock.calls[0][0]
    // Letter, whichever orientation Standard/auto picked for a 4:3 viewport.
    const dims = [setup.paperWmm, setup.paperHmm].map((v: number) => Math.round(v * 10) / 10).sort((a: number, b: number) => a - b)
    expect(dims).toEqual([215.9, 279.4])

    expect(onPagesReady).toHaveBeenCalledTimes(1)
    const [pages] = onPagesReady.mock.calls[0]
    expect(pages[0].imageUrl).toBe('blob:x')

    await waitFor(() => expect(dialogRoot()).toHaveAttribute('data-status', 'sent'))
    expect(document.getElementById('hew-print-root')).not.toBeNull()
    expect(document.querySelectorAll('#hew-print-root .hew-print-page').length).toBeGreaterThan(0)
  })

  it('Save PDF… builds the PDF through the mocked writer and saves it', async () => {
    const { savePdf } = renderDialog({ documentName: 'bridge.hew' })
    fireEvent.click(screen.getByTestId('print-save-pdf'))

    await waitFor(() => expect(savePdf).toHaveBeenCalledTimes(1))
    expect(buildPrintPdf).toHaveBeenCalledTimes(1)

    const [bytes, name] = savePdf.mock.calls[0]
    expect(bytes).toBeInstanceOf(Uint8Array)
    // The file is named after the document WITHOUT its extension.
    expect(name).toMatch(/^bridge — /)
    expect(name).not.toContain('.hew')

    await waitFor(() => expect(dialogRoot()).toHaveAttribute('data-status', 'saved'))
    // The footer stays quiet — the save dialog was the confirmation.
    expect(screen.getByTestId('print-summary')).toHaveTextContent('')
  })

  it('offers "Each Scene" when Scenes exist and updates the summary when selected', () => {
    const scenes: SceneSource[] = [
      { sid: 1, name: 'Front', camera: null, hidden: null, section: undefined },
      { sid: 2, name: 'Back', camera: null, hidden: null, section: undefined },
    ]
    renderDialog({ getScenes: () => scenes })

    const pagesSelect = document.getElementById('print-pages-select') as HTMLSelectElement
    expect(pagesSelect).not.toBeNull()
    expect(within(pagesSelect).getByText('Each Scene (2)')).toBeInTheDocument()

    fireEvent.change(pagesSelect, { target: { value: 'each_scene' } })
    expect(summaryAttr()).toContain('2 pages · 2 Scenes')
  })

  it('renders vector line-art SVG in the preview when the kernel scene is available', () => {
    // A 100 mm square hard-edge loop in the print camera's view plane
    // (meters, y up) — matches the fake `computePrintExtent`'s extent.
    const fakeScene = {
      line_drawing: vi.fn(() => ({
        coords: () => new Float64Array([-0.05, -0.05, 0.05, -0.05, 0.05, -0.05, 0.05, 0.05, 0.05, 0.05, -0.05, 0.05, -0.05, 0.05, -0.05, -0.05]),
        kinds: () => new Uint8Array([0, 0, 0, 0]),
        sids: () => new BigUint64Array([1n, 1n, 1n, 1n]),
        bounds: () => [-0.05, -0.05, 0.05, 0.05],
        free: () => {},
      })),
    }
    const { container } = renderDialog({ getScene: () => fakeScene as unknown as WasmScene })
    // Scaled + Line art (the default `scaled` style preference) is what
    // routes through the vector path instead of raster.
    clickChip('print-mode', 'Scaled')
    expect(container.querySelector('.hew-print-drawing[data-kind="vector"] svg')).not.toBeNull()
  })

  it('adds a cut list page when "Cut list page" is checked (cutListRows mocked)', () => {
    mockCutListRows.mockReturnValue([{ label: 'Leg', qty: 1, l: '1 m', w: '2 m', h: '3 m' }])
    renderDialog({ getScene: () => ({} as unknown as WasmScene) })

    const before = screen.getAllByTestId('print-preview-page').length
    fireEvent.click(screen.getByRole('checkbox', { name: /Cut list page/ }))

    expect(summaryAttr()).toContain('+ 1 cut list')
    expect(screen.getAllByTestId('print-preview-page')).toHaveLength(before + 1)
  })

  it('sizes the cut list on the REAL page geometry: a tiled Scaled job with the band reserved reports one consistent "of N" on every page', async () => {
    // 25 rows: 25 fit a full-height Letter drawing area (244 mm) but only 24
    // fit the tiled one (234 mm, overlap band reserved) — so the count must
    // come from the job's own layout, not a reference page.
    mockCutListRows.mockReturnValue(Array.from({ length: 25 }, (_, i) => ({ label: `Part ${i}`, qty: 1, l: '1 m', w: '2 m', h: '3 m' })))
    const api = createFakeApi()
    api.computePrintExtent.mockReturnValue({ center: [0, 0, 0], rect: { x: -0.25, y: -0.225, w: 0.5, h: 0.45 }, depth: { min: -0.02, max: 0.02 }, empty: false })
    renderDialog({ getScene: () => ({} as unknown as WasmScene) }, api)
    clickChip('print-mode', 'Scaled')
    clickChip('print-orientation', 'Portrait')
    const scaleSelect = screen.getByLabelText('Scale') as HTMLSelectElement
    const oneToOne = Array.from(scaleSelect.options).findIndex((o) => o.textContent?.startsWith('1:1 ') || o.textContent === '1:1')
    fireEvent.change(scaleSelect, { target: { value: String(oneToOne) } })
    fireEvent.click(screen.getByRole('checkbox', { name: /Cut list page/ }))
    await waitFor(() => expect(summaryAttr()).toContain('6 pages (3 × 2) + 2 cut list'))
    expect(screen.getAllByTestId('print-preview-page')).toHaveLength(8)
    const pageTexts = Array.from(document.querySelectorAll('[data-testid=print-preview-page] text[data-role=title-page]')).map((t) => t.textContent ?? '')
    expect(pageTexts).toHaveLength(8)
    expect(pageTexts.every((t) => t.includes('of 8'))).toBe(true)
  })

  it('Fit closes the custom-scale row and a stale row never overwrites the fitted scale', async () => {
    renderDialog()
    clickChip('print-mode', 'Scaled')
    const scaleSelect = screen.getByLabelText('Scale') as HTMLSelectElement
    fireEvent.change(scaleSelect, { target: { value: 'custom' } })
    const paperIn = screen.getByLabelText('On paper') as HTMLInputElement
    const modelIn = screen.getByLabelText('In model') as HTMLInputElement
    fireEvent.change(paperIn, { target: { value: '1 cm' } })
    fireEvent.change(modelIn, { target: { value: '3 cm' } })
    fireEvent.blur(modelIn)
    expect(summaryAttr()).toContain('(1:3)')
    fireEvent.click(screen.getByRole('button', { name: 'Fit' }))
    expect(screen.queryByLabelText('On paper')).not.toBeInTheDocument()
    expect(summaryAttr()).not.toContain('(1:3)')
    expect(scaleSelect.value).not.toBe('custom')
  })

  it('a stale custom row never re-commits: pick a preset after a custom scale, reopen Custom, blur — the preset stands', () => {
    renderDialog()
    clickChip('print-mode', 'Scaled')
    const scaleSelect = screen.getByLabelText('Scale') as HTMLSelectElement
    fireEvent.change(scaleSelect, { target: { value: 'custom' } })
    fireEvent.change(screen.getByLabelText('On paper'), { target: { value: '1 cm' } })
    fireEvent.change(screen.getByLabelText('In model'), { target: { value: '3 cm' } })
    fireEvent.blur(screen.getByLabelText('In model'))
    expect(summaryAttr()).toContain('(1:3)')
    // A preset from the menu replaces it…
    fireEvent.change(scaleSelect, { target: { value: '0' } })
    expect(summaryAttr()).not.toContain('(1:3)')
    // …and reopening Custom shows a fresh, empty row; a blur commits nothing.
    fireEvent.change(scaleSelect, { target: { value: 'custom' } })
    const paperIn = screen.getByLabelText('On paper') as HTMLInputElement
    expect(paperIn.value).toBe('')
    fireEvent.focus(paperIn)
    fireEvent.blur(paperIn)
    expect(summaryAttr()).not.toContain('(1:3)')
  })

  it('recovers from a plan that failed against an in-flight print pass (retries instead of a dead dialog)', async () => {
    const api = createFakeApi()
    let calls = 0
    api.computePrintExtent.mockImplementation(() => {
      calls += 1
      // The first measurement lands while a preview pass is active.
      if (calls === 1) throw new Error('SceneRenderer: a print pass is already active')
      return { center: [0, 0, 0] as [number, number, number], rect: { x: -0.05, y: -0.05, w: 0.1, h: 0.1 }, depth: { min: -0.05, max: 0.05 }, empty: false }
    })
    renderDialog({}, api)
    clickChip('print-mode', 'Scaled')
    await waitFor(() => expect(summaryAttr()).toContain('1 page'), { timeout: 3000 })
    expect(calls).toBeGreaterThanOrEqual(2)
  })

  it('shows a stored preset with the ladder\'s CURRENT label (an old “1\" = 1\'-0\"” reads “1\" = 1\'”)', () => {
    setLengthUnit('arch')
    try {
      const prefs = getPrintPrefs()
      setPrintPrefs({
        ...prefs,
        lastMode: 'scaled',
        scaled: { ...prefs.scaled, scale: { paperMeters: 0.0254, modelMeters: 0.3048, label: '1" = 1\'-0"', kind: 'preset' } },
      })
      renderDialog()
      const scaleSelect = screen.getByLabelText('Scale') as HTMLSelectElement
      const shown = scaleSelect.selectedOptions[0].textContent ?? ''
      expect(shown).toContain('1" = 1\'')
      expect(shown).not.toContain('-0"')
      expect(scaleSelect.value).not.toBe('custom')
    } finally {
      setLengthUnit('m')
    }
  })

  it('persists a hand-picked paper across dialog instances', () => {
    const first = renderDialog()
    const paperSelect = screen.getByLabelText('Paper') as HTMLSelectElement
    fireEvent.change(paperSelect, { target: { value: 'a4' } })
    expect(paperSelect.value).toBe('a4')
    first.unmount()

    renderDialog()
    const paperSelect2 = screen.getByLabelText('Paper') as HTMLSelectElement
    expect(paperSelect2.value).toBe('a4')
    expect(getPrintPrefs().paperSeeded).toBe(true)
  })

  it('a page click opens it at 100 % print scale (1 CSS mm per mm, pannable); a second click goes back', () => {
    // Inspect resolves from pointerdown/pointerup on the groups container
    // (not a native `click` — see the long comment on `onGroupsPointerDown`
    // in PrintDialog.tsx), so the gesture is simulated the same way here.
    const { container } = renderDialog()
    const preview = screen.getByTestId('print-preview')
    const tile = screen.getAllByTestId('print-preview-page')[0]
    fireEvent.pointerDown(tile, { clientX: 10, clientY: 10 })
    fireEvent.pointerUp(preview, { clientX: 10, clientY: 10 })
    expect(screen.getByText(/100 % — drag to pan · click to go back/)).toBeInTheDocument()
    const overlay = container.querySelector('.hwprint__inspect')
    expect(overlay).not.toBeNull()
    // Real size: Letter is 215.9 × 279.4 mm = 816 × 1056 CSS px at 96 dpi
    // (landscape here — the fake viewport is 4:3 and Standard's Auto follows it).
    const pageBox = container.querySelector<HTMLElement>('.hwprint__inspect-page')!
    expect([816, 1056]).toContain(Math.round(parseFloat(pageBox.style.width)))
    expect(preview.classList.contains('hwprint__groups--inspect')).toBe(true)
    fireEvent.pointerDown(overlay!, { clientX: 10, clientY: 10 })
    fireEvent.pointerUp(preview, { clientX: 10, clientY: 10 })
    expect(screen.queryByText(/click to go back/)).not.toBeInTheDocument()
  })

  it('drags the preview to nudge the tile grid (Scaled, multi-tile) and Center resets it', async () => {
    const api = createFakeApi()
    // A 500x300mm slab so 1:1 tiles onto more than one page.
    api.computePrintExtent.mockReturnValue({ center: [0, 0, 0], rect: { x: -0.25, y: -0.15, w: 0.5, h: 0.3 }, depth: { min: -0.02, max: 0.02 }, empty: false })
    renderDialog({}, api)
    clickChip('print-mode', 'Scaled')
    const scaleSelect = screen.getByLabelText('Scale') as HTMLSelectElement
    const oneToOne = Array.from(scaleSelect.options).findIndex((o) => o.textContent?.startsWith('1:1 ') || o.textContent === '1:1')
    fireEvent.change(scaleSelect, { target: { value: String(oneToOne) } })
    await waitFor(() => expect(screen.getAllByTestId('print-preview-page').length).toBeGreaterThan(1))

    const preview = screen.getByTestId('print-preview')
    fireEvent.pointerDown(preview, { clientX: 0, clientY: 0 })
    fireEvent.pointerMove(preview, { clientX: 40, clientY: 20 })
    fireEvent.pointerUp(preview, { clientX: 40, clientY: 20 })

    const center = await screen.findByTestId('print-nudge-readout')
    expect(center).toHaveTextContent('Center')
    expect(center.getAttribute('title')).toMatch(/mm/)
    fireEvent.click(center)
    await waitFor(() => expect(screen.queryByTestId('print-nudge-readout')).not.toBeInTheDocument())
  })

  it('nudges the tile grid 1 mm per arrow key, 10 mm with Shift', async () => {
    const api = createFakeApi()
    api.computePrintExtent.mockReturnValue({ center: [0, 0, 0], rect: { x: -0.25, y: -0.15, w: 0.5, h: 0.3 }, depth: { min: -0.02, max: 0.02 }, empty: false })
    renderDialog({}, api)
    clickChip('print-mode', 'Scaled')
    const scaleSelect = screen.getByLabelText('Scale') as HTMLSelectElement
    const oneToOne = Array.from(scaleSelect.options).findIndex((o) => o.textContent?.startsWith('1:1 ') || o.textContent === '1:1')
    fireEvent.change(scaleSelect, { target: { value: String(oneToOne) } })
    await waitFor(() => expect(screen.getAllByTestId('print-preview-page').length).toBeGreaterThan(1))

    const preview = screen.getByTestId('print-preview')
    fireEvent.keyDown(preview, { key: 'ArrowRight' })
    let readout = await screen.findByTestId('print-nudge-readout')
    expect(readout.getAttribute('title')).toContain('1 mm')

    fireEvent.keyDown(preview, { key: 'ArrowRight', shiftKey: true })
    readout = await screen.findByTestId('print-nudge-readout')
    expect(readout.getAttribute('title')).toContain('11 mm')
  })

  it('shows the browser-print fallback link on a driver error', async () => {
    const printPrint = vi.fn().mockRejectedValue(new Error('no driver'))
    const fallbackPrint = vi.fn().mockResolvedValue(undefined)
    const printHost: PrintHost = { print: printPrint, fallback: { print: fallbackPrint } }
    renderDialog({ printHost })
    fireEvent.click(screen.getByTestId('print-confirm'))
    await waitFor(() => expect(screen.getByTestId('print-summary')).toHaveTextContent("Couldn’t open the system print dialog."))
    const link = screen.getByRole('button', { name: 'Open the browser print dialog instead' })
    fireEvent.click(link)
    await waitFor(() => expect(fallbackPrint).toHaveBeenCalledTimes(1))
  })
})
