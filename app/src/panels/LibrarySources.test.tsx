/* oxlint-disable lingui/no-unlocalized-strings -- test fixtures */
/**
 * LibraryDialog + sources (docs/design/shared-library-sources.md 5, 7.5, 11):
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
import { getCheckSources } from '../settings/checkSources'

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

// ---------------------------------------------------------------------------
// Acceptance 1, 44, 45, 46: the block, the setting, and when requests happen
// ---------------------------------------------------------------------------

describe('Sources block and the requests the dialog may make', () => {
  it('1: with no sources the block offers "Browse the catalog" and "Add a source…" and no rows', async () => {
    await openDialog(bindingFor(env))
    expect(await screen.findByTestId('library-browse-catalog')).toHaveTextContent('Browse the catalog')
    expect(screen.getByTestId('library-add-source')).toHaveTextContent('Add a source…')
    expect(sourceRows()).toHaveLength(0)
  })

  it('45: setting on and no sources added: opening the Library makes no request', async () => {
    await openDialog(bindingFor(env))
    await screen.findByTestId('library-browse-catalog')
    await act(async () => {
      await new Promise((r) => setTimeout(r, 30))
    })
    expect(env.fetcher.calls).toEqual([])
  })

  it('without a sources binding there is no Sources block and no request at all', async () => {
    await openDialog(undefined)
    await act(async () => {
      await new Promise((r) => setTimeout(r, 20))
    })
    expect(screen.queryByTestId('library-sources')).not.toBeInTheDocument()
    expect(env.fetcher.calls).toEqual([])
  })

  it('"Browse the catalog" is hidden when the catalog address is empty', async () => {
    env = makeEnv({ catalogUrl: '' })
    fx.store = env.store
    await openDialog(bindingFor(env))
    await screen.findByTestId('library-add-source')
    expect(screen.queryByTestId('library-browse-catalog')).not.toBeInTheDocument()
  })

  it('44: with the setting off nothing is checked on open; Refresh runs one check', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(2) })
    env.clock.advance(7 * 3600_000)
    // The kit's gate reads this option; make it read the real setting, so the
    // stored value is what turns the check off.
    Object.defineProperty(env.options, 'autoCheck', { get: () => getCheckSources(), configurable: true })
    localStorage.setItem('hew.settings.checkSources', 'false')
    env.fetcher.reset()
    await openDialog(bindingFor(env))
    const row = await screen.findByTestId('library-source-row')
    await act(async () => {
      await new Promise((r) => setTimeout(r, 30))
    })
    expect(env.fetcher.calls).toEqual([])
    fireEvent.click(row)
    fireEvent.click(await screen.findByTestId('source-refresh'))
    await waitFor(() => expect(env.fetcher.calls.length).toBeGreaterThan(0))
    // One check: resolve, then the index (git kind); no catalog, no files.
    expect(env.fetcher.urls().some((u) => u.includes('catalog.example.test'))).toBe(false)
  })

  it('a source last checked over 6 hours ago is checked when the dialog opens, and the catalog with it', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    env.server.publishCatalog({ sources: [] })
    env.clock.advance(7 * 3600_000)
    env.fetcher.reset()
    await openDialog(bindingFor(env))
    await waitFor(() => expect(env.fetcher.urls().some((u) => u.includes('catalog.example.test'))).toBe(true))
    expect(env.fetcher.urls().some((u) => u.includes('api.example.test'))).toBe(true)
  })

  it('a source checked minutes ago is not checked again on open', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    env.server.publishCatalog({ sources: [] })
    await env.sources.catalog()
    env.fetcher.reset()
    await openDialog(bindingFor(env))
    await screen.findByTestId('library-source-row')
    await act(async () => {
      await new Promise((r) => setTimeout(r, 30))
    })
    expect(env.fetcher.calls).toEqual([])
  })

  it('46: Off shows one line, no rows or buttons, and makes no request; an added item still shows where it came from', async () => {
    const pub = await env.addSource(SRC, { name: 'Hinges', license: 'MIT', items: parts(1) })
    await env.sources.addItem(SRC, uuid(1))
    env.fetcher.reset()
    env.fetcher.on = false
    void pub
    await openDialog(bindingFor(env))
    expect(await screen.findByTestId('library-sources-off')).toHaveTextContent('Sources are turned off on this server.')
    expect(screen.queryByTestId('library-browse-catalog')).not.toBeInTheDocument()
    expect(screen.queryByTestId('library-add-source')).not.toBeInTheDocument()
    expect(sourceRows()).toHaveLength(0)
    const t = await within(localGrid()).findByText('Part 1')
    fireEvent.click(t)
    const info = await screen.findByTestId('library-item-source')
    expect(info).toHaveTextContent('From Hinges · MIT')
    // No update is offered while Off.
    expect(within(info).queryByText('Update from source')).not.toBeInTheDocument()
    expect(env.fetcher.calls).toEqual([])
  })

  it('marks a followed source and shows a warning on a removed or unreachable one', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    await env.addSource(SRC2, { name: 'Slides', items: parts(1) })
    await env.sources.follow(SRC2)
    // Slides becomes unreachable at its next check; Hinges is on the removed list.
    env.server.fail((u) => u.includes('slides'), 'network')
    await env.sources.check(SRC2)
    env.server.publishCatalog({ sources: [], removed: [{ source: SRC, severity: 'removed', reason: 'Copied from another maker.' }] })
    await env.sources.catalog()
    await openDialog(bindingFor(env))
    await waitFor(() => expect(sourceRows()).toHaveLength(2))
    const [hinges, slides] = sourceRows()
    expect(within(hinges).getByTestId('source-warning-mark')).toHaveAttribute('aria-label', 'The Hew catalog removed this source: Copied from another maker.')
    expect(within(hinges).queryByTestId('source-followed-mark')).not.toBeInTheDocument()
    expect(within(slides).getByTestId('source-followed-mark')).toHaveAttribute('aria-label', 'Following')
    expect(within(slides).getByTestId('source-warning-mark')).toHaveAttribute('aria-label', 'Hew could not reach this source the last time it checked.')
  })
})

// ---------------------------------------------------------------------------
// Acceptance 2, 3, 4, 11, 17: the catalog
// ---------------------------------------------------------------------------

describe('Catalog view', () => {
  it('2: an empty catalog says so, offers "Add a source…" and links the publishing guide', async () => {
    publishCatalog([])
    await openDialog(bindingFor(env))
    fireEvent.click(await screen.findByTestId('library-browse-catalog'))
    const empty = await screen.findByTestId('library-catalog-empty')
    expect(empty).toHaveTextContent('No sources are listed yet.')
    fireEvent.click(within(empty).getByRole('button', { name: 'Add a source…' }))
    expect(await screen.findByTestId('add-source-dialog')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    const link = within(empty).getByRole('link')
    expect(link).toHaveAttribute('href', 'https://hew3d.com/learn/publishing-a-source')
    expect(link).toHaveAttribute('rel', expect.stringContaining('noopener'))
  })

  it('3: lists each source with name, author, license and item count; a row opens its items from the snapshot', async () => {
    publishCatalog([
      { source: SRC, name: 'Hinges', author: 'Ann', license: 'MIT', description: 'Real hinges', items: 2 },
      { source: SRC2, name: 'Slides', author: 'Bo', license: 'CC-BY-4.0', items: 1 },
    ])
    publishSnapshot(0, 'Hinges', parts(2))
    publishSnapshot(1, 'Slides', [{ id: uuid(9), name: 'Drawer slide' }])
    await openDialog(bindingFor(env))
    fireEvent.click(await screen.findByTestId('library-browse-catalog'))
    const rows = await screen.findAllByTestId('library-catalog-row')
    expect(rows).toHaveLength(2)
    expect(rows[0]).toHaveTextContent('Hinges')
    expect(rows[0]).toHaveTextContent('Ann')
    expect(rows[0]).toHaveTextContent('MIT · 2 items')
    expect(rows[1]).toHaveTextContent('CC-BY-4.0 · 1 item')
    fireEvent.click(rows[0])
    expect(await within(await waitFor(sourceGrid)).findByText('Part 1')).toBeInTheDocument()
    expect(within(sourceGrid()).getByText('Part 2')).toBeInTheDocument()
    // Read from the snapshot: the source itself was never asked.
    expect(env.fetcher.urls().some((u) => u.includes('snap.example.test'))).toBe(false)
    // A source not added yet offers adding it, not the source's own buttons.
    expect(screen.getByRole('button', { name: 'Add this source' })).toBeInTheDocument()
    expect(screen.queryByTestId('source-refresh')).not.toBeInTheDocument()
  })

  it('17: a catalog that cannot be read shows the fetch message and nothing else; added sources still work', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    env.server.fail((u) => u.includes('catalog.example.test'), 'network')
    await openDialog(bindingFor(env))
    fireEvent.click(await screen.findByTestId('library-browse-catalog'))
    const err = await screen.findByTestId('library-catalog-error')
    expect(err).toHaveTextContent(/Hew could not read this site from the browser/)
    expect(screen.queryByTestId('library-catalog-empty')).not.toBeInTheDocument()
    expect(screen.queryAllByTestId('library-catalog-row')).toHaveLength(0)
    fireEvent.click(screen.getByTestId('library-source-row'))
    expect(await within(await waitFor(sourceGrid)).findByText('Part 1')).toBeInTheDocument()
  })

  it('4: Add on a catalog row adds the source to the sidebar', async () => {
    publishCatalog([{ source: SRC, name: 'Hinges', license: 'MIT', items: 2 }])
    env.server.publish(SRC, { name: 'Hinges', license: 'MIT', items: parts(2) })
    await openDialog(bindingFor(env))
    fireEvent.click(await screen.findByTestId('library-browse-catalog'))
    fireEvent.click(await screen.findByRole('button', { name: 'Add Hinges' }))
    await waitFor(() => expect(sourceRows()).toHaveLength(1))
    expect(sourceRows()[0]).toHaveTextContent('Hinges')
    expect(screen.getByRole('button', { name: 'Hinges is added' })).toBeDisabled()
  })

  it('11: from a preview of a source not added yet, adding one item adds the source and the item', async () => {
    publishCatalog([{ source: SRC, name: 'Hinges', license: 'MIT', items: 2 }])
    publishSnapshot(0, 'Hinges', parts(2))
    env.server.publish(SRC, { name: 'Hinges', license: 'MIT', items: parts(2) })
    await openDialog(bindingFor(env))
    fireEvent.click(await screen.findByTestId('library-browse-catalog'))
    fireEvent.click((await screen.findAllByTestId('library-catalog-row'))[0])
    fireEvent.click(await within(await waitFor(sourceGrid)).findByText('Part 1'))
    const detail = await screen.findByTestId('source-item-detail')
    fireEvent.click(within(detail).getByRole('button', { name: 'Add to my Library' }))
    await waitFor(() => expect(sourceRows()).toHaveLength(1))
    expect(env.store.writes).toHaveLength(1)
    // The view now shows the added source, with the item in the Library.
    await waitFor(() => expect(screen.getByTestId('source-item-state')).toHaveTextContent('In your Library'))
    expect(noteOf(env.store.items.get(env.store.writes[0])!)?.source).toBe(SRC)
  })
})

// ---------------------------------------------------------------------------
// Acceptance 5, 6, 7, 19: the Add a source dialog
// ---------------------------------------------------------------------------

describe('Add a source', () => {
  async function openAdd(): Promise<void> {
    await openDialog(bindingFor(env))
    fireEvent.click(await screen.findByTestId('library-add-source'))
    await screen.findByTestId('add-source-dialog')
  }
  const field = (): HTMLInputElement => screen.getByRole('textbox', { name: 'Address of the source' }) as HTMLInputElement
  async function lookUp(text: string): Promise<void> {
    fireEvent.change(field(), { target: { value: text } })
    fireEvent.click(screen.getByRole('button', { name: 'Look up' }))
  }

  it('5: an unlisted source shows its facts, "No license stated" and that Hew has not checked it', async () => {
    env.server.publishCatalog({ sources: [] })
    env.server.publish('web:https://shop.example.test/parts/hew.json', { name: 'Shop parts', header: { author: 'Cy', description: 'Odds and ends' }, items: parts(3) })
    await openAdd()
    await lookUp('https://shop.example.test/parts/hew.json')
    const facts = await screen.findByTestId('add-source-facts')
    expect(facts).toHaveTextContent('Shop parts')
    expect(facts).toHaveTextContent('Cy')
    expect(facts).toHaveTextContent('shop.example.test')
    expect(facts).toHaveTextContent('No license stated')
    expect(facts).toHaveTextContent('3 items')
    expect(screen.getByTestId('add-source-unlisted')).toHaveTextContent('This source is not in the Hew catalog. Hew has not checked it.')
    fireEvent.click(screen.getByRole('button', { name: 'Add' }))
    await waitFor(() => expect(screen.queryByTestId('add-source-dialog')).not.toBeInTheDocument())
    expect(sourceRows()).toHaveLength(1)
  })

  it('a listed source carries no warning and shows its license', async () => {
    env.server.publishCatalog({ sources: [{ source: SRC, kind: 'github', name: 'Hinges', items: 1, bytes: 1, categories: {}, stale: false, snapshot: 'sources/x.json' }] })
    env.server.publish(SRC, { name: 'Hinges', license: 'MIT', items: parts(1) })
    await openAdd()
    await lookUp(SRC)
    expect(await screen.findByTestId('add-source-facts')).toHaveTextContent('MIT')
    expect(screen.queryByTestId('add-source-unlisted')).not.toBeInTheDocument()
  })

  it('6: the same source again changes nothing and the sidebar keeps one row; another address makes a second row', async () => {
    env.server.publishCatalog({ sources: [] })
    env.server.publish(SRC, { name: 'Hinges', items: parts(1) })
    env.server.publish('web:https://shop.example.test/hew.json', { name: 'Hinges', items: parts(1) })
    await openAdd()
    await lookUp(SRC)
    fireEvent.click(await screen.findByRole('button', { name: 'Add' }))
    await waitFor(() => expect(sourceRows()).toHaveLength(1))
    fireEvent.click(screen.getByTestId('library-add-source'))
    await lookUp(SRC)
    expect(await screen.findByTestId('add-source-existing')).toHaveTextContent('This source is already in your list.')
    fireEvent.click(screen.getByRole('button', { name: 'Done' }))
    expect(sourceRows()).toHaveLength(1)
    expect(env.store.sourceWrites.filter((w) => w.entry !== null)).toHaveLength(1)
    fireEvent.click(screen.getByTestId('library-add-source'))
    await lookUp('https://shop.example.test/hew.json')
    fireEvent.click(await screen.findByRole('button', { name: 'Add' }))
    await waitFor(() => expect(sourceRows()).toHaveLength(2))
  })

  it('7: an address with no source behind it says there is no Hew source there', async () => {
    env.server.publishCatalog({ sources: [] })
    env.server.put('https://nothing.example.test/hew.json', new TextEncoder().encode('<html>'))
    await openAdd()
    await lookUp('https://nothing.example.test/hew.json')
    expect(await screen.findByTestId('add-source-error')).toHaveTextContent('This address does not have a Hew source.')
    expect(sourceRows()).toHaveLength(0)
  })

  it('19: an http:// address is refused without a request: Sources must use https.', async () => {
    await openAdd()
    await lookUp('http://example.test/hew.json')
    expect(await screen.findByTestId('add-source-error')).toHaveTextContent('Sources must use https.')
    expect(env.fetcher.calls).toEqual([])
  })

  it('15: an index from a newer Hew says so', async () => {
    env.server.publishCatalog({ sources: [] })
    env.server.publish(SRC, { name: 'Future', header: { hew: 2 } as never, items: [] })
    await openAdd()
    await lookUp(SRC)
    expect(await screen.findByTestId('add-source-error')).toHaveTextContent('This source needs a newer version of Hew.')
  })

  it('18: on the web a host that cannot be read names both causes and the desktop app', async () => {
    env.server.fail((u) => u.includes('api.example.test') || u.includes('files.example.test'), 'network')
    await openAdd()
    await lookUp(SRC)
    expect(await screen.findByTestId('add-source-error')).toHaveTextContent(
      'Hew could not read this site from the browser. The site may be down, or it may not allow web apps to read it. The desktop app can read it either way.',
    )
  })

  it('Cancel after the source loaded leaves the list and the store untouched (Look up writes nothing)', async () => {
    env.server.publishCatalog({ sources: [] })
    env.server.publish(SRC, { name: 'Hinges', items: parts(1) })
    await openAdd()
    await lookUp(SRC)
    await screen.findByTestId('add-source-facts')
    expect(sourceRows()).toHaveLength(0)
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    await waitFor(() => expect(screen.queryByTestId('add-source-dialog')).not.toBeInTheDocument())
    expect(sourceRows()).toHaveLength(0)
    expect(env.store.sourceWrites).toEqual([])
    expect(env.store.cacheWrites).toEqual([])
  })

  it('Escape closes the dialog and not the Library', async () => {
    const { p } = await (async () => {
      const r = await openDialog(bindingFor(env))
      fireEvent.click(await screen.findByTestId('library-add-source'))
      await screen.findByTestId('add-source-dialog')
      return r
    })()
    fireEvent.keyDown(document.activeElement ?? document.body, { key: 'Escape' })
    await waitFor(() => expect(screen.queryByTestId('add-source-dialog')).not.toBeInTheDocument())
    expect(p.onClose).not.toHaveBeenCalled()
  })
})

// ---------------------------------------------------------------------------
// Source view: header, states, items, actions
// ---------------------------------------------------------------------------

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

const otherNote = { source: SRC2, name: 'Slides', path: 'x.hew', sha256: 'f'.repeat(64), fetchedAt: '2026-11-01T00:00:00.000Z', edited: false }

describe('Source view', () => {
  it('4/5.1: shows every item in its state, in words', async () => {
    const items = parts(6)
    await env.addSource(SRC, { name: 'Hinges', license: 'MIT', items })
    env.store.seed('Components/part-2-000002.hew', { id: uuid(2), name: 'Part 2' })
    env.store.seed('Components/part-3-000003.hew', { id: uuid(3), name: 'Part 3', remote: otherNote })
    for (const n of [4, 5, 6]) await env.sources.addItem(SRC, uuid(n))
    // Part 5 gets a newer version, Part 6 leaves the source, Part 4 is changed here.
    env.server.publish(SRC, { name: 'Hinges', license: 'MIT', items: [...items.slice(0, 4), bump(items[4])] })
    await env.sources.check(SRC)
    setEdited('Components/part-4-000004.hew')
    await openDialog(bindingFor(env))
    await gotoSource()
    expect(within(sourceTile('Part 1')).getByText('Not in your Library')).toBeInTheDocument()
    expect(within(sourceTile('Part 2')).getByText('In your Library')).toBeInTheDocument()
    expect(within(sourceTile('Part 3')).getByText('In your Library from Slides')).toBeInTheDocument()
    expect(within(sourceTile('Part 4')).getByText('In your Library · changed here')).toBeInTheDocument()
    expect(within(sourceTile('Part 5')).getByText('Newer at the source')).toBeInTheDocument()
    expect(within(sourceTile('Part 6')).getByText('The source no longer has this item')).toBeInTheDocument()
  })

  it('shows the header: name, author, license, a safe homepage link, last check, and the buttons', async () => {
    await env.addSource(SRC, {
      name: 'Hinges',
      license: 'MIT',
      header: { author: 'Ann', homepage: 'https://example.test/home', description: 'Real hinges' },
      items: parts(1),
    })
    await openDialog(bindingFor(env))
    await gotoSource()
    await waitFor(() => expect(screen.getByTestId('source-title')).toHaveTextContent('Hinges'))
    const head = screen.getByTestId('source-browser')
    await waitFor(() => expect(head).toHaveTextContent('by Ann'))
    expect(within(head).getByTestId('source-license')).toHaveTextContent('License: MIT')
    expect(within(head).getByText('Real hinges')).toBeInTheDocument()
    const link = within(head).getByRole('link', { name: 'Homepage' })
    expect(link).toHaveAttribute('href', 'https://example.test/home')
    expect(link).toHaveAttribute('rel', 'noopener noreferrer')
    expect(within(head).getByTestId('source-checked')).toHaveTextContent(/^Checked /)
    for (const name of ['Refresh', 'Add all', 'Follow', 'Remove source']) {
      expect(within(head).getByRole('button', { name })).toBeEnabled()
    }
  })

  it('a homepage that is not https is never a link', async () => {
    await env.addSource(SRC, { name: 'Hinges', header: { homepage: 'http://example.test/home' }, items: parts(1) })
    await openDialog(bindingFor(env))
    await gotoSource()
    await waitFor(() => expect(screen.getByTestId('source-homepage')).toBeInTheDocument())
    expect(within(screen.getByTestId('source-homepage')).queryByRole('link')).not.toBeInTheDocument()
  })

  it('is searchable', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: [{ id: uuid(1), name: 'Butt hinge', keywords: ['brass'] }, { id: uuid(2), name: 'Drawer slide' }] })
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.change(screen.getByRole('textbox', { name: 'Search this source' }), { target: { value: 'brass' } })
    expect(within(sourceGrid()).getByText('Butt hinge')).toBeInTheDocument()
    expect(within(sourceGrid()).queryByText('Drawer slide')).not.toBeInTheDocument()
    fireEvent.change(screen.getByRole('textbox', { name: 'Search this source' }), { target: { value: 'zzz' } })
    expect(screen.getByTestId('source-empty')).toHaveTextContent('No items match your search.')
  })

  it('works in the List view too, with the same state labels', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(2) })
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(screen.getByRole('button', { name: 'List view' }))
    const g = await screen.findByRole('grid', { name: /^Items in/ })
    expect(within(g).getAllByText('Not in your Library')).toHaveLength(2)
  })

  it('8: Add to my Library adds the item; the license shows before the button; it files under the source', async () => {
    await env.addSource(SRC, { name: 'Hinges', license: 'MIT', items: [{ id: uuid(1), name: 'Part 1', collection: 'Hinges' }] })
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(sourceTile('Part 1'))
    const detail = await screen.findByTestId('source-item-detail')
    const license = within(detail).getByTestId('source-item-license')
    const add = within(detail).getByRole('button', { name: 'Add to my Library' })
    expect(license).toHaveTextContent('License: MIT')
    expect(license.compareDocumentPosition(add) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    fireEvent.click(add)
    await waitFor(() => expect(env.store.writes).toHaveLength(1))
    const bytes = env.store.items.get(env.store.writes[0])!
    expect(noteOf(bytes)?.source).toBe(SRC)
    expect(parseFake(bytes).lib.collection).toBe('Hinges/Hinges')
    await waitFor(() => expect(screen.getByTestId('source-item-state')).toHaveTextContent('In your Library'))
  })

  it('9: Insert on an item not in the Library adds it, then hands it to the placement flow', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    const { p } = await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(sourceTile('Part 1'))
    fireEvent.click(within(await screen.findByTestId('source-item-detail')).getByRole('button', { name: 'Insert' }))
    await waitFor(() => expect(p.onInsert).toHaveBeenCalledTimes(1))
    const [item, bytes] = (p.onInsert as ReturnType<typeof vi.fn>).mock.calls[0] as [{ displayName: string; file: { fileName: string } }, Uint8Array]
    expect(item.displayName).toBe('Part 1')
    expect(env.store.writes).toEqual([item.file.fileName])
    expect(noteOf(bytes)?.source).toBe(SRC)
  })

  it('Insert on an item already in the Library places it without another add', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    await env.sources.addItem(SRC, uuid(1))
    env.store.resetLogs()
    const { p } = await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(sourceTile('Part 1'))
    fireEvent.click(within(await screen.findByTestId('source-item-detail')).getByRole('button', { name: 'Insert' }))
    await waitFor(() => expect(p.onInsert).toHaveBeenCalledTimes(1))
    expect(env.store.writes).toEqual([])
  })

  it('10: a material can be added but not inserted; a model is added the same way', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: [{ id: uuid(1), name: 'Oak', category: 'material' }, { id: uuid(2), name: 'House', category: 'model' }] })
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(sourceTile('Oak'))
    let detail = await screen.findByTestId('source-item-detail')
    expect(within(detail).queryByRole('button', { name: 'Insert' })).not.toBeInTheDocument()
    fireEvent.click(within(detail).getByRole('button', { name: 'Add to my Library' }))
    await waitFor(() => expect(env.store.writes).toHaveLength(1))
    expect(env.store.writes[0]).toMatch(/^Materials\//)
    fireEvent.click(sourceTile('House'))
    detail = await screen.findByTestId('source-item-detail')
    fireEvent.click(within(detail).getByRole('button', { name: 'Add to my Library' }))
    await waitFor(() => expect(env.store.writes).toHaveLength(2))
    expect(env.store.writes[1]).toMatch(/^Models\//)
  })

  it('29: an id already in the Library with no note asks "Replace the item in your Library?"; Yes links it, No leaves it', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    env.store.seed('Components/part-1-000001.hew', { id: uuid(1), name: 'My own Part 1' })
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(sourceTile('Part 1'))
    // The state says it is in the Library, and Add is still offered: it asks to replace
    // (the full flow is in LibrarySources.replace.test.tsx).
    expect(within(await screen.findByTestId('source-item-detail')).getByRole('button', { name: 'Add to my Library' })).toBeEnabled()
  })

  it('29: replacing asks first (here through Insert on an unlinked copy of another file name)', async () => {
    // An index entry whose id is in the Library under another path, no note: the
    // module asks to replace. Drive the module question through the dialog's own flow.
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    const base = await openDialog(bindingFor(env))
    await gotoSource()
    // Put the item in the Library AFTER the view loaded, so the row still says
    // "Not in your Library" and Add reaches the module's question.
    env.store.seed('Components/part-1-000001.hew', { id: uuid(1), name: 'My own Part 1' })
    fireEvent.click(sourceTile('Part 1'))
    fireEvent.click(within(await screen.findByTestId('source-item-detail')).getByRole('button', { name: 'Add to my Library' }))
    const ask = await screen.findByTestId('source-confirm')
    expect(ask).toHaveAttribute('data-kind', 'replace_item')
    expect(within(ask).getByText('Replace the item in your Library?')).toBeInTheDocument()
    fireEvent.click(within(ask).getByRole('button', { name: 'No' }))
    expect(screen.queryByTestId('source-confirm')).not.toBeInTheDocument()
    expect(env.store.writes).toEqual([])
    fireEvent.click(within(await screen.findByTestId('source-item-detail')).getByRole('button', { name: 'Add to my Library' }))
    fireEvent.click(within(await screen.findByTestId('source-confirm')).getByRole('button', { name: 'Yes' }))
    await waitFor(() => expect(env.store.writes).toHaveLength(1))
    expect(noteOf(env.store.items.get('Components/part-1-000001.hew')!)?.source).toBe(SRC)
    void base
  })

  it('12: an item whose bytes do not match shows "changed while Hew was reading it" and the Library is unchanged', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: [{ id: uuid(1), name: 'Part 1', servedBytes: enc.encode('not the item') }] })
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(sourceTile('Part 1'))
    fireEvent.click(within(await screen.findByTestId('source-item-detail')).getByRole('button', { name: 'Add to my Library' }))
    expect(await screen.findByTestId('source-message')).toHaveTextContent('This item changed while Hew was reading it. Try again in a few minutes.')
    expect(env.store.items.size).toBe(0)
  })

  it('13: an entry whose id differs from the file is refused: "The source lists this item incorrectly."', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: [{ id: uuid(1), name: 'Part 1', listedId: uuid(7) }] })
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(sourceTile('Part 1'))
    fireEvent.click(within(await screen.findByTestId('source-item-detail')).getByRole('button', { name: 'Add to my Library' }))
    expect(await screen.findByTestId('source-message')).toHaveTextContent('The source lists this item incorrectly. Hew did not add it.')
    expect(env.store.items.size).toBe(0)
  })

  it('14: a file name taken by a different item gives the name-clash message and leaves that item alone', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    const clash = env.store.seed('Components/part-1-000001.hew', { id: uuid(99), name: 'Part 1' })
    const before = env.store.items.get(clash)!
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(sourceTile('Part 1'))
    fireEvent.click(within(await screen.findByTestId('source-item-detail')).getByRole('button', { name: 'Add to my Library' }))
    expect(await screen.findByTestId('source-message')).toHaveTextContent('An item with the same file name is already in your Library.')
    expect(env.store.items.get(clash)).toBe(before)
  })

  it('16: a source that goes offline keeps its listing with the time of the last good check; coming back clears the warning', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(2) })
    await openDialog(bindingFor(env))
    await gotoSource()
    env.server.fail((u) => u.includes('example.test') && !u.includes('catalog'), 'network')
    fireEvent.click(screen.getByTestId('source-refresh'))
    expect(await screen.findByTestId('source-message')).toHaveTextContent(/Hew could not read this site from the browser/)
    await waitFor(() => expect(screen.getByTestId('source-unreachable')).toBeInTheDocument())
    expect(within(sourceGrid()).getByText('Part 1')).toBeInTheDocument()
    expect(screen.getByTestId('source-checked')).toHaveTextContent(/^Checked /)
    expect(within(sourceRows()[0]).getByTestId('source-warning-mark')).toBeInTheDocument()
    env.server.clearRules()
    fireEvent.click(screen.getByTestId('source-refresh'))
    await waitFor(() => expect(screen.queryByTestId('source-unreachable')).not.toBeInTheDocument())
    expect(within(sourceRows()[0]).queryByTestId('source-warning-mark')).not.toBeInTheDocument()
    expect(screen.queryByTestId('source-message')).not.toBeInTheDocument()
  })

  it('20: following a source bigger than the free storage refuses and shows both sizes', async () => {
    env.options.storage = { usage: 0, quota: 10 }
    await env.addSource(SRC, { name: 'Hinges', items: parts(3) })
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(screen.getByTestId('source-follow'))
    const msg = await screen.findByTestId('source-run-message')
    expect(msg).toHaveTextContent(/This source needs .* Your browser has 10 B free\./)
    expect(env.store.items.size).toBe(0)
  })

  it('26: an item open for editing reads "Open for editing. Close it and update again."', async () => {
    const items = parts(1)
    await env.addSource(SRC, { name: 'Hinges', items })
    await env.sources.addItem(SRC, uuid(1))
    env.server.publish(SRC, { name: 'Hinges', items: [bump(items[0])] })
    await env.sources.check(SRC)
    env.store.editing = [env.store.writes[0]]
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(sourceTile('Part 1'))
    fireEvent.click(within(await screen.findByTestId('source-item-detail')).getByRole('button', { name: 'Update from source' }))
    expect(await screen.findByTestId('source-message')).toHaveTextContent('Open for editing. Close it and update again.')
  })

  it('24/25: updating an item changed here asks "Replace your changes" or "Keep mine"', async () => {
    const items = parts(1)
    await env.addSource(SRC, { name: 'Hinges', items })
    await env.sources.addItem(SRC, uuid(1))
    const file = env.store.writes[0]
    setEdited(file)
    env.server.publish(SRC, { name: 'Hinges', items: [bump(items[0], 'v2')] })
    await env.sources.check(SRC)
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(sourceTile('Part 1'))
    const update = (): HTMLElement => within(screen.getByTestId('source-item-detail')).getByRole('button', { name: 'Update from source' })
    fireEvent.click(await waitFor(update))
    let ask = await screen.findByTestId('source-confirm')
    expect(ask).toHaveAttribute('data-kind', 'replace_changes')
    // Keep mine: nothing is written, the row still reads newer, the item is kept.
    fireEvent.click(within(ask).getByRole('button', { name: 'Keep mine' }))
    await waitFor(() => expect(env.store.entryFor(SRC)?.kept[uuid(1)]).toBeTruthy())
    expect(env.store.writes).toHaveLength(1)
    expect(within(sourceTile('Part 1')).getByText('Newer at the source · changed here')).toBeInTheDocument()
    // Replace your changes: the file is the source's and "changed here" is gone.
    fireEvent.click(update())
    ask = await screen.findByTestId('source-confirm')
    fireEvent.click(within(ask).getByRole('button', { name: 'Replace your changes' }))
    await waitFor(() => expect(env.store.writes).toHaveLength(2))
    expect(noteOf(env.store.items.get(file)!)?.edited).toBe(false)
    expect(parseFake(env.store.items.get(file)!).body).toBe('v2')
  })

  it('35: a removed source shows the reason, disables Add all, Follow and Add, and still inserts what is in the Library', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(2) })
    await env.sources.addItem(SRC, uuid(1))
    env.server.publishCatalog({ sources: [], removed: [{ source: SRC, severity: 'removed', reason: 'Copied from another maker.', link: 'https://example.test/why' }] })
    await env.sources.catalog()
    const { p } = await openDialog(bindingFor(env))
    await gotoSource()
    const removed = screen.getByTestId('source-removed')
    expect(removed).toHaveTextContent('The Hew catalog removed this source: Copied from another maker.')
    expect(within(removed).getByRole('link')).toHaveAttribute('href', 'https://example.test/why')
    expect(screen.getByTestId('source-add-all')).toBeDisabled()
    expect(screen.getByTestId('source-follow')).toBeDisabled()
    fireEvent.click(sourceTile('Part 2'))
    const detail = await screen.findByTestId('source-item-detail')
    expect(within(detail).getByRole('button', { name: 'Add to my Library' })).toBeDisabled()
    expect(within(detail).getByTestId('source-item-removed')).toHaveTextContent('Copied from another maker.')
    fireEvent.click(sourceTile('Part 1'))
    fireEvent.click(within(await screen.findByTestId('source-item-detail')).getByRole('button', { name: 'Insert' }))
    await waitFor(() => expect(p.onInsert).toHaveBeenCalledTimes(1))
    // No bar count for a removed source.
    expect(screen.queryByTestId('source-update-bar')).not.toBeInTheDocument()
  })

  it('36: Remove source asks, then the row leaves the sidebar and its items stay in the Library', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    await env.sources.addItem(SRC, uuid(1))
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(screen.getByTestId('source-remove'))
    const ask = await screen.findByTestId('source-confirm')
    expect(ask).toHaveAttribute('data-kind', 'remove_source')
    fireEvent.click(within(ask).getByRole('button', { name: 'Cancel' }))
    expect(sourceRows()).toHaveLength(1)
    fireEvent.click(screen.getByTestId('source-remove'))
    fireEvent.click(within(await screen.findByTestId('source-confirm')).getByRole('button', { name: 'Remove source' }))
    await waitFor(() => expect(sourceRows()).toHaveLength(0))
    expect(env.store.items.size).toBe(1)
    // The Library's own view is back, and the item reads where it came from with no update offered.
    fireEvent.click(await within(await waitFor(localGrid)).findByText('Part 1'))
    const info = await screen.findByTestId('library-item-source')
    expect(info).toHaveTextContent('From Hinges')
    expect(info).toHaveTextContent('This source is not in your list. Add it again to get updates.')
    expect(within(info).queryByText('Update from source')).not.toBeInTheDocument()
    expect(screen.queryByTestId('source-mark')).not.toBeInTheDocument()
  })
})

// ---------------------------------------------------------------------------
// Runs: Add all, Follow, progress, Cancel, summary, Retry
// ---------------------------------------------------------------------------

function gateFetch(match: string): { release(): void } {
  let release!: () => void
  const gate = new Promise<void>((r) => {
    release = r
  })
  const orig = env.fetcher.fetchBytes.bind(env.fetcher)
  env.fetcher.fetchBytes = async (url, max) => {
    if (url.includes(match)) await gate
    return orig(url, max)
  }
  return { release }
}

describe('Add all and Follow', () => {
  it('30: Follow counts to the total and adds every item; the Follow button becomes a pressed toggle', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(5) })
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(screen.getByTestId('source-follow'))
    await waitFor(() => expect(env.store.writes).toHaveLength(5))
    const summary = await screen.findByTestId('source-run-summary')
    expect(summary).toHaveTextContent('5 added, 0 failed')
    expect(screen.getByTestId('source-follow')).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByTestId('source-follow')).toHaveTextContent('Following')
    expect(within(sourceRows()[0]).getByTestId('source-followed-mark')).toBeInTheDocument()
    fireEvent.click(screen.getByTestId('source-follow'))
    await waitFor(() => expect(screen.getByTestId('source-follow')).toHaveAttribute('aria-pressed', 'false'))
    expect(env.store.entryFor(SRC)?.followed).toBe(false)
  })

  it('shows progress with Cancel while it runs; Cancel keeps what landed and the rest stay "Not in your Library" (32)', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(5) })
    const gate = gateFetch('part-3')
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(screen.getByTestId('source-add-all'))
    const progress = await screen.findByTestId('source-progress')
    await waitFor(() => expect(progress).toHaveTextContent('Adding 2 of 5…'))
    fireEvent.click(screen.getByTestId('source-cancel-run'))
    gate.release()
    const summary = await screen.findByTestId('source-run-summary')
    await waitFor(() => expect(summary).toHaveTextContent('3 added, 0 failed'))
    expect(summary).toHaveTextContent('Stopped.')
    expect(env.store.writes).toHaveLength(3)
    await waitFor(() => expect(within(sourceTile('Part 5')).getByText('Not in your Library')).toBeInTheDocument())
    expect(within(sourceTile('Part 1')).getByText('In your Library')).toBeInTheDocument()
  })

  it('33: one failing item: the run finishes, the summary reads "N added, 1 failed", and Retry adds it once it is served', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(4) })
    env.server.fail((u) => u.includes('part-3'), 'network')
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(screen.getByTestId('source-add-all'))
    const summary = await screen.findByTestId('source-run-summary')
    await waitFor(() => expect(summary).toHaveTextContent('3 added, 1 failed'))
    env.server.clearRules()
    fireEvent.click(screen.getByTestId('source-retry'))
    await waitFor(() => expect(screen.getByTestId('source-run-summary')).toHaveTextContent('1 added, 0 failed'))
    expect(env.store.writes).toHaveLength(4)
    expect(screen.queryByTestId('source-retry')).not.toBeInTheDocument()
  })

  it('a rate limit stops the run and says so', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(4) })
    env.server.fail((u) => u.includes('part-2'), 'rate_limited')
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(screen.getByTestId('source-add-all'))
    expect(await screen.findByTestId('source-run-message')).toHaveTextContent(/is limiting requests right now\. Try again later\./)
  })

  it('profiles are left out when the profile preview is off', async () => {
    env.options.profiles = false
    await env.addSource(SRC, { name: 'Hinges', items: [{ id: uuid(1), name: 'Part 1' }, { id: uuid(2), name: 'A profile', category: 'profile' }] })
    await openDialog(bindingFor(env))
    await gotoSource()
    expect(within(sourceGrid()).queryByText('A profile')).not.toBeInTheDocument()
    fireEvent.click(screen.getByTestId('source-add-all'))
    await screen.findByTestId('source-run-summary')
    expect(env.store.writes).toHaveLength(1)
  })
})

// ---------------------------------------------------------------------------
// Local items: "From", state line, the mark, the menu
// ---------------------------------------------------------------------------

async function addedItem(over: { license?: string } = {}) {
  const items = parts(1)
  await env.addSource(SRC, { name: 'Hinges', license: over.license ?? 'MIT', items })
  await env.sources.addItem(SRC, uuid(1))
  return { items, file: env.store.writes[0] }
}

async function makeNewer(items: ReturnType<typeof parts>): Promise<void> {
  env.server.publish(SRC, { name: 'Hinges', license: 'MIT', items: [bump(items[0])] })
  await env.sources.check(SRC)
}

describe('Local items from a source', () => {
  it('8: the detail pane reads "From <source> · <license>" and the state line', async () => {
    await addedItem()
    await openDialog(bindingFor(env))
    fireEvent.click(await within(await waitFor(localGrid)).findByText('Part 1'))
    const info = await screen.findByTestId('library-item-source')
    expect(info).toHaveTextContent('From Hinges · MIT')
    expect(info).toHaveTextContent('It matches the source.')
    expect(screen.queryByTestId('source-mark')).not.toBeInTheDocument()
  })

  it('an item with no note has no source line, mark or menu rows', async () => {
    env.store.seed('Components/plain-000001.hew', { id: uuid(50), name: 'Plain' })
    await openDialog(bindingFor(env))
    fireEvent.click(await within(await waitFor(localGrid)).findByText('Plain'))
    await waitFor(() => expect(document.querySelector('.hwlib__detail:not(.hwlib__detail--empty)')).not.toBeNull())
    expect(screen.queryByTestId('library-item-source')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Plain actions' }))
    const menu = await screen.findByRole('menu')
    expect(within(menu).queryByText('Update from source')).not.toBeInTheDocument()
    expect(within(menu).queryByText(/^Unlink from/)).not.toBeInTheDocument()
  })

  it('21: a newer item shows the mark on its tile and its row, and the state line says so', async () => {
    const { items } = await addedItem()
    await makeNewer(items)
    await openDialog(bindingFor(env))
    const t = await within(await waitFor(localGrid)).findByText('Part 1')
    await waitFor(() => expect(within(localTile('Part 1')).getByTestId('source-mark')).toHaveAttribute('aria-label', 'Newer at the source'))
    fireEvent.click(t)
    expect(await screen.findByTestId('library-item-source')).toHaveTextContent('Newer at the source')
    fireEvent.click(screen.getByRole('button', { name: 'List view' }))
    const g = await screen.findByRole('grid', { name: /^Library items in/ })
    await waitFor(() => expect(within(g).getByTestId('source-mark')).toBeInTheDocument())
  })

  it('28: the web menu rows "Update from source" and "Unlink from <source>" work', async () => {
    const { items, file } = await addedItem()
    await makeNewer(items)
    await openDialog(bindingFor(env))
    await within(await waitFor(localGrid)).findByText('Part 1')
    await waitFor(() => expect(screen.getByTestId('source-mark')).toBeInTheDocument())
    fireEvent.click(screen.getByRole('button', { name: 'Part 1 actions' }))
    fireEvent.click(within(await screen.findByRole('menu')).getByText('Update from source'))
    await waitFor(() => expect(parseFake(env.store.items.get(file)!).body).toBe('v2'))
    await waitFor(() => expect(screen.queryByTestId('source-mark')).not.toBeInTheDocument())
    fireEvent.click(screen.getByRole('button', { name: 'Part 1 actions' }))
    expect(within(screen.getByRole('menu')).queryByText('Update from source')).not.toBeInTheDocument()
    fireEvent.click(within(screen.getByRole('menu')).getByText('Unlink from Hinges'))
    await waitFor(() => expect(noteOf(env.store.items.get(file)!)).toBeUndefined())
    await waitFor(() => expect(screen.queryByTestId('library-item-source')).not.toBeInTheDocument())
    // The source view now reads "In your Library".
    fireEvent.click(screen.getByTestId('library-source-row'))
    await waitFor(() => sourceGrid())
    expect(await within(sourceTile('Part 1')).findByText('In your Library')).toBeInTheDocument()
  })

  it('the detail pane offers the same two actions, and an update failure reads in plain words', async () => {
    const { items, file } = await addedItem()
    await makeNewer(items)
    env.store.editing = [file]
    await openDialog(bindingFor(env))
    fireEvent.click(await within(await waitFor(localGrid)).findByText('Part 1'))
    const info = await screen.findByTestId('library-item-source')
    fireEvent.click(await within(info).findByRole('button', { name: 'Update from source' }))
    expect(await screen.findByText('Open for editing. Close it and update again.')).toBeInTheDocument()
    expect(within(info).getByRole('button', { name: 'Unlink from Hinges' })).toBeInTheDocument()
  })

  it('the desktop window variant offers the rows through the native menu and acts on the pick', async () => {
    const { items, file } = await addedItem()
    await makeNewer(items)
    fx.menuPicks = ['updateSource']
    await openDialog(bindingFor(env), { variant: 'window' })
    await within(await waitFor(localGrid)).findByText('Part 1')
    await waitFor(() => expect(screen.getByTestId('source-mark')).toBeInTheDocument())
    fireEvent.contextMenu(localTile('Part 1'))
    await waitFor(() => expect(fx.menuEntryCalls).toHaveLength(1))
    const labels = fx.menuEntryCalls[0].map((e) => e.label)
    expect(labels).toContain('Update from source')
    expect(labels).toContain('Unlink from Hinges')
    await waitFor(() => expect(parseFake(env.store.items.get(file)!).body).toBe('v2'))
  })

  it('the native menu of an item with no note has no source rows', async () => {
    env.store.seed('Components/plain-000001.hew', { id: uuid(50), name: 'Plain' })
    await openDialog(bindingFor(env), { variant: 'window' })
    await within(await waitFor(localGrid)).findByText('Plain')
    fireEvent.contextMenu(localTile('Plain'))
    await waitFor(() => expect(fx.menuEntryCalls).toHaveLength(1))
    const labels = fx.menuEntryCalls[0].map((e) => e.label)
    expect(labels.some((l) => /source/i.test(l) && l !== 'Remove Source Info')).toBe(false)
  })

  it('34: an item the source no longer has says so, and offers no update', async () => {
    const { items } = await addedItem()
    env.server.publish(SRC, { name: 'Hinges', items: [{ id: uuid(2), name: 'Other' }] })
    await env.sources.check(SRC)
    void items
    await openDialog(bindingFor(env))
    fireEvent.click(await within(await waitFor(localGrid)).findByText('Part 1'))
    const info = await screen.findByTestId('library-item-source')
    await waitFor(() => expect(info).toHaveTextContent('The source no longer has this item.'))
    expect(within(info).queryByRole('button', { name: 'Update from source' })).not.toBeInTheDocument()
    expect(screen.queryByTestId('source-mark')).not.toBeInTheDocument()
  })

  it('31: deleting an item from a followed source puts its id on the skip list', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(2) })
    await env.sources.follow(SRC)
    await openDialog(bindingFor(env))
    fireEvent.click(await within(await waitFor(localGrid)).findByText('Part 1'))
    fireEvent.click(await screen.findByRole('button', { name: 'Delete from library…' }))
    fireEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Delete' }))
    await waitFor(() => expect(env.store.entryFor(SRC)?.skipped).toContain(uuid(1)))
    // Not counted as new, and reads "Not in your Library" in the source view.
    await waitFor(() => expect(screen.queryByTestId('source-update-bar')).not.toBeInTheDocument())
    fireEvent.click(screen.getByTestId('library-source-row'))
    await waitFor(() => sourceGrid())
    expect(await within(sourceTile('Part 1')).findByText('Not in your Library')).toBeInTheDocument()
  })
})

// ---------------------------------------------------------------------------
// The bar and Review
// ---------------------------------------------------------------------------

describe('Update bar and Review', () => {
  async function twoNewer() {
    const items = parts(3)
    await env.addSource(SRC, { name: 'Hinges', license: 'MIT', items })
    for (const n of [1, 2, 3]) await env.sources.addItem(SRC, uuid(n))
    env.server.publish(SRC, { name: 'Hinges', license: 'MIT', items: [bump(items[0]), bump(items[1]), items[2]] })
    await env.sources.check(SRC)
    return items
  }

  it('21: says how many items have newer versions at their sources, and Update all updates them', async () => {
    await twoNewer()
    await openDialog(bindingFor(env))
    const bar = await screen.findByTestId('source-update-bar')
    expect(bar).toHaveTextContent('2 items have newer versions at their sources.')
    fireEvent.click(within(bar).getByRole('button', { name: 'Update all' }))
    const review = await screen.findByTestId('source-update-review')
    await waitFor(() => expect(within(review).getAllByTestId('source-review-result')).toHaveLength(2))
    for (const r of within(review).getAllByTestId('source-review-result')) expect(r).toHaveTextContent('Updated')
    fireEvent.click(within(review).getByRole('button', { name: 'Done' }))
    await waitFor(() => expect(screen.queryByTestId('source-update-bar')).not.toBeInTheDocument())
    expect(parseFake(env.store.items.get(env.store.writes[0])!).body).toBe('v2')
  })

  it('one item newer reads "1 item has a newer version at its source."', async () => {
    const items = parts(1)
    await env.addSource(SRC, { name: 'Hinges', items })
    await env.sources.addItem(SRC, uuid(1))
    env.server.publish(SRC, { name: 'Hinges', items: [bump(items[0])] })
    await env.sources.check(SRC)
    await openDialog(bindingFor(env))
    expect(await screen.findByTestId('source-update-bar')).toHaveTextContent('1 item has a newer version at its source.')
  })

  it('Later hides the bar until the next check', async () => {
    await twoNewer()
    await openDialog(bindingFor(env))
    const bar = await screen.findByTestId('source-update-bar')
    fireEvent.click(within(bar).getByRole('button', { name: 'Later' }))
    expect(screen.queryByTestId('source-update-bar')).not.toBeInTheDocument()
    // A newer check (another tab, or Refresh) brings it back.
    env.clock.advance(60_000)
    await env.sources.check(SRC)
    act(() => notify())
    expect(await screen.findByTestId('source-update-bar')).toBeInTheDocument()
  })

  it('30: a followed source gets its own line: "<Source> has N new and M changed items"', async () => {
    const items = parts(4)
    await env.addSource(SRC, { name: 'Hinges', items: items.slice(0, 2) })
    await env.sources.follow(SRC)
    env.server.publish(SRC, { name: 'Hinges', items: [bump(items[0]), items[1], items[2], items[3]] })
    await env.sources.check(SRC)
    await openDialog(bindingFor(env))
    const bar = await screen.findByTestId('source-update-bar')
    expect(within(bar).getByTestId('source-update-followed')).toHaveTextContent('Hinges has 2 new and 1 changed items')
    expect(within(bar).queryByTestId('source-update-newer')).not.toBeInTheDocument()
  })

  it('Review lists each row\'s source and the name it will take, including a local rename', async () => {
    await twoNewer()
    const file = env.store.writes[0]
    const f = parseFake(env.store.items.get(file)!)
    f.lib.name = 'My hinge'
    env.store.items.set(file, enc.encode(JSON.stringify(f)))
    await openDialog(bindingFor(env))
    fireEvent.click(within(await screen.findByTestId('source-update-bar')).getByRole('button', { name: 'Review…' }))
    const review = await screen.findByTestId('source-update-review')
    const rows = await within(review).findAllByTestId('source-review-row')
    expect(rows).toHaveLength(2)
    expect(within(rows[0]).getByTestId('source-review-name')).toHaveTextContent('Part 1')
    expect(within(rows[0]).getByTestId('source-review-source')).toHaveTextContent('From Hinges')
    expect(within(rows[0]).getByTestId('source-review-rename')).toHaveTextContent('Now named “My hinge”')
    expect(within(rows[1]).queryByTestId('source-review-rename')).not.toBeInTheDocument()
    // Update selected takes only what is checked.
    fireEvent.click(within(rows[1]).getByRole('checkbox'))
    fireEvent.click(within(review).getByTestId('source-review-update-selected'))
    await waitFor(() => expect(within(review).getAllByTestId('source-review-result')).toHaveLength(1))
    expect(env.store.writes).toHaveLength(4) // three adds + one update
  })

  it('24: an item changed here is left by Update all; Review asks, and "Keep mine" stops the count until it changes again', async () => {
    const items = parts(2)
    await env.addSource(SRC, { name: 'Hinges', items })
    for (const n of [1, 2]) await env.sources.addItem(SRC, uuid(n))
    setEdited(env.store.writes[0])
    env.server.publish(SRC, { name: 'Hinges', items: [bump(items[0]), bump(items[1])] })
    await env.sources.check(SRC)
    await openDialog(bindingFor(env))
    const bar = await screen.findByTestId('source-update-bar')
    expect(bar).toHaveTextContent('2 items have newer versions')
    fireEvent.click(within(bar).getByRole('button', { name: 'Update all' }))
    const review = await screen.findByTestId('source-update-review')
    await waitFor(() => expect(within(review).getAllByTestId('source-review-result')).toHaveLength(1))
    // The edited item was not replaced.
    expect(parseFake(env.store.items.get(env.store.writes[0])!).body).not.toBe('v2')
    const row = within(review).getAllByTestId('source-review-row').find((r) => r.getAttribute('data-edited') === 'true')!
    fireEvent.click(within(row).getByRole('button', { name: 'Keep mine' }))
    await waitFor(() => expect(within(row).getByTestId('source-review-result')).toHaveTextContent('Kept yours'))
    fireEvent.click(within(review).getByRole('button', { name: 'Done' }))
    await waitFor(() => expect(screen.queryByTestId('source-update-bar')).not.toBeInTheDocument())
    // The source view still reads newer.
    fireEvent.click(screen.getByTestId('library-source-row'))
    await waitFor(() => sourceGrid())
    expect(await within(sourceTile('Part 1')).findByText('Newer at the source · changed here')).toBeInTheDocument()
  })

  it('25: Review "Replace your changes" takes the source\'s file and clears "changed here"', async () => {
    const items = parts(1)
    await env.addSource(SRC, { name: 'Hinges', items })
    await env.sources.addItem(SRC, uuid(1))
    const file = env.store.writes[0]
    setEdited(file)
    env.server.publish(SRC, { name: 'Hinges', items: [bump(items[0])] })
    await env.sources.check(SRC)
    await openDialog(bindingFor(env))
    fireEvent.click(within(await screen.findByTestId('source-update-bar')).getByRole('button', { name: 'Review…' }))
    const review = await screen.findByTestId('source-update-review')
    fireEvent.click(await within(review).findByRole('button', { name: 'Replace your changes' }))
    await waitFor(() => expect(within(review).getByTestId('source-review-result')).toHaveTextContent('Replaced with the source’s version'))
    expect(noteOf(env.store.items.get(file)!)?.edited).toBe(false)
  })

  it('26: Update all reports an item open for editing and updates the others', async () => {
    await twoNewer()
    env.store.editing = [env.store.writes[0]]
    await openDialog(bindingFor(env))
    fireEvent.click(within(await screen.findByTestId('source-update-bar')).getByRole('button', { name: 'Update all' }))
    const review = await screen.findByTestId('source-update-review')
    await waitFor(() => expect(within(review).getAllByTestId('source-review-result')).toHaveLength(2))
    const results = within(review).getAllByTestId('source-review-result').map((r) => r.textContent)
    expect(results).toContain('Open for editing. Close it and update again.')
    expect(results).toContain('Updated')
  })

  it('Off: no bar, even with a newer item cached', async () => {
    await twoNewer()
    env.fetcher.on = false
    await openDialog(bindingFor(env))
    await screen.findByTestId('library-sources-off')
    expect(screen.queryByTestId('source-update-bar')).not.toBeInTheDocument()
  })
})
