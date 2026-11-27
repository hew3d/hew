/* oxlint-disable lingui/no-unlocalized-strings -- test fixtures */
/**
 * Review round: UI and wiring fixes for sources (findings F19-F23, needsCheck).
 * the real sources module over the fake index server and fake Library store of
 * `sourcesTestKit`, drawn through the real dialog. No network anywhere: the
 * fetcher is the kit's, and every request is recorded.
 */

import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { LibraryDialog, type LibraryDialogProps } from './LibraryDialog'
import type { SourcesBinding } from '../library/sourcesDeps'
import {
  bump,
  itemBytes,
  makeEnv,
  noteOf,
  parseFake,
  parts,
  SRC,
  SRC2,
  uuid,
  type Env,
  type FakeStore,
} from '../library/sourcesTestKit'
import type { CatalogSourceEntry } from '../library/sources'

// ---------------------------------------------------------------------------
// Mocks: the Library store is the kit's fake; item reading parses the kit's
// tiny item format; nothing touches wasm or the network.
// ---------------------------------------------------------------------------

const fx = vi.hoisted(() => ({
  store: null as unknown as FakeStore,
  listeners: new Set<() => void>(),
  menuPicks: [] as (string | null)[],
  menuEntryCalls: [] as { id: string; label: string; separator?: boolean }[][],
}))

if (typeof URL.createObjectURL !== 'function') URL.createObjectURL = () => 'blob:mock'
if (typeof URL.revokeObjectURL !== 'function') URL.revokeObjectURL = () => {}

vi.mock('../io/libraryStore', () => ({
  libraryStore: () => ({
    available: () => true,
    folderInfo: async () => ({ path: '~/Hew Library' }),
    chooseFolder: async () => null,
    list: () => fx.store.list(),
    read: (f: string) => fx.store.read(f),
    write: (f: string, b: Uint8Array) => fx.store.write(f, b),
    remove: (f: string) => fx.store.remove(f),
    readThumbnail: async () => null,
    writeThumbnail: async () => {},
    reveal: async () => {},
    capabilities: () => ({ canReveal: false, canChooseFolder: true, canDownload: true }),
    webStorage: async () => ({ mode: 'browser', needsReconnect: false }),
    subscribe: (l: () => void) => {
      fx.listeners.add(l)
      return () => fx.listeners.delete(l)
    },
  }),
}))
vi.mock('./library/gridMeasure', () => ({ measureGridColumns: () => 3 }))
vi.mock('../library/nativeChrome', () => ({
  nativeConfirm: async () => true,
  popupNativeMenu: async (entries: { id: string; label: string; separator?: boolean }[]) => {
    fx.menuEntryCalls.push(entries)
    return fx.menuPicks.shift() ?? null
  },
}))
vi.mock('../library/profiles', () => ({
  bundledProfiles: async () => [],
  profileShapeOfBytes: async () => null,
  profileThumbnailPng: async () => new Uint8Array(),
}))
vi.mock('../library/itemFiles', async () => {
  const kit = await import('../library/sourcesTestKit')
  return {
    readItemSummary: async (bytes: Uint8Array) => {
      const f = kit.parseFake(bytes)
      const material = f.lib.category === 'material'
      return {
        format_version: 1,
        objects: material ? 0 : 1,
        materials: material ? 1 : 0,
        components: material ? 0 : 1,
        instances: material ? 0 : 1,
        groups: 0,
        world_sketches: 0,
        annotations: 0,
        guides: 0,
        first_component_name: null,
        first_component_sid: null,
        first_root_name: null,
        doc_attrs: { 'hew.library': f.lib },
        material_entries: [],
        profile_chain_only: false,
      }
    },
    renderItemThumbnail: async () => null,
    updateItemMeta: async (bytes: Uint8Array) => bytes,
    readItemAsset: async () => new Uint8Array(),
    sha256Hex: async (bytes: Uint8Array) => kit.sha256(bytes),
  }
})

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

const CATALOG_SNAPSHOT_BASE = 'https://catalog.example.test/data/'

let env: Env
let notify: () => void

function bindingFor(e: Env): SourcesBinding {
  const listeners = new Set<() => void>()
  notify = () => listeners.forEach((l) => l())
  return {
    sources: e.sources,
    available: () => e.fetcher.on,
    subscribe: (l) => {
      listeners.add(l)
      return () => listeners.delete(l)
    },
  }
}

function props(binding: SourcesBinding | undefined, over: Partial<LibraryDialogProps> = {}): LibraryDialogProps {
  return {
    open: true,
    onClose: vi.fn(),
    placements: {},
    onInsert: vi.fn(),
    onOpenAsDocument: vi.fn(),
    onPaintWith: vi.fn(),
    onAddToPalette: vi.fn(),
    ...(binding !== undefined ? { sources: binding } : {}),
    ...over,
  }
}

/** Publishes the catalog and one snapshot per entry (the index as the build read it). */
function publishCatalog(entries: Array<{ source: string; name: string; license?: string; author?: string; description?: string; items: number }>, removed: Array<{ source: string; reason: string; link?: string }> = []): void {
  const sources: CatalogSourceEntry[] = entries.map((e, i) => ({
    source: e.source,
    kind: 'github',
    name: e.name,
    ...(e.author !== undefined ? { author: e.author } : {}),
    ...(e.description !== undefined ? { description: e.description } : {}),
    ...(e.license !== undefined ? { license: e.license } : {}),
    items: e.items,
    bytes: 100,
    categories: { Components: e.items },
    stale: false,
    snapshot: `sources/s${i}.json`,
  }))
  env.server.publishCatalog({ sources, removed: removed.map((r) => ({ ...r, severity: 'removed' as const })) })
}

function publishSnapshot(i: number, name: string, items: ReturnType<typeof parts>): void {
  // The catalog's snapshot of a source is its index as the build read it.
  const pub = env.server.publish(`web:https://snap.example.test/s${i}/hew.json`, { name, license: 'MIT', items })
  env.server.put(`${CATALOG_SNAPSHOT_BASE}sources/s${i}.json`, new TextEncoder().encode(JSON.stringify(pub.index)))
}

async function openDialog(binding: SourcesBinding | undefined, over: Partial<LibraryDialogProps> = {}) {
  const p = props(binding, over)
  const utils = render(<LibraryDialog {...p} />)
  return { ...utils, p }
}

const sourcesBlock = (): HTMLElement => screen.getByTestId('library-sources')
const sourceRows = (): HTMLElement[] => screen.queryAllByTestId('library-source-row')
/** The Library's own grid, and a source's. */
const localGrid = (): HTMLElement => screen.getByRole('listbox', { name: /^Library items in/ })
const sourceGrid = (): HTMLElement => screen.getByRole('listbox', { name: /^Items in/ })
const localTile = (name: string): HTMLElement => within(localGrid()).getByText(name).closest('[role="option"]') as HTMLElement
const sourceTile = (name: string): HTMLElement => within(sourceGrid()).getByText(name).closest('[role="option"]') as HTMLElement

beforeEach(() => {
  env = makeEnv()
  fx.store = env.store
  fx.listeners.clear()
  fx.menuPicks = []
  fx.menuEntryCalls = []
  localStorage.removeItem('hew.library.view')
  localStorage.removeItem('hew.settings.checkSources')
  Object.defineProperty(window, 'matchMedia', { value: undefined, configurable: true, writable: true })
})

afterEach(() => {
  env.assertClean()
})

const enc = new TextEncoder()

async function gotoSource(name = 'Hinges'): Promise<void> {
  await waitFor(() => expect(sourceRows().length).toBeGreaterThan(0))
  fireEvent.click(sourceRows().find((r) => r.textContent?.includes(name))!)
  await waitFor(() => sourceGrid())
  await waitFor(() => expect(screen.queryByText('Loading…')).not.toBeInTheDocument())
}

function setEdited(file: string): void {
  const f = parseFake(env.store.items.get(file)!)
  ;(f.lib.remote as Record<string, unknown>).edited = true
  env.store.items.set(file, enc.encode(JSON.stringify(f)))
}

describe('F19: Add on a catalog row that fails says why', () => {
  it('shows the failure on the catalog view and keeps the button as Add', async () => {
    publishCatalog([{ source: SRC, name: 'Hinges', license: 'MIT', items: 2 }])
    // No index is published for SRC: reading it fails.
    await openDialog(bindingFor(env))
    fireEvent.click(await screen.findByTestId('library-browse-catalog'))
    fireEvent.click(await screen.findByRole('button', { name: 'Add Hinges' }))
    const alert = await screen.findByTestId('library-catalog-add-error')
    expect(alert.textContent ?? '').not.toBe('')
    expect(screen.getByRole('button', { name: 'Add Hinges' })).toBeEnabled()
  })
})

describe('F22: an item in the Library stays insertable when its source stopped listing it', () => {
  it('Insert works on a gone row, from the button and from Enter', async () => {
    const items = parts(2)
    await env.addSource(SRC, { name: 'Hinges', items })
    await env.sources.addItem(SRC, uuid(1))
    env.server.publish(SRC, { name: 'Hinges', items: [items[1]] })
    await env.sources.check(SRC)
    const { p } = await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(sourceTile('Part 1'))
    const detail = await screen.findByTestId('source-item-detail')
    expect(detail).toHaveAttribute('data-state', 'gone')
    const insert = within(detail).getByRole('button', { name: 'Insert' })
    expect(insert).toBeEnabled()
    fireEvent.click(insert)
    await waitFor(() => expect(p.onInsert).toHaveBeenCalledTimes(1))
    fireEvent.keyDown(sourceGrid(), { key: 'Enter' })
    await waitFor(() => expect(p.onInsert).toHaveBeenCalledTimes(2))
  })
})

describe('F23: Follow is unavailable for a removed source, followed or not', () => {
  it('a followed source the catalog removed keeps Follow enabled so it can be unfollowed (see LibrarySources.unfollow.test.tsx)', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    await env.sources.follow(SRC)
    env.server.publishCatalog({ sources: [], removed: [{ source: SRC, severity: 'removed', reason: 'Copied.' }] })
    await env.sources.catalog()
    await openDialog(bindingFor(env))
    await gotoSource()
    expect(screen.getByTestId('source-removed')).toBeInTheDocument()
    expect(screen.getByTestId('source-follow')).toBeEnabled()
  })
})

describe('F21: Update all cannot be started twice', () => {
  it('two quick clicks run one pass', async () => {
    const items = parts(2)
    await env.addSource(SRC, { name: 'Hinges', items })
    for (const n of [1, 2]) await env.sources.addItem(SRC, uuid(n))
    env.server.publish(SRC, { name: 'Hinges', items: [bump(items[0]), bump(items[1])] })
    await env.sources.check(SRC)
    const real = env.sources.updateAll.bind(env.sources)
    const spy = vi.fn(real)
    ;(env.sources as { updateAll: typeof real }).updateAll = spy
    await openDialog(bindingFor(env))
    const button = within(await screen.findByTestId('source-update-bar')).getByRole('button', { name: 'Update all' })
    fireEvent.click(button)
    fireEvent.click(button)
    const review = await screen.findByTestId('source-update-review')
    await waitFor(() => expect(within(review).getAllByTestId('source-review-result')).toHaveLength(2))
    expect(spy).toHaveBeenCalledTimes(1)
  })
})

describe('Remove source stays removed', () => {
  it('the open source page does not read the source back into the list', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(2) })
    // The entry is gone from the file before the removal call returns (its
    // thumbnail cleanup still runs); the page is open that whole time.
    const realRemove = env.sources.removeSource.bind(env.sources)
    ;(env.sources as { removeSource: typeof realRemove }).removeSource = async (s) => {
      const r = await realRemove(s)
      await new Promise((done) => setTimeout(done, 150))
      return r
    }
    await openDialog(bindingFor(env))
    await gotoSource('Hinges')
    fireEvent.click(await screen.findByTestId('source-remove'))
    const ask = await screen.findByTestId('source-confirm')
    fireEvent.click(within(ask).getByRole('button', { name: 'Remove source' }))
    // The Library's change notice for the removed entry reaches the open page
    // before the removal has finished its cleanup.
    for (let i = 0; i < 5; i++) {
      await act(async () => {
        for (const l of fx.listeners) l()
        await Promise.resolve()
      })
    }
    await waitFor(() => expect(sourceRows()).toHaveLength(0))
    // A later read of the page that was open must not add it back.
    await act(async () => {
      await new Promise((r) => setTimeout(r, 300))
    })
    expect(sourceRows()).toHaveLength(0)
  })
})

describe('Source updates: nothing left to update', () => {
  it('shows only Done, as the default, once every row has its result', async () => {
    const items = parts(1)
    await env.addSource(SRC, { name: 'Hinges', items })
    await env.sources.addItem(SRC, uuid(1))
    env.server.publish(SRC, { name: 'Hinges', items: [bump(items[0])] })
    await env.sources.check(SRC)
    await openDialog(bindingFor(env))
    fireEvent.click(within(await screen.findByTestId('source-update-bar')).getByRole('button', { name: 'Update all' }))
    const review = await screen.findByTestId('source-update-review')
    await waitFor(() => expect(within(review).getAllByTestId('source-review-result')).toHaveLength(1))
    expect(within(review).queryByTestId('source-review-update-selected')).not.toBeInTheDocument()
    const done = within(review).getByTestId('source-review-close')
    expect(done).toHaveTextContent('Done')
    expect(done).toHaveClass('hwlib__btn-primary')
  })
})

describe('F20: a failure after answering "Replace your changes" shows in the Library view', () => {
  it('the detail pane says the item is open for editing', async () => {
    const items = parts(1)
    await env.addSource(SRC, { name: 'Hinges', items })
    await env.sources.addItem(SRC, uuid(1))
    const file = env.store.writes[0]
    setEdited(file)
    env.server.publish(SRC, { name: 'Hinges', items: [bump(items[0])] })
    await env.sources.check(SRC)
    await openDialog(bindingFor(env))
    fireEvent.click(await within(await waitFor(localGrid)).findByText('Part 1'))
    const info = await screen.findByTestId('library-item-source')
    fireEvent.click(within(info).getByRole('button', { name: 'Update from source' }))
    const ask = await screen.findByTestId('source-confirm')
    env.store.editing = [file]
    fireEvent.click(within(ask).getByRole('button', { name: 'Replace your changes' }))
    await waitFor(() => expect(screen.getAllByText('Open for editing. Close it and update again.').length).toBeGreaterThan(0))
  })
})

describe('needsCheck: a source with no cached listing and the setting off', () => {
  async function uncached(): Promise<void> {
    env.options.autoCheck = false
    await env.addSource(SRC, { name: 'Hinges', items: parts(2) })
    env.store.cache.clear()
    env.fetcher.reset()
  }

  it('the source view says it has not been read and offers Refresh, not "0 items"', async () => {
    await uncached()
    await openDialog(bindingFor(env))
    await gotoSource()
    expect(await screen.findByTestId('source-not-checked')).toHaveTextContent('Hew has not read this source yet. Press Refresh to check it.')
    expect(screen.getByTestId('source-refresh')).toBeEnabled()
    expect(screen.queryByTestId('source-empty')).not.toBeInTheDocument()
  })

  it('Look up on that source in the Add dialog opens it without showing "0 items"', async () => {
    await uncached()
    await openDialog(bindingFor(env))
    fireEvent.click(await screen.findByTestId('library-add-source'))
    const dlg = await screen.findByTestId('add-source-dialog')
    fireEvent.change(within(dlg).getByLabelText('Address of the source'), { target: { value: SRC } })
    fireEvent.click(within(dlg).getByRole('button', { name: 'Look up' }))
    await waitFor(() => expect(screen.queryByTestId('add-source-dialog')).not.toBeInTheDocument())
    expect(await screen.findByTestId('source-browser')).toBeInTheDocument()
  })
})
