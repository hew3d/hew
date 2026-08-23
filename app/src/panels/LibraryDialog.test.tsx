/**
 * LibraryDialog — component tests for the Library browser modal.
 *
 * `../io/libraryStore` and `../library/itemFiles` are both mocked: an
 * in-memory file map stands in for the storage seam, and item bytes are
 * plain text "markers" (`sha256Hex`/`readItemSummary`/`updateItemMeta` all
 * key off the marker string rather than doing real parsing/hashing) — these
 * tests exercise the DIALOG's own wiring (listing, filtering, selection,
 * keyboard actions, manage mutations), not the wasm summary reader or the
 * real Tauri backend.
 */

import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  clampColWidthLive,
  COL_WIDTH_MAX,
  COL_WIDTH_MIN,
  LibraryDialog,
  loadColWidths,
  persistColWidths,
  type LibraryDialogProps,
} from './LibraryDialog'
import type { LibraryItemSummary } from '../library/types'
import { loadListColWidths } from './library/LibraryListHead'

/** Scopes a name lookup to the tile grid (`role="listbox"`). Plain
 * `screen.getByText`/`findByText` are ambiguous for whichever item is
 * currently SELECTED: its name renders both as the tile label AND inside
 * the detail pane's auto-growing name `<textarea>` — unlike an `<input>`,
 * a controlled textarea's value renders as real DOM text content, so a
 * global text query matches both. */
function grid(): HTMLElement {
  // Thumbnail view is a listbox of options; list view is a real
  // three-column grid (role=grid) — same container either way.
  return screen.queryByRole('listbox') ?? screen.getByRole('grid')
}

/** The "All" category tab.
 *
 *  Matched WITHOUT a trailing `\b`: the tab renders its label and its item
 *  count as two adjacent `<span>`s, and accessible-name computation
 *  concatenates inline children with no separator — so the name is "All3",
 *  not "All 3", and a word boundary after "all" never matches. (jsdom 25
 *  inserted a space here; jsdom 30 does not.) Still unambiguous: the other
 *  category tabs are Components/Materials/Models, and the Collections "All"
 *  button carries no `tab` role. */
const ALL_TAB = /^all/i

/** Resolve once the dialog's DEFAULT SELECTION has landed and the detail pane
 *  is showing a real item.
 *
 *  `LibraryDialog` picks the first visible tile from an effect that reacts to
 *  `displayItems`, so the selection necessarily lands a render after the grid
 *  itself. Waiting on a tile label (`findByText('Theater Chair')`) therefore
 *  proves the GRID is populated but says nothing about the detail pane, which
 *  renders `.hwlib__detail--empty` until the selection arrives. Any assertion
 *  reaching straight into the detail pane has to wait for this instead of
 *  assuming the two land in the same tick. */
async function awaitDefaultSelection(): Promise<void> {
  await waitFor(() =>
    expect(document.querySelector('.hwlib__detail:not(.hwlib__detail--empty)')).not.toBeNull(),
  )
}

// jsdom implements neither — the dialog creates an object URL per thumbnail
// and per texture preview, so a no-op stub keeps the load effects from
// throwing without pretending to model real blob semantics.
if (typeof URL.createObjectURL !== 'function') {
  URL.createObjectURL = () => 'blob:mock'
}
if (typeof URL.revokeObjectURL !== 'function') {
  URL.revokeObjectURL = () => {}
}

/** Every dialog's Escape handler must `stopPropagation()` alongside its
 * `preventDefault()` — see `panels/dialogs.test.tsx`'s identical helper,
 * duplicated here rather than imported since that file is off-limits
 * (STRICT FILE OWNERSHIP for this effort). */
function expectEscapeStopsPropagationToWindow(): void {
  const windowKeyDown = vi.fn()
  window.addEventListener('keydown', windowKeyDown)
  try {
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(windowKeyDown).not.toHaveBeenCalled()
  } finally {
    window.removeEventListener('keydown', windowKeyDown)
  }
}

// ---------------------------------------------------------------------------
// In-memory library store + itemFiles mocks
// ---------------------------------------------------------------------------

const fx = vi.hoisted(() => ({
  files: new Map<string, Uint8Array>(),
  thumbnails: new Map<string, Uint8Array>(),
  summaries: new Map<string, unknown>(),
  available: true,
  folderPath: '~/Hew Library' as string | null,
  listeners: new Set<() => void>(),
  canReveal: true,
  revealed: [] as string[],
  removed: [] as string[],
  mutationCounter: 0,
  // S18 coverage: forces `write` to reject so a manage mutation's failure
  // path (surfacing `actionError` instead of an unhandled rejection) is
  // exercised for real.
  failWrite: false,
  // Web-store knobs (webLibraryStore's optional surface).
  canDownload: false,
  needsReconnect: false,
  reconnectResult: true,
  reconnectCalls: 0,
  // Forces list() itself to reject — the load-level (not per-item) failure.
  failList: false,
}))

function bytesFor(marker: string): Uint8Array {
  return new TextEncoder().encode(marker)
}
function markerOf(bytes: Uint8Array): string {
  return new TextDecoder().decode(bytes)
}

vi.mock('../io/libraryStore', () => ({
  libraryStore: () => ({
    available: () => fx.available,
    folderInfo: async () => ({ path: fx.folderPath }),
    chooseFolder: async () => null,
    list: async () => {
      if (fx.failList) throw new Error('storage exploded')
      return Array.from(fx.files.entries()).map(([fileName, bytes]) => ({
        fileName,
        size: bytes.length,
        mtimeMs: 1_700_000_000_000,
      }))
    },
    read: async (fileName: string) => {
      const bytes = fx.files.get(fileName)
      if (!bytes) throw new Error('not found')
      return bytes
    },
    write: async (fileName: string, bytes: Uint8Array) => {
      if (fx.failWrite) throw new Error('disk full')
      fx.files.set(fileName, bytes)
    },
    remove: async (fileName: string) => {
      fx.files.delete(fileName)
      fx.removed.push(fileName)
    },
    readThumbnail: async (key: string) => fx.thumbnails.get(key) ?? null,
    writeThumbnail: async (key: string, png: Uint8Array) => {
      fx.thumbnails.set(key, png)
    },
    reveal: async (fileName: string) => {
      fx.revealed.push(fileName)
    },
    capabilities: () => ({ canReveal: fx.canReveal, canChooseFolder: true, canDownload: fx.canDownload }),
    webStorage: async () => ({ mode: 'browser', needsReconnect: fx.needsReconnect }),
    reconnect: async () => {
      fx.reconnectCalls += 1
      if (fx.reconnectResult) fx.needsReconnect = false
      return fx.reconnectResult
    },
    subscribe: (l: () => void) => {
      fx.listeners.add(l)
      return () => fx.listeners.delete(l)
    },
  }),
}))

// The grid's arrow-key nav measures the ACTUAL rendered column count
// (`../panels/library/gridMeasure`'s `measureGridColumns`) instead of
// assuming a fixed 3/4-column layout — jsdom has no layout engine (every
// `offsetTop` reads 0), so the DOM-reading half of that module can't be
// exercised meaningfully here. Mocking it lets these tests pick a column
// count and assert the dialog actually uses it.
const gridMeasureFx = vi.hoisted(() => ({ columns: 3 }))
vi.mock('./library/gridMeasure', () => ({
  measureGridColumns: () => gridMeasureFx.columns,
}))

// The 'window' variant's native menu/confirm (`../library/nativeChrome`) —
// only ever reached from a `variant="window"` render (dynamically imported,
// but `vi.mock` intercepts that the same as a static import).
const nativeChromeFx = vi.hoisted(() => ({
  confirmResult: true,
  confirmCalls: [] as { title: string; message: string; actionLabel: string }[],
  menuPicks: [] as (string | null)[],
  // Every `entries` array `openNativeMenu` (LibraryDialog.tsx) ever built,
  // in call order — the mock previously discarded this argument entirely,
  // so nothing could assert on what the native ⋯ menu actually OFFERS
  // (finding #5: e.g. that a model's entries omit "Remove Source Info").
  menuEntryCalls: [] as { id: string; label: string; separator?: boolean }[][],
}))
vi.mock('../library/nativeChrome', () => ({
  nativeConfirm: async (title: string, message: string, actionLabel: string) => {
    nativeChromeFx.confirmCalls.push({ title, message, actionLabel })
    return nativeChromeFx.confirmResult
  },
  popupNativeMenu: async (entries: { id: string; label: string; separator?: boolean }[]) => {
    nativeChromeFx.menuEntryCalls.push(entries)
    return nativeChromeFx.menuPicks.shift() ?? null
  },
}))

vi.mock('../library/itemFiles', () => ({
  readItemSummary: async (bytes: Uint8Array) => {
    const s = fx.summaries.get(markerOf(bytes))
    if (!s) throw new Error('BadFixture: unknown item marker')
    return s
  },
  renderItemThumbnail: async () => new Uint8Array([1, 2, 3, 4]),
  updateItemMeta: async (bytes: Uint8Array, meta: Record<string, unknown>) => {
    const oldMarker = markerOf(bytes)
    const oldSummary = fx.summaries.get(oldMarker) as LibraryItemSummary | undefined
    if (!oldSummary) throw new Error('BadFixture: unknown item marker')
    const oldMeta = (oldSummary.doc_attrs['hew.library'] ?? {}) as Record<string, unknown>
    const newMeta = { ...oldMeta }
    for (const [k, v] of Object.entries(meta)) {
      if (v === null) delete newMeta[k]
      else newMeta[k] = v
    }
    const newMarker = `${oldMarker}#mut${fx.mutationCounter++}`
    const newSummary: LibraryItemSummary = { ...oldSummary, doc_attrs: { 'hew.library': newMeta } }
    fx.summaries.set(newMarker, newSummary)
    return bytesFor(newMarker)
  },
  readItemAsset: async () => new Uint8Array([9, 9, 9]),
  sha256Hex: async (bytes: Uint8Array) => `hash-${markerOf(bytes)}`,
}))

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

function summary(overrides: Partial<LibraryItemSummary> = {}): LibraryItemSummary {
  return {
    format_version: 1,
    objects: 0,
    materials: 0,
    components: 0,
    instances: 0,
    groups: 0,
    world_sketches: 0,
    annotations: 0,
    guides: 0,
    first_component_name: null,
    first_component_sid: null,
    first_root_name: null,
    doc_attrs: {},
    material_entries: [],
    ...overrides,
  }
}

const CHAIR = summary({
  objects: 1,
  components: 1,
  groups: 0,
  instances: 1,
  doc_attrs: { 'hew.library': { id: 'src-chair', name: 'Theater Chair', keywords: ['seating'] } },
})
const HINGE = summary({
  objects: 1,
  components: 1,
  groups: 0,
  instances: 1,
  doc_attrs: { 'hew.library': { id: 'src-hinge', name: 'Door Hinge', keywords: ['hardware'] } },
})
const OAK = summary({
  materials: 1,
  material_entries: [{ name: 'Oak', color: [180, 140, 90, 255], texture_asset: null, texture_format: null, texture_world_size: null, content_hash: '1234' }],
  doc_attrs: { 'hew.library': { id: 'src-oak', name: 'Oak' } },
})
const HOUSE = summary({
  objects: 5,
  groups: 2,
  materials: 2,
  doc_attrs: { 'hew.library': { id: 'src-house', name: 'Little House' } },
})

/** Seeds the mock store: `broken.hew` is deliberately NOT registered in
 * `fx.summaries`, so `readItemSummary` throws for it and the dialog builds
 * an error-state tile (the errored-item path). */
function seedDefault() {
  fx.files.clear()
  fx.summaries.clear()
  fx.thumbnails.clear()
  fx.files.set('chair.hew', bytesFor('chair'))
  fx.summaries.set('chair', CHAIR)
  fx.files.set('hinge.hew', bytesFor('hinge'))
  fx.summaries.set('hinge', HINGE)
  fx.files.set('oak.hew', bytesFor('oak'))
  fx.summaries.set('oak', OAK)
  fx.files.set('house.hew', bytesFor('house'))
  fx.summaries.set('house', HOUSE)
  fx.files.set('broken.hew', bytesFor('broken'))
}

function baseProps(overrides: Partial<LibraryDialogProps> = {}): LibraryDialogProps {
  return {
    open: true,
    onClose: vi.fn(),
    placements: {},
    onInsert: vi.fn(),
    onOpenAsDocument: vi.fn(),
    onPaintWith: vi.fn(),
    onAddToPalette: vi.fn(),
    ...overrides,
  }
}

beforeEach(() => {
  fx.files.clear()
  fx.summaries.clear()
  fx.thumbnails.clear()
  fx.listeners.clear()
  fx.available = true
  fx.folderPath = '~/Hew Library'
  fx.canReveal = true
  fx.revealed = []
  fx.removed = []
  fx.mutationCounter = 0
  fx.failWrite = false
  fx.canDownload = false
  fx.needsReconnect = false
  fx.reconnectResult = true
  fx.reconnectCalls = 0
  fx.failList = false
  gridMeasureFx.columns = 3
  localStorage.removeItem('hew.library.view')
  localStorage.removeItem('hew.library.listCols')
  localStorage.removeItem('hew.library.colWidths')
  nativeChromeFx.confirmResult = true
  nativeChromeFx.confirmCalls = []
  nativeChromeFx.menuPicks = []
  nativeChromeFx.menuEntryCalls = []
})

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('LibraryDialog', () => {
  it('renders nothing when closed', () => {
    seedDefault()
    render(<LibraryDialog {...baseProps({ open: false })} />)
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('has the expected ARIA dialog role and label', async () => {
    seedDefault()
    render(<LibraryDialog {...baseProps()} />)
    expect(screen.getByRole('dialog', { name: /library/i })).toBeInTheDocument()
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()
  })

  it('lists items with per-category counts (errored items count as models)', async () => {
    seedDefault()
    const { container } = render(<LibraryDialog {...baseProps()} />)
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()
    await within(grid()).findByText('Door Hinge')
    await awaitDefaultSelection()
    const counts = Array.from(container.querySelectorAll('.hwlib__cat-count')).map((el) => el.textContent)
    // All: every item = 5. Components: chair, hinge = 2. Materials: oak = 1.
    // Models: house + the errored "broken" fixture (erroredItem defaults to
    // 'model') = 2.
    expect(counts).toEqual(['5', '2', '1', '2'])
  })

  it('switches category', async () => {
    seedDefault()
    render(<LibraryDialog {...baseProps()} />)
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()

    fireEvent.click(screen.getByRole('tab', { name: /materials/i }))
    await within(grid()).findByText('Oak')
    await awaitDefaultSelection()
    expect(within(grid()).queryByText('Theater Chair')).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole('tab', { name: /models/i }))
    await within(grid()).findByText('Little House')
    await awaitDefaultSelection()
    expect(within(grid()).queryByText('Oak')).not.toBeInTheDocument()
  })

  it('filters by search query within the current category', async () => {
    seedDefault()
    render(<LibraryDialog {...baseProps()} />)
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()
    await within(grid()).findByText('Door Hinge')
    await awaitDefaultSelection()

    fireEvent.change(screen.getByPlaceholderText(/search name or keyword/i), { target: { value: 'chair' } })
    expect(within(grid()).getByText('Theater Chair')).toBeInTheDocument()
    expect(within(grid()).queryByText('Door Hinge')).not.toBeInTheDocument()
  })

  it('scope "In this model" keeps only items with a positive placement count', async () => {
    seedDefault()
    render(<LibraryDialog {...baseProps({ placements: { 'src-chair': 3 } })} />)
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()
    await within(grid()).findByText('Door Hinge')
    await awaitDefaultSelection()

    fireEvent.click(screen.getByRole('button', { name: /in this model/i }))
    expect(within(grid()).getByText('Theater Chair')).toBeInTheDocument()
    expect(within(grid()).queryByText('Door Hinge')).not.toBeInTheDocument()
  })

  it('Enter triggers the category default action (Insert for components) on the selected tile', async () => {
    seedDefault()
    const onInsert = vi.fn()
    render(<LibraryDialog {...baseProps({ onInsert })} />)
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()

    fireEvent.click(within(grid()).getByText('Theater Chair'))
    fireEvent.keyDown(document, { key: 'Enter' })

    expect(onInsert).toHaveBeenCalledOnce()
    const [item, bytes] = onInsert.mock.calls[0]
    expect(item.displayName).toBe('Theater Chair')
    // `ArrayBuffer.isView` rather than `toBeInstanceOf(Uint8Array)` — the
    // mocked bytes cross a realm boundary under vitest's jsdom pool, where
    // `instanceof` can't be trusted across the module/test-runner divide.
    expect(ArrayBuffer.isView(bytes)).toBe(true)
  })

  it('mod+Enter triggers the "other" action (Open as document for components)', async () => {
    seedDefault()
    const onOpenAsDocument = vi.fn()
    render(<LibraryDialog {...baseProps({ onOpenAsDocument })} />)
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()

    fireEvent.click(within(grid()).getByText('Theater Chair'))
    fireEvent.keyDown(document, { key: 'Enter', metaKey: true })

    expect(onOpenAsDocument).toHaveBeenCalledOnce()
    expect(onOpenAsDocument.mock.calls[0][0].displayName).toBe('Theater Chair')
  })

  it('calls onClose when Escape is pressed', async () => {
    seedDefault()
    const onClose = vi.fn()
    render(<LibraryDialog {...baseProps({ onClose })} />)
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('stops Escape from bubbling to window (so it does not also fire the Viewport handler)', async () => {
    seedDefault()
    render(<LibraryDialog {...baseProps()} />)
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()
    expectEscapeStopsPropagationToWindow()
  })

  it('shows an empty-library state when the folder has nothing in it', async () => {
    render(<LibraryDialog {...baseProps()} />)
    await screen.findByText(/save a selection with/i)
  })

  it('shows an unavailable state when the store reports unavailable (web build)', async () => {
    fx.available = false
    render(<LibraryDialog {...baseProps()} />)
    await screen.findByText(/isn.t available in this browser/i)
  })

  it('shows the Reconnect state when the bound web folder lost permission, and reloads after a successful reconnect', async () => {
    seedDefault()
    fx.needsReconnect = true
    render(<LibraryDialog {...baseProps()} />)
    await screen.findByText(/needs permission again/i)
    fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))
    expect(fx.reconnectCalls).toBe(1)
    // reconnect() cleared the flag — the reload lists the items again.
    await within(grid()).findByText('Theater Chair')
  })

  it('a denied Reconnect explains itself instead of silently no-oping', async () => {
    fx.needsReconnect = true
    fx.reconnectResult = false
    render(<LibraryDialog {...baseProps()} />)
    await screen.findByText(/needs permission again/i)
    fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }))
    await screen.findByText(/denied access/i)
    // Still in the reconnect state — the click did not fake a success.
    expect(screen.getByRole('button', { name: 'Reconnect' })).toBeInTheDocument()
  })

  it('a rejecting list() shows a read error, not a false empty-library invitation', async () => {
    fx.failList = true
    render(<LibraryDialog {...baseProps()} />)
    await screen.findByText(/couldn.t read the library/i)
    expect(screen.queryByText(/save a selection with/i)).toBeNull()
  })

  it('offers Download… in the detail pane when the store reports canDownload (web)', async () => {
    seedDefault()
    fx.canDownload = true
    fx.canReveal = false
    render(<LibraryDialog {...baseProps()} />)
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()
    expect(await screen.findByRole('button', { name: 'Download…' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /reveal in|show in/i })).toBeNull()
  })

  it('shows an empty-search state distinct from the empty-library state', async () => {
    seedDefault()
    render(<LibraryDialog {...baseProps()} />)
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()
    fireEvent.change(screen.getByPlaceholderText(/search name or keyword/i), { target: { value: 'nonexistent-xyz' } })
    await screen.findByText(/no items match your search/i)
  })

  it('deletes an item through the detail pane’s confirm flow', async () => {
    fx.files.set('chair.hew', bytesFor('chair'))
    fx.summaries.set('chair', CHAIR)
    render(<LibraryDialog {...baseProps()} />)
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()

    fireEvent.click(screen.getByRole('button', { name: /delete from library/i }))
    await screen.findByText(/can.t be undone/i)

    fireEvent.click(screen.getByRole('button', { name: /^delete$/i }))

    await waitFor(() => expect(fx.removed).toContain('chair.hew'))
    await waitFor(() => expect(within(grid()).queryByText('Theater Chair')).not.toBeInTheDocument())
  })

  it('cancelling the delete confirm keeps the item', async () => {
    fx.files.set('chair.hew', bytesFor('chair'))
    fx.summaries.set('chair', CHAIR)
    render(<LibraryDialog {...baseProps()} />)
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()

    fireEvent.click(screen.getByRole('button', { name: /delete from library/i }))
    await screen.findByText(/can.t be undone/i)
    fireEvent.click(screen.getByRole('button', { name: /^cancel$/i }))

    expect(screen.queryByText(/can.t be undone/i)).not.toBeInTheDocument()
    expect(within(grid()).getByText('Theater Chair')).toBeInTheDocument()
    expect(fx.removed).not.toContain('chair.hew')
  })

  // --- S15: Enter must not hijack a focused button --------------------------
  it('does not hijack Enter on a focused button into the category-default action', async () => {
    fx.files.set('chair.hew', bytesFor('chair'))
    fx.summaries.set('chair', CHAIR)
    const onInsert = vi.fn()
    render(<LibraryDialog {...baseProps({ onInsert })} />)
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()

    fireEvent.click(within(grid()).getByText('Theater Chair'))
    // Arm the delete confirm — its Cancel button is the concrete case S15
    // calls out: Enter on a focused button must activate the BUTTON, not
    // insert the selected item.
    fireEvent.click(screen.getByRole('button', { name: /delete from library/i }))
    const cancelButton = await screen.findByRole('button', { name: /^cancel$/i })
    cancelButton.focus()
    expect(cancelButton).toHaveFocus()

    fireEvent.keyDown(cancelButton, { key: 'Enter' })

    expect(onInsert).not.toHaveBeenCalled()
    // The delete confirm is untouched by our synthetic Enter (jsdom does not
    // auto-invoke a button's click on Enter the way a real browser does) —
    // what matters here is only that the dialog's own default-action
    // dispatch didn't also fire.
  })

  // --- S16: the dialog must not leak keys to window-level listeners ---------
  it('stops a plain keydown from bubbling to a window-level listener while open', async () => {
    seedDefault()
    render(<LibraryDialog {...baseProps()} />)
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()

    const windowKeyDown = vi.fn()
    window.addEventListener('keydown', windowKeyDown)
    try {
      // A bare letter — the shape of App.tsx's bare-letter tool shortcuts —
      // dispatched with no special target (defaults to document, mirroring a
      // key that never reached a more specific focused element).
      fireEvent.keyDown(document, { key: 'r' })
      expect(windowKeyDown).not.toHaveBeenCalled()
    } finally {
      window.removeEventListener('keydown', windowKeyDown)
    }
  })

  it('stops an arrow key used for grid navigation from bubbling to window too', async () => {
    seedDefault()
    render(<LibraryDialog {...baseProps()} />)
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()

    const windowKeyDown = vi.fn()
    window.addEventListener('keydown', windowKeyDown)
    try {
      fireEvent.keyDown(document, { key: 'ArrowRight' })
      expect(windowKeyDown).not.toHaveBeenCalled()
    } finally {
      window.removeEventListener('keydown', windowKeyDown)
    }
  })

  // --- Arrow-key nav uses the MEASURED column count, not a fixed 3/4 -------
  describe('grid navigation with a measured column count', () => {
    /** Five components — enough to move down two full "rows" at a mocked
     * column count of 2 (item0/1, item2/3, item4). */
    function seedFiveComponents() {
      fx.files.clear()
      fx.summaries.clear()
      fx.thumbnails.clear()
      for (let i = 0; i < 5; i++) {
        const s = summary({
          objects: 1,
          components: 1,
          groups: 0,
          instances: 1,
          doc_attrs: { 'hew.library': { id: `src-${i}`, name: `Item ${i}` } },
        })
        fx.files.set(`item${i}.hew`, bytesFor(`item${i}`))
        fx.summaries.set(`item${i}`, s)
      }
    }

    it('ArrowDown moves by the measured column count (2), not a hardcoded 3', async () => {
      seedFiveComponents()
      gridMeasureFx.columns = 2
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Item 0')
      await awaitDefaultSelection()

      fireEvent.click(within(grid()).getByText('Item 0'))
      fireEvent.keyDown(document, { key: 'ArrowDown' })
      // With 2 measured columns, one row down from index 0 lands on index 2.
      await screen.findByDisplayValue('Item 2')
    })

    it('ArrowUp moves back up by the measured column count', async () => {
      seedFiveComponents()
      gridMeasureFx.columns = 2
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Item 0')
      await awaitDefaultSelection()

      fireEvent.click(within(grid()).getByText('Item 4'))
      fireEvent.keyDown(document, { key: 'ArrowUp' })
      // index 4 - 2 columns = index 2.
      await screen.findByDisplayValue('Item 2')
    })

    it('a different measured column count (e.g. after a resize) changes the stride', async () => {
      seedFiveComponents()
      gridMeasureFx.columns = 4
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Item 0')
      await awaitDefaultSelection()

      fireEvent.click(within(grid()).getByText('Item 0'))
      fireEvent.keyDown(document, { key: 'ArrowDown' })
      // With 4 measured columns, one row down from index 0 lands on index 4
      // (clamped to the last item — there is no index 4... there IS, 5 items
      // means indices 0-4, so this lands exactly on the last one).
      await screen.findByDisplayValue('Item 4')
    })

    it('ArrowRight/ArrowLeft always move by exactly one regardless of column count', async () => {
      seedFiveComponents()
      gridMeasureFx.columns = 2
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Item 0')
      await awaitDefaultSelection()

      fireEvent.click(within(grid()).getByText('Item 0'))
      fireEvent.keyDown(document, { key: 'ArrowRight' })
      await screen.findByDisplayValue('Item 1')

      fireEvent.keyDown(document, { key: 'ArrowLeft' })
      await screen.findByDisplayValue('Item 0')
    })
  })

  // --- S18: manage mutations surface failures instead of swallowing them ----
  it('surfaces a failed rename as a compact error line instead of swallowing it', async () => {
    fx.files.set('chair.hew', bytesFor('chair'))
    fx.summaries.set('chair', CHAIR)
    fx.failWrite = true
    render(<LibraryDialog {...baseProps()} />)
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()

    // Pin the selection to the chair explicitly (auto-select defaults to the
    // first visible tile, which need not be the chair) and wait for its name
    // to land in the draft before typing — the deterministic user flow.
    fireEvent.click(within(grid()).getByText('Theater Chair'))
    await screen.findByDisplayValue('Theater Chair')
    const nameInput = screen.getByLabelText(/item name/i)
    fireEvent.change(nameInput, { target: { value: 'Renamed Chair' } })
    fireEvent.blur(nameInput)

    await screen.findByText(/couldn.t save changes/i, undefined, { timeout: 10000 })
    // The failed write never landed — the store still holds the old bytes.
    expect(markerOf(fx.files.get('chair.hew')!)).toBe('chair')
  }, 15000)

  it('clears a manage-mutation error once the selection changes', async () => {
    fx.files.set('chair.hew', bytesFor('chair'))
    fx.summaries.set('chair', CHAIR)
    fx.files.set('hinge.hew', bytesFor('hinge'))
    fx.summaries.set('hinge', HINGE)
    fx.failWrite = true
    render(<LibraryDialog {...baseProps()} />)
    await within(grid()).findByText('Theater Chair')
    await awaitDefaultSelection()

    // Pin the selection to the chair explicitly (auto-select defaults to the
    // first visible tile, which need not be the chair) and wait for its name
    // to land in the draft before typing — the deterministic user flow.
    fireEvent.click(within(grid()).getByText('Theater Chair'))
    await screen.findByDisplayValue('Theater Chair')
    const nameInput = screen.getByLabelText(/item name/i)
    fireEvent.change(nameInput, { target: { value: 'Renamed Chair' } })
    fireEvent.blur(nameInput)
    await screen.findByText(/couldn.t save changes/i, undefined, { timeout: 10000 })

    fireEvent.click(within(grid()).getByText('Door Hinge'))
    expect(screen.queryByText(/couldn.t save changes/i)).not.toBeInTheDocument()
  }, 15000)

  // --- View toggle: grid/list + localStorage persistence --------------------
  describe('grid/list view toggle', () => {
    it('defaults to grid view and switches to list view on click', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      expect(container.querySelector('.hwlib__grid-inner')).not.toBeNull()
      expect(container.querySelector('.hwlib__list')).toBeNull()

      fireEvent.click(screen.getByRole('button', { name: /list view/i }))
      expect(container.querySelector('.hwlib__list')).not.toBeNull()
      expect(container.querySelector('.hwlib__grid-inner')).toBeNull()
      expect(screen.getByRole('button', { name: /list view/i })).toHaveAttribute('aria-pressed', 'true')
      expect(screen.getByRole('button', { name: /grid view/i })).toHaveAttribute('aria-pressed', 'false')
    })

    it('persists the view choice to localStorage and honors it on the next mount', async () => {
      seedDefault()
      const { unmount } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('button', { name: /list view/i }))
      expect(localStorage.getItem('hew.library.view')).toBe('list')
      unmount()

      render(<LibraryDialog {...baseProps()} />)
      await screen.findByText('Theater Chair')
      expect(document.querySelector('.hwlib__list')).not.toBeNull()
      expect(document.querySelector('.hwlib__grid-inner')).toBeNull()
    })

    it('list rows respond to the same double-click default action as tiles', async () => {
      seedDefault()
      const onInsert = vi.fn()
      render(<LibraryDialog {...baseProps({ onInsert })} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('button', { name: /list view/i }))

      fireEvent.doubleClick(within(grid()).getByText('Theater Chair'))
      expect(onInsert).toHaveBeenCalledOnce()
    })
  })

  // --- Remove Source Info ----------------------------------------------------
  describe('Remove Source Info', () => {
    function seedWithSourceDoc() {
      fx.files.clear()
      fx.summaries.clear()
      fx.thumbnails.clear()
      const s = summary({
        objects: 1,
        components: 1,
        groups: 0,
        instances: 1,
        doc_attrs: {
          'hew.library': { id: 'src-lamp', name: 'Desk Lamp', sourceDoc: 'workshop-project.hew' },
        },
      })
      fx.files.set('lamp.hew', bytesFor('lamp'))
      fx.summaries.set('lamp', s)
    }

    it('shows the source line and Remove Source Info only when sourceDoc is set', async () => {
      seedDefault() // CHAIR has no sourceDoc
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      expect(screen.queryByRole('button', { name: /remove source info/i })).not.toBeInTheDocument()
    })

    it('clears meta.sourceDoc without touching the rest of the item', async () => {
      seedWithSourceDoc()
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Desk Lamp')
      await awaitDefaultSelection()
      expect(screen.getByText(/from workshop-project\.hew/i)).toBeInTheDocument()

      fireEvent.click(screen.getByRole('button', { name: /remove source info/i }))

      await waitFor(() => expect(screen.queryByText(/from workshop-project\.hew/i)).not.toBeInTheDocument())
      expect(screen.queryByRole('button', { name: /remove source info/i })).not.toBeInTheDocument()
      // The name survived the mutation untouched.
      expect(within(grid()).getByText('Desk Lamp')).toBeInTheDocument()
    })
  })

  // --- Creating a nested collection from the detail pane ---------------------
  describe('collection management from the detail pane', () => {
    it('swaps to an inline input on "+ New collection…" and assigns the item on Enter', async () => {
      seedDefault()
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(within(grid()).getByText('Theater Chair'))

      // Selected by its visible option text rather than a hardcoded sentinel
      // value — the exact internal sentinel is an implementation detail of
      // LibraryDetailPane, not something a real user (or this test) sees.
      const select = screen.getByLabelText(/^collection$/i) as HTMLSelectElement
      const newCollectionOption = within(select).getByText('+ New collection…') as HTMLOptionElement
      fireEvent.change(select, { target: { value: newCollectionOption.value } })

      const input = screen.getByLabelText(/new collection path/i)
      fireEvent.change(input, { target: { value: 'Hardware/Fasteners' } })
      fireEvent.keyDown(input, { key: 'Enter' })

      // Back to a dropdown, now showing the newly-assigned nested collection.
      await waitFor(() => {
        const reselect = screen.getByLabelText(/^collection$/i) as HTMLSelectElement
        expect(reselect.value).toBe('Hardware/Fasteners')
      })
    })

    it('Escape cancels the inline input back to the dropdown without assigning anything', async () => {
      seedDefault()
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(within(grid()).getByText('Theater Chair'))

      // Selected by its visible option text rather than a hardcoded sentinel
      // value — the exact internal sentinel is an implementation detail of
      // LibraryDetailPane, not something a real user (or this test) sees.
      const select = screen.getByLabelText(/^collection$/i) as HTMLSelectElement
      const newCollectionOption = within(select).getByText('+ New collection…') as HTMLOptionElement
      fireEvent.change(select, { target: { value: newCollectionOption.value } })
      const input = screen.getByLabelText(/new collection path/i)
      fireEvent.change(input, { target: { value: 'Should Not Stick' } })
      fireEvent.keyDown(input, { key: 'Escape' })

      const reselect = await screen.findByLabelText(/^collection$/i)
      expect((reselect as HTMLSelectElement).value).toBe('')
    })
  })

  // --- Nested collections: sidebar tree + subtree filtering ------------------
  describe('nested collections', () => {
    function seedNestedCollections() {
      fx.files.clear()
      fx.summaries.clear()
      fx.thumbnails.clear()
      const make = (id: string, name: string, collection: string) =>
        summary({
          objects: 1,
          components: 1,
          groups: 0,
          instances: 1,
          doc_attrs: { 'hew.library': { id, name, collection } },
        })
      fx.files.set('bolt.hew', bytesFor('bolt'))
      fx.summaries.set('bolt', make('src-bolt', 'Bolt', 'Hardware/Fasteners'))
      fx.files.set('screw.hew', bytesFor('screw'))
      fx.summaries.set('screw', make('src-screw', 'Screw', 'Hardware/Fasteners'))
      fx.files.set('bracket.hew', bytesFor('bracket'))
      fx.summaries.set('bracket', make('src-bracket', 'Bracket', 'Hardware/Brackets'))
    }

    it('renders a synthesized parent row even though no item is collected directly under it', async () => {
      seedNestedCollections()
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Bolt')
      await awaitDefaultSelection()
      expect(screen.getByRole('button', { name: 'Hardware' })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Fasteners' })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Brackets' })).toBeInTheDocument()
    })

    it('selecting the parent collection filters in its whole subtree', async () => {
      seedNestedCollections()
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Bolt')
      await awaitDefaultSelection()

      fireEvent.click(screen.getByRole('button', { name: 'Hardware' }))
      expect(within(grid()).getByText('Bolt')).toBeInTheDocument()
      expect(within(grid()).getByText('Screw')).toBeInTheDocument()
      expect(within(grid()).getByText('Bracket')).toBeInTheDocument()
    })

    it('selecting a leaf collection excludes its siblings', async () => {
      seedNestedCollections()
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Bolt')
      await awaitDefaultSelection()

      fireEvent.click(screen.getByRole('button', { name: 'Fasteners' }))
      expect(within(grid()).getByText('Bolt')).toBeInTheDocument()
      expect(within(grid()).getByText('Screw')).toBeInTheDocument()
      expect(within(grid()).queryByText('Bracket')).not.toBeInTheDocument()
    })
  })

  // --- Playtest round-3 finding #1/#2: window-variant chrome ---------------
  describe('window variant chrome', () => {
    it('renders no LIBRARY label and no close button — the native titlebar supplies both', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps({ variant: 'window' })} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      expect(container.querySelector('.hwlib__header')).toBeNull()
      expect(container.querySelector('.hwlib__label')).toBeNull()
      expect(screen.queryByRole('button', { name: /^close$/i })).not.toBeInTheDocument()
    })

    it('moves the search box into the sidebar instead of a header', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps({ variant: 'window' })} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      const search = screen.getByPlaceholderText(/search name or keyword/i)
      expect(container.querySelector('.hwlib__sidebar-search')?.contains(search)).toBe(true)
    })

    it('the modal variant keeps its header (label + search + close)', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      expect(container.querySelector('.hwlib__header .hwlib__label')).not.toBeNull()
      expect(container.querySelector('.hwlib__header .hwlib__search')).not.toBeNull()
      expect(screen.getByRole('button', { name: /^close$/i })).toBeInTheDocument()
    })

    it('both variants share the scope-chip/view-toggle bar above the center viewport, not inside the header', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      expect(container.querySelector('.hwlib__header .hwlib__scopes')).toBeNull()
      expect(container.querySelector('.hwlib__center-bar .hwlib__scopes')).not.toBeNull()
      expect(container.querySelector('.hwlib__center-bar .hwlib__view-toggle')).not.toBeNull()
    })
  })

  // --- Playtest round-3 finding #5: native detail buttons + delete confirm --
  describe('window variant: native detail buttons and delete confirm', () => {
    function seedChair() {
      fx.files.set('chair.hew', bytesFor('chair'))
      fx.summaries.set('chair', CHAIR)
    }

    it('renders the primary/secondary/manage buttons unstyled instead of the custom classes', async () => {
      seedChair()
      const { container } = render(<LibraryDialog {...baseProps({ variant: 'window' })} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      expect(container.querySelector('.hwlib__actions .hwlib__btn-primary')).toBeNull()
      expect(container.querySelector('.hwlib__actions .hwlib__native-btn')).not.toBeNull()
      expect(container.querySelector('.hwlib__manage-row .hwlib__btn-danger')).toBeNull()
      expect(container.querySelector('.hwlib__manage-row .hwlib__native-btn')).not.toBeNull()
    })

    it('the modal variant keeps the custom-styled buttons', async () => {
      seedChair()
      const { container } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      expect(container.querySelector('.hwlib__actions .hwlib__btn-primary')).not.toBeNull()
      expect(container.querySelector('.hwlib__actions .hwlib__native-btn')).toBeNull()
    })

    it('Delete from the detail pane goes through the native confirm, not the inline armed confirm', async () => {
      seedChair()
      render(<LibraryDialog {...baseProps({ variant: 'window' })} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()

      fireEvent.click(screen.getByRole('button', { name: /delete from library/i }))
      await waitFor(() => expect(nativeChromeFx.confirmCalls).toHaveLength(1))
      expect(nativeChromeFx.confirmCalls[0].title).toBe('Delete from Library')
      // The inline armed-confirm UI (the modal/web fallback) never appears.
      expect(screen.queryByText(/can.t be undone/i)).not.toBeInTheDocument()

      await waitFor(() => expect(fx.removed).toContain('chair.hew'))
    })

    it('cancelling the native confirm leaves the item in place', async () => {
      seedChair()
      nativeChromeFx.confirmResult = false
      render(<LibraryDialog {...baseProps({ variant: 'window' })} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()

      fireEvent.click(screen.getByRole('button', { name: /delete from library/i }))
      await waitFor(() => expect(nativeChromeFx.confirmCalls).toHaveLength(1))
      expect(fx.removed).not.toContain('chair.hew')
      expect(within(grid()).getByText('Theater Chair')).toBeInTheDocument()
    })

    it('the modal variant still uses the inline armed confirm (unchanged fallback)', async () => {
      seedChair()
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()

      fireEvent.click(screen.getByRole('button', { name: /delete from library/i }))
      await screen.findByText(/can.t be undone/i)
      expect(nativeChromeFx.confirmCalls).toHaveLength(0)
    })
  })

  // --- Playtest round-3 finding #3: List view Name/Type/Size ----------------
  describe('list view: Name/Type/Size columns', () => {
    it('shows exactly three sortable column headers: Name, Type, Size', async () => {
      seedDefault()
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('button', { name: /list view/i }))

      const headers = screen.getAllByRole('columnheader')
      expect(headers.map((h) => h.textContent?.replace(/[▲▼]/, '').trim())).toEqual(['Name', 'Type', 'Size'])
    })

    it('defaults to Name ascending, and shows Type/Size instead of a solids/materials count', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('button', { name: /list view/i }))

      expect(screen.getAllByRole('columnheader', { name: /^name/i })[0]).toHaveAttribute('aria-sort', 'ascending')
      const rows = within(grid()).getAllByRole('row').filter((r) => r.hasAttribute('data-filename'))
      expect(rows[0]).toHaveTextContent('Door Hinge') // "Door Hinge" < "Theater Chair"
      expect(container.querySelector('.hwlib__row-cell--type')).toHaveTextContent('Component')
      expect(container.querySelector('.hwlib__row-meta')).toBeNull() // no leftover solids/materials line
    })

    it('clicking a header sorts by it; clicking the SAME header again reverses direction', async () => {
      seedDefault()
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('button', { name: /list view/i }))

      fireEvent.click(screen.getByRole('button', { name: /^name/i }))
      expect(within(grid()).getAllByRole('row').filter((r) => r.hasAttribute('data-filename'))[0]).toHaveTextContent('Theater Chair')
      expect(screen.getAllByRole('columnheader', { name: /^name/i })[0]).toHaveAttribute('aria-sort', 'descending')

      fireEvent.click(screen.getByRole('button', { name: /^name/i }))
      expect(within(grid()).getAllByRole('row').filter((r) => r.hasAttribute('data-filename'))[0]).toHaveTextContent('Door Hinge')
      expect(screen.getAllByRole('columnheader', { name: /^name/i })[0]).toHaveAttribute('aria-sort', 'ascending')
    })

    it('switching to a different column makes IT the sole active sort, defaulting to ascending', async () => {
      seedDefault()
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('button', { name: /list view/i }))

      fireEvent.click(screen.getByRole('button', { name: /^type/i }))
      expect(screen.getAllByRole('columnheader', { name: /^type/i })[0]).toHaveAttribute('aria-sort', 'ascending')
      expect(screen.getAllByRole('columnheader', { name: /^name/i })[0]).toHaveAttribute('aria-sort', 'none')
    })

    it('the sort composes with the active search filter', async () => {
      seedDefault()
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('button', { name: /list view/i }))
      fireEvent.change(screen.getByPlaceholderText(/search name or keyword/i), { target: { value: 'chair' } })

      const rows = within(grid()).getAllByRole('row').filter((r) => r.hasAttribute('data-filename'))
      expect(rows).toHaveLength(1)
      expect(rows[0]).toHaveTextContent('Theater Chair')
    })

    // The drag itself (pointerdown/pointermove/pointerup) can't be simulated
    // here — jsdom has no `PointerEvent` implementation — so the resize
    // MATH (`nextColumnWidth`) is unit-tested directly in
    // `library/LibraryListHead.test.ts`; these two only cover the wiring
    // around it: initial widths load from storage, and which columns even
    // offer a divider.
    it('honors a persisted column width from localStorage on mount', async () => {
      localStorage.setItem('hew.library.listCols', JSON.stringify({ name: 300, type: 120 }))
      seedDefault()
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('button', { name: /list view/i }))

      const [nameHeader] = screen.getAllByRole('columnheader', { name: /^name/i })
      const [typeHeader] = screen.getAllByRole('columnheader', { name: /^type/i })
      expect(nameHeader.style.width).toBe('300px')
      expect(typeHeader.style.width).toBe('120px')
    })

    it('renders a resize divider for Name and Type, but not for the trailing Size column', async () => {
      seedDefault()
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('button', { name: /list view/i }))

      expect(screen.getByRole('separator', { name: /resize name column/i })).toBeInTheDocument()
      expect(screen.getByRole('separator', { name: /resize type column/i })).toBeInTheDocument()
      expect(screen.queryByRole('separator', { name: /resize size column/i })).not.toBeInTheDocument()
    })

    // Finding #3 (find it = fix it): LibraryListHead had the exact same
    // Escape-mid-drag gap as the body-column resize below — its window
    // listeners were only ever removed by its own `onUp`, never by an
    // unmount. Closing the dialog mid-drag unmounts this component for
    // real (it's only ever rendered while `view === 'list'` inside the
    // dialog's own `if (!open) return null`), so a plain unmount cleanup
    // is the fix there — this proves it via the same real-drag dispatch
    // trick the body-column tests below use (see their describe block's
    // comment for why a bare `MouseEvent` typed 'pointerdown'/'pointermove'
    // works despite jsdom having no `PointerEvent` constructor).
    it('Escape mid-drag tears down the List view resize listeners — a later pointermove no longer mutates the persisted width', async () => {
      seedDefault()
      const { rerender } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('button', { name: /list view/i }))
      const separator = screen.getByRole('separator', { name: /resize name column/i })

      fireEvent(separator, new MouseEvent('pointerdown', { bubbles: true, cancelable: true, clientX: 100 }))
      fireEvent(window, new MouseEvent('pointermove', { bubbles: true, cancelable: true, clientX: 140 }))
      const [nameHeader] = screen.getAllByRole('columnheader', { name: /^name/i })
      expect(nameHeader.style.width).toBe('260px')

      // Close the dialog — the same `open` transition Escape drives — mid-
      // drag, without ever firing a pointerup.
      rerender(<LibraryDialog {...baseProps({ open: false })} />)

      // A stray pointermove after the close must be a no-op.
      fireEvent(window, new MouseEvent('pointermove', { bubbles: true, cancelable: true, clientX: 400 }))
      expect(loadListColWidths().name).toBe(260)
    })
  })

  // --- Playtest round-4 finding #1: resizable sidebar/detail columns --------
  describe('body column resize', () => {
    // jsdom has no `PointerEvent` CONSTRUCTOR (`window.PointerEvent` is
    // undefined — confirmed directly against this repo's jsdom version), so
    // `fireEvent.pointerDown`/etc. would fall back to a bare `Event` with no
    // `clientX` at all. But `beginColResize`'s own listeners only care about
    // the EVENT TYPE STRING ('pointerdown'/'pointermove'/'pointerup') and
    // `ev.clientX` — both of which a plain `MouseEvent` constructed with
    // that type string supplies just fine, dispatched directly rather than
    // through the `fireEvent.pointerDown` alias. That's what the direction
    // tests below do to drive the REAL drag path (pointerdown on the
    // separator, pointermove/pointerup on `window`, exactly where
    // `beginColResize` itself listens) — including the divider-direction
    // sign flip, which is dialog-specific wiring `nextColumnWidth`'s own
    // unit tests (`library/LibraryListHead.test.ts`) never exercise, since
    // that module has no divider to flip. `loadColWidths`/`persistColWidths`
    // (this module's own storage round-trip) are covered directly below.
    it('honors a persisted sidebar/detail width from localStorage on mount', async () => {
      persistColWidths({ sidebar: 210, detail: 260 })
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()

      const sidebar = container.querySelector('.hwlib__sidebar') as HTMLElement
      const detail = container.querySelector('.hwlib__detail') as HTMLElement
      expect(sidebar.style.width).toBe('210px')
      expect(detail.style.width).toBe('260px')
    })

    it('falls back to the built-in defaults when nothing is stored', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()

      const sidebar = container.querySelector('.hwlib__sidebar') as HTMLElement
      const detail = container.querySelector('.hwlib__detail') as HTMLElement
      expect(sidebar.style.width).toBe('168px')
      expect(detail.style.width).toBe('236px')
    })

    it('renders a resize divider between the sidebar and the center column, and between the center column and the detail pane', async () => {
      seedDefault()
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()

      expect(screen.getByRole('separator', { name: /resize sidebar/i })).toBeInTheDocument()
      expect(screen.getByRole('separator', { name: /resize detail pane/i })).toBeInTheDocument()
    })

    // Finding #4: the divider-direction math (`beginColResize`'s
    // `deltaX = drag.column === 'sidebar' ? rawDelta : -rawDelta`) had zero
    // coverage — these drive the real drag path (see the describe block's
    // own comment above) and assert BOTH dividers' direction explicitly.
    it('dragging the sidebar divider right widens the sidebar', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      const sidebar = container.querySelector('.hwlib__sidebar') as HTMLElement
      const separator = screen.getByRole('separator', { name: /resize sidebar/i })
      expect(sidebar.style.width).toBe('168px')

      fireEvent(separator, new MouseEvent('pointerdown', { bubbles: true, cancelable: true, clientX: 100 }))
      fireEvent(window, new MouseEvent('pointermove', { bubbles: true, cancelable: true, clientX: 140 }))
      fireEvent(window, new MouseEvent('pointerup', { bubbles: true, cancelable: true }))

      expect(sidebar.style.width).toBe('208px')
    })

    it('dragging the sidebar divider left narrows the sidebar', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      const sidebar = container.querySelector('.hwlib__sidebar') as HTMLElement
      const separator = screen.getByRole('separator', { name: /resize sidebar/i })

      // -4px, comfortably above COL_WIDTH_MIN.sidebar (160) so this shows
      // real narrowing rather than the floor clamp (already covered by
      // `nextColumnWidth`'s own unit tests).
      fireEvent(separator, new MouseEvent('pointerdown', { bubbles: true, cancelable: true, clientX: 100 }))
      fireEvent(window, new MouseEvent('pointermove', { bubbles: true, cancelable: true, clientX: 96 }))
      fireEvent(window, new MouseEvent('pointerup', { bubbles: true, cancelable: true }))

      expect(sidebar.style.width).toBe('164px')
    })

    it('dragging the detail divider left widens the detail pane (its drag sign is inverted — the divider sits on the pane’s own LEFT edge)', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      const detail = container.querySelector('.hwlib__detail') as HTMLElement
      const separator = screen.getByRole('separator', { name: /resize detail pane/i })
      expect(detail.style.width).toBe('236px')

      fireEvent(separator, new MouseEvent('pointerdown', { bubbles: true, cancelable: true, clientX: 500 }))
      fireEvent(window, new MouseEvent('pointermove', { bubbles: true, cancelable: true, clientX: 460 }))
      fireEvent(window, new MouseEvent('pointerup', { bubbles: true, cancelable: true }))

      expect(detail.style.width).toBe('276px')
    })

    it('dragging the detail divider right narrows the detail pane', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      const detail = container.querySelector('.hwlib__detail') as HTMLElement
      const separator = screen.getByRole('separator', { name: /resize detail pane/i })

      // +10px, comfortably above COL_WIDTH_MIN.detail (220 — a 40px drag
      // here would floor at the minimum, already covered by
      // `nextColumnWidth`'s own unit tests) so this shows real narrowing.
      fireEvent(separator, new MouseEvent('pointerdown', { bubbles: true, cancelable: true, clientX: 500 }))
      fireEvent(window, new MouseEvent('pointermove', { bubbles: true, cancelable: true, clientX: 510 }))
      fireEvent(window, new MouseEvent('pointerup', { bubbles: true, cancelable: true }))

      expect(detail.style.width).toBe('226px')
    })

    it('stops mutating widths after pointerup — the window listeners are torn down', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      const sidebar = container.querySelector('.hwlib__sidebar') as HTMLElement
      const separator = screen.getByRole('separator', { name: /resize sidebar/i })

      fireEvent(separator, new MouseEvent('pointerdown', { bubbles: true, cancelable: true, clientX: 100 }))
      fireEvent(window, new MouseEvent('pointermove', { bubbles: true, cancelable: true, clientX: 140 }))
      fireEvent(window, new MouseEvent('pointerup', { bubbles: true, cancelable: true }))
      expect(sidebar.style.width).toBe('208px')

      // A stray pointermove after pointerup must be a no-op — the listeners
      // were removed in `onUp`.
      fireEvent(window, new MouseEvent('pointermove', { bubbles: true, cancelable: true, clientX: 300 }))
      expect(sidebar.style.width).toBe('208px')
    })

    // Finding #3: Escape closing the dialog mid-drag must tear down the
    // still-live window listeners, or bare mouse movement afterward keeps
    // mutating (and persisting) `colWidths` invisibly.
    it('Escape mid-drag tears down the resize listeners — a later pointermove no longer mutates the persisted width', async () => {
      seedDefault()
      const { container, rerender } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      const sidebar = container.querySelector('.hwlib__sidebar') as HTMLElement
      const separator = screen.getByRole('separator', { name: /resize sidebar/i })

      fireEvent(separator, new MouseEvent('pointerdown', { bubbles: true, cancelable: true, clientX: 100 }))
      fireEvent(window, new MouseEvent('pointermove', { bubbles: true, cancelable: true, clientX: 140 }))
      expect(sidebar.style.width).toBe('208px')

      // Close the dialog the same way Escape does — via the `open` prop —
      // without ever firing a pointerup.
      rerender(<LibraryDialog {...baseProps({ open: false })} />)

      // A stray pointermove after the close must be a no-op: the listeners
      // were torn down by the `open`-keyed effect, not left dangling.
      fireEvent(window, new MouseEvent('pointermove', { bubbles: true, cancelable: true, clientX: 400 }))
      expect(loadColWidths().sidebar).toBe(208)

      rerender(<LibraryDialog {...baseProps({ open: true })} />)
      const reopenedSidebar = container.querySelector('.hwlib__sidebar') as HTMLElement
      expect(reopenedSidebar.style.width).toBe('208px')
    })
  })

  describe('loadColWidths / persistColWidths', () => {
    it('falls back to the built-in defaults when nothing is stored', () => {
      expect(loadColWidths()).toEqual({ sidebar: 168, detail: 236 })
    })

    it('round-trips a persisted value', () => {
      persistColWidths({ sidebar: 200, detail: 300 })
      expect(loadColWidths()).toEqual({ sidebar: 200, detail: 300 })
    })

    it('treats a stored width below the current minimum as absent, not trusted verbatim', () => {
      localStorage.setItem('hew.library.colWidths', JSON.stringify({ sidebar: 10, detail: 5 }))
      expect(loadColWidths()).toEqual({ sidebar: 168, detail: 236 })
      expect(COL_WIDTH_MIN.sidebar).toBeGreaterThan(10)
      expect(COL_WIDTH_MIN.detail).toBeGreaterThan(5)
    })

    it('degrades to the default on malformed JSON rather than throwing', () => {
      localStorage.setItem('hew.library.colWidths', 'not json')
      expect(loadColWidths()).toEqual({ sidebar: 168, detail: 236 })
    })

    // Finding #1c: a hand-edited (or otherwise corrupted) localStorage value
    // large enough to clip the center column and both resize dividers must
    // be capped, not trusted verbatim — this is the belt-and-suspenders
    // layer under the CSS `max-width: 40%` on `.hwlib__sidebar`/
    // `.hwlib__detail` and the live drag clamp (`clampColWidthLive`,
    // covered separately below).
    it('caps an oversized stored value at the hard maximum instead of trusting it verbatim', () => {
      localStorage.setItem('hew.library.colWidths', JSON.stringify({ sidebar: 5000, detail: 9999 }))
      expect(loadColWidths()).toEqual({ sidebar: COL_WIDTH_MAX.sidebar, detail: COL_WIDTH_MAX.detail })
    })
  })

  // Finding #1b: the live drag clamp is pure and exported (same convention
  // as `nextColumnWidth`) — unit-tested directly on plain numbers, since
  // jsdom can't produce a real layout for `beginColResize`'s own
  // `getBoundingClientRect` read to measure.
  describe('clampColWidthLive', () => {
    it('passes a desired width through unchanged when the container fits it', () => {
      expect(clampColWidthLive(300, 160, 236, 920, 10, 200)).toBe(300)
    })

    it('caps the desired width so the center column never drops below centerMin', () => {
      // container 920, other column 236, dividers 10, centerMin 200:
      // max = 920 - 236 - 10 - 200 = 474.
      expect(clampColWidthLive(600, 160, 236, 920, 10, 200)).toBe(474)
    })

    it('never returns less than the column minimum, even if that violates centerMin', () => {
      // A tiny container where centerMin can't possibly be honored still
      // must not shrink the column below its own floor.
      expect(clampColWidthLive(300, 160, 236, 300, 10, 200)).toBe(160)
    })

    it('disables the live clamp when the container has no measurable width (containerWidth <= 0)', () => {
      // jsdom's no-op layout engine — every element measures 0 — must
      // degrade to plain min-flooring rather than collapsing the column.
      expect(clampColWidthLive(9999, 160, 236, 0, 10, 200)).toBe(9999)
    })
  })

  // --- Playtest round-3 finding #4: the "All" category ----------------------
  describe('the "All" category', () => {
    it('lists every category together, with its count summing every item', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()

      fireEvent.click(screen.getByRole('tab', { name: ALL_TAB }))
      await within(grid()).findByText('Oak')
      await awaitDefaultSelection()
      expect(within(grid()).getByText('Door Hinge')).toBeInTheDocument()
      expect(within(grid()).getByText('Little House')).toBeInTheDocument()

      const allCount = container.querySelector('.hwlib__cat-row--active .hwlib__cat-count')
      // 2 components + 1 material + 2 models (incl. the errored "broken" fixture).
      expect(allCount?.textContent).toBe('5')
    })

    it("Enter/double-click always runs the SELECTED ITEM's own category action, never a fixed one", async () => {
      seedDefault()
      const onPaintWith = vi.fn()
      const onInsert = vi.fn()
      render(<LibraryDialog {...baseProps({ onPaintWith, onInsert })} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('tab', { name: ALL_TAB }))
      await within(grid()).findByText('Oak')
      await awaitDefaultSelection()

      fireEvent.click(within(grid()).getByText('Oak'))
      fireEvent.keyDown(document, { key: 'Enter' })
      expect(onPaintWith).toHaveBeenCalledOnce() // the material's own default action
      expect(onInsert).not.toHaveBeenCalled()

      fireEvent.click(within(grid()).getByText('Theater Chair'))
      fireEvent.keyDown(document, { key: 'Enter' })
      expect(onInsert).toHaveBeenCalledOnce() // the component's own default action
    })

    it('the grid track uses the larger (component/model) size so material swatches never squeeze thumbnail tiles', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('tab', { name: ALL_TAB }))
      await within(grid()).findByText('Oak')
      await awaitDefaultSelection()
      expect(container.querySelector('.hwlib__grid-inner--all')).not.toBeNull()
    })
  })

  // --- Playtest round-4 finding #2: materials match Components' "in this
  // model" treatment (blue dot, not a green pill; matched by the "In this
  // model" scope filter too) --------------------------------------------
  describe('materials match Components’ "in this model" treatment', () => {
    function inPaletteWhenOak(bytes: Uint8Array): Promise<boolean> {
      return Promise.resolve(markerOf(bytes) === 'oak')
    }

    // Finding #6: seeds a SECOND material that `inPaletteWhenOak` resolves
    // `false` for, alongside Oak (which it resolves `true` for) — Oak's dot
    // appearing is the signal that the async check has actually settled
    // (same convention `waitFor` already relies on above), so Pine's
    // absence can be asserted with confidence rather than just catching the
    // dot-less INITIAL render before the check ever runs. Every prior test
    // in this file only ever exercised the true case; this is what actually
    // exercises `isMaterial && inPalette` (LibraryTile.tsx/LibraryListRow.tsx) —
    // a mutant that dropped the `isMaterial &&` gate, or unconditionally
    // read `inPalette` as true, had nothing here to catch it.
    function seedSecondMaterialNotInPalette() {
      fx.files.set('pine.hew', bytesFor('pine'))
      fx.summaries.set(
        'pine',
        summary({
          materials: 1,
          material_entries: [
            {
              name: 'Pine',
              color: [200, 180, 140, 255],
              texture_asset: null,
              texture_format: null,
              texture_world_size: null,
              content_hash: '5678',
            },
          ],
          doc_attrs: { 'hew.library': { id: 'src-pine', name: 'Pine' } },
        }),
      )
    }

    it('shows a blue dot, never the old green pill, on a material tile already in the palette', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps({ materialInPalette: inPaletteWhenOak })} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('tab', { name: /materials/i }))
      await within(grid()).findByText('Oak')
      await awaitDefaultSelection()

      await waitFor(() => expect(container.querySelector('.hwlib__tile-dot')).not.toBeNull())
      expect(container.querySelector('.hwlib__tile-palette-pill')).toBeNull()
    })

    it('shows a blue dot, never the old green pill, on a material row in list view', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps({ materialInPalette: inPaletteWhenOak })} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('tab', { name: /materials/i }))
      await within(grid()).findByText('Oak')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('button', { name: /list view/i }))

      await waitFor(() => expect(container.querySelector('.hwlib__row-dot')).not.toBeNull())
      expect(container.querySelector('.hwlib__row-palette-pill')).toBeNull()
    })

    it('shows no blue dot on a material tile NOT in the palette, even while a sibling material has one', async () => {
      seedDefault()
      seedSecondMaterialNotInPalette()
      const { container } = render(<LibraryDialog {...baseProps({ materialInPalette: inPaletteWhenOak })} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('tab', { name: /materials/i }))
      await within(grid()).findByText('Pine')
      await awaitDefaultSelection()

      // Oak's dot appearing is the signal the async check has settled for
      // every item (one `Promise.all`, one `setInPaletteMap`) — only then
      // is Pine's absence a real assertion, not a race against the
      // dot-less initial render.
      const oakTile = container.querySelector('[data-filename="oak.hew"]') as HTMLElement
      await waitFor(() => expect(oakTile.querySelector('.hwlib__tile-dot')).not.toBeNull())

      const pineTile = container.querySelector('[data-filename="pine.hew"]') as HTMLElement
      expect(pineTile.querySelector('.hwlib__tile-dot')).toBeNull()
    })

    it('shows no blue dot on a material row NOT in the palette, even while a sibling material has one', async () => {
      seedDefault()
      seedSecondMaterialNotInPalette()
      const { container } = render(<LibraryDialog {...baseProps({ materialInPalette: inPaletteWhenOak })} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('tab', { name: /materials/i }))
      await within(grid()).findByText('Pine')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('button', { name: /list view/i }))

      const oakRow = container.querySelector('[data-filename="oak.hew"]') as HTMLElement
      await waitFor(() => expect(oakRow.querySelector('.hwlib__row-dot')).not.toBeNull())

      const pineRow = container.querySelector('[data-filename="pine.hew"]') as HTMLElement
      expect(pineRow.querySelector('.hwlib__row-dot')).toBeNull()
    })

    // Finding #2: the modal variant's App.tsx-supplied `materialInPalette`
    // used to be a permanently-stable `useCallback` identity (`[]` deps) —
    // "Add to palette" (or an undo/redo that added/removed a palette entry)
    // never refreshed the dot/filter until the dialog was closed and
    // reopened, even though components' own placement-count parity updated
    // live in the same flow. The fix (App.tsx) keys that `useCallback` off
    // `docRev` instead, which bumps on every palette mutation and every
    // undo/redo — giving it a NEW identity exactly when a refresh is
    // needed. This dialog-level test proves the mechanism that fix relies
    // on: a changed `materialInPalette` identity (this component's own
    // `[open, items, materialInPalette]` effect deps) refreshes the dot AND
    // the scope filter WITHOUT the dialog ever closing — the same live
    // parity the window variant already gets for free from its inline
    // lambda's fresh identity on every render.
    it('refreshes the dot and the "in this model" scope filter when materialInPalette gets a new identity, without closing the dialog', async () => {
      seedDefault()
      const { container, rerender } = render(<LibraryDialog {...baseProps({ materialInPalette: async () => false })} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('tab', { name: /materials/i }))
      await within(grid()).findByText('Oak')
      await awaitDefaultSelection()
      const oakTile = container.querySelector('[data-filename="oak.hew"]') as HTMLElement
      await waitFor(() => expect(oakTile.querySelector('.hwlib__tile-dot')).toBeNull())

      // A brand-new function identity (never `[]`-memoized) — the same
      // shape App.tsx's `libraryMaterialInPalette` now takes on across a
      // `docRev` bump.
      rerender(<LibraryDialog {...baseProps({ materialInPalette: async () => true })} />)

      await waitFor(() => expect(oakTile.querySelector('.hwlib__tile-dot')).not.toBeNull())

      fireEvent.click(screen.getByRole('button', { name: /in this model/i }))
      await waitFor(() => expect(within(grid()).getByText('Oak')).toBeInTheDocument())
    })

    it('scope "In this model" includes a material already in the document palette', async () => {
      seedDefault()
      render(<LibraryDialog {...baseProps({ materialInPalette: inPaletteWhenOak })} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('tab', { name: /materials/i }))
      await within(grid()).findByText('Oak')
      await awaitDefaultSelection()

      fireEvent.click(screen.getByRole('button', { name: /in this model/i }))
      await waitFor(() => expect(within(grid()).getByText('Oak')).toBeInTheDocument())
    })

    it('scope "In this model" excludes a material NOT in the document palette', async () => {
      seedDefault()
      render(<LibraryDialog {...baseProps({ materialInPalette: async () => false })} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('tab', { name: /materials/i }))
      await within(grid()).findByText('Oak')
      await awaitDefaultSelection()

      fireEvent.click(screen.getByRole('button', { name: /in this model/i }))
      await waitFor(() => expect(within(grid()).queryByText('Oak')).not.toBeInTheDocument())
    })

    it('detail pane: the primary action reads "Paint", and "Open as document" is a real button, not a link', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('tab', { name: /materials/i }))
      await within(grid()).findByText('Oak')
      await awaitDefaultSelection()
      fireEvent.click(within(grid()).getByText('Oak'))

      // Scoped to the detail pane's action row: the grid tile ALSO has its
      // own hover "Paint" button (the tile's default-action affordance) —
      // querying the whole document would match both.
      const actions = within(container.querySelector('.hwlib__actions') as HTMLElement)
      expect(actions.getByRole('button', { name: /^paint$/i })).toBeInTheDocument()
      expect(actions.queryByRole('button', { name: /paint with this/i })).not.toBeInTheDocument()
      const openAsDoc = actions.getByRole('button', { name: /^open as document$/i })
      expect(openAsDoc.className).not.toMatch(/btn-link/)
    })

    it('detail pane: shows "In this model" (not the old palette wording) for a material already in the palette', async () => {
      seedDefault()
      const { container } = render(<LibraryDialog {...baseProps({ materialInPalette: inPaletteWhenOak })} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(screen.getByRole('tab', { name: /materials/i }))
      await within(grid()).findByText('Oak')
      await awaitDefaultSelection()
      fireEvent.click(within(grid()).getByText('Oak'))

      // `.hwlib__inmodel`, not a bare text query — the "In this model" scope
      // chip in the center bar carries the exact same label.
      await waitFor(() => expect(container.querySelector('.hwlib__inmodel')).toHaveTextContent('In this model'))
      expect(screen.queryByText(/already in this document's palette/i)).not.toBeInTheDocument()
    })
  })

  // --- Playtest round-4 finding #3: "None" instead of "Uncollected" --------
  describe('the "None" collection label', () => {
    it('the detail pane’s Collection dropdown shows "None" for an item with no collection', async () => {
      seedDefault()
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Theater Chair')
      await awaitDefaultSelection()
      fireEvent.click(within(grid()).getByText('Theater Chair'))

      const select = screen.getByLabelText(/^collection$/i)
      expect(within(select).getByText('None')).toBeInTheDocument()
      expect(within(select).queryByText('Uncollected')).not.toBeInTheDocument()
    })
  })

  // --- Playtest round-4 finding #4: Models have no Source Info -------------
  describe('Models have no Source Info', () => {
    function seedModelWithSourceDoc() {
      fx.files.clear()
      fx.summaries.clear()
      fx.thumbnails.clear()
      const s = summary({
        objects: 5,
        groups: 2,
        materials: 1,
        doc_attrs: {
          'hew.library': { id: 'src-shed', name: 'Garden Shed', sourceDoc: 'workshop-project.hew' },
        },
      })
      fx.files.set('shed.hew', bytesFor('shed'))
      fx.summaries.set('shed', s)
    }

    it('does not show the Source Info row in the detail pane for a model, even when sourceDoc is set', async () => {
      seedModelWithSourceDoc()
      render(<LibraryDialog {...baseProps()} />)
      // The shed is a Model — the sidebar defaults to the Components
      // category, which doesn't list it.
      fireEvent.click(screen.getByRole('tab', { name: /models/i }))
      await within(grid()).findByText('Garden Shed')
      await awaitDefaultSelection()
      fireEvent.click(within(grid()).getByText('Garden Shed'))

      expect(screen.queryByText(/from workshop-project\.hew/i)).not.toBeInTheDocument()
      expect(screen.queryByRole('button', { name: /remove source info/i })).not.toBeInTheDocument()
    })

    it('does not offer "Remove Source Info" in the ⋯ menu for a model', async () => {
      seedModelWithSourceDoc()
      render(<LibraryDialog {...baseProps()} />)
      fireEvent.click(screen.getByRole('tab', { name: /models/i }))
      await within(grid()).findByText('Garden Shed')
      await awaitDefaultSelection()
      fireEvent.click(within(grid()).getByRole('button', { name: /garden shed actions/i }))

      const menu = screen.getByRole('menu', { name: /garden shed actions/i })
      expect(within(menu).queryByRole('button', { name: /remove source info/i })).not.toBeInTheDocument()
    })

    // Finding #5: the window variant's ⋯ menu (`openNativeMenu`,
    // LibraryDialog.tsx) builds its own separate `entries` array rather
    // than reusing `LibraryMenu` — the modal test above never exercises
    // that path at all. Uses the harness already established for the
    // native-detail-buttons/delete-confirm tests above (`nativeChromeFx`).
    it('does not offer "Remove Source Info" in the native ⋯ menu for a model', async () => {
      seedModelWithSourceDoc()
      render(<LibraryDialog {...baseProps({ variant: 'window' })} />)
      fireEvent.click(screen.getByRole('tab', { name: /models/i }))
      await within(grid()).findByText('Garden Shed')
      await awaitDefaultSelection()
      fireEvent.click(within(grid()).getByRole('button', { name: /garden shed actions/i }))

      await waitFor(() => expect(nativeChromeFx.menuEntryCalls).toHaveLength(1))
      const labels = nativeChromeFx.menuEntryCalls[0].map((entry) => entry.label)
      expect(labels).not.toContain('Remove Source Info')
    })

    it('offers "Remove Source Info" in the native ⋯ menu for a component with sourceDoc set', async () => {
      fx.files.clear()
      fx.summaries.clear()
      fx.thumbnails.clear()
      fx.files.set('lamp.hew', bytesFor('lamp'))
      fx.summaries.set(
        'lamp',
        summary({
          objects: 1,
          components: 1,
          groups: 0,
          instances: 1,
          doc_attrs: { 'hew.library': { id: 'src-lamp', name: 'Desk Lamp', sourceDoc: 'workshop-project.hew' } },
        }),
      )
      render(<LibraryDialog {...baseProps({ variant: 'window' })} />)
      await within(grid()).findByText('Desk Lamp')
      await awaitDefaultSelection()
      fireEvent.click(within(grid()).getByRole('button', { name: /desk lamp actions/i }))

      await waitFor(() => expect(nativeChromeFx.menuEntryCalls).toHaveLength(1))
      const labels = nativeChromeFx.menuEntryCalls[0].map((entry) => entry.label)
      expect(labels).toContain('Remove Source Info')
    })

    it('still shows Source Info for a component with sourceDoc set (unchanged)', async () => {
      fx.files.clear()
      fx.summaries.clear()
      fx.thumbnails.clear()
      const s = summary({
        objects: 1,
        components: 1,
        groups: 0,
        instances: 1,
        doc_attrs: { 'hew.library': { id: 'src-lamp', name: 'Desk Lamp', sourceDoc: 'workshop-project.hew' } },
      })
      fx.files.set('lamp.hew', bytesFor('lamp'))
      fx.summaries.set('lamp', s)
      render(<LibraryDialog {...baseProps()} />)
      await within(grid()).findByText('Desk Lamp')
      await awaitDefaultSelection()
      expect(screen.getByText(/from workshop-project\.hew/i)).toBeInTheDocument()
      expect(screen.getByRole('button', { name: /remove source info/i })).toBeInTheDocument()
    })
  })
})
