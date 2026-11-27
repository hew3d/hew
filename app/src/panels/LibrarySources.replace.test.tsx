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

async function gotoSource(name = 'Hinges'): Promise<void> {
  await waitFor(() => expect(sourceRows().length).toBeGreaterThan(0))
  fireEvent.click(sourceRows().find((r) => r.textContent?.includes(name))!)
  await waitFor(() => sourceGrid())
  await waitFor(() => expect(screen.queryByText('Loading…')).not.toBeInTheDocument())
}

// ---------------------------------------------------------------------------
// Design 5.1 / 5.2 / 11 line 29: an id in the Library with no note
// ---------------------------------------------------------------------------

describe('Add to my Library on an item already in the Library without a note', () => {
  it('enables the button, asks "Replace the item in your Library?", and No changes nothing', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    env.store.seed('Components/part-1-000001.hew', { id: uuid(1), name: 'My own Part 1' })
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(sourceTile('Part 1'))
    const detail = await screen.findByTestId('source-item-detail')
    expect(detail).toHaveAttribute('data-state', 'in_library')
    const add = within(detail).getByRole('button', { name: 'Add to my Library' })
    expect(add).toBeEnabled()
    fireEvent.click(add)
    const ask = await screen.findByTestId('source-confirm')
    expect(ask).toHaveAttribute('data-kind', 'replace_item')
    expect(within(ask).getByText('Replace the item in your Library?')).toBeInTheDocument()
    fireEvent.click(within(ask).getByRole('button', { name: 'No' }))
    expect(screen.queryByTestId('source-confirm')).not.toBeInTheDocument()
    expect(env.store.writes).toEqual([])
    expect(noteOf(env.store.items.get('Components/part-1-000001.hew')!)).toBeUndefined()
    expect(within(screen.getByTestId('source-item-detail')).getByRole('button', { name: 'Add to my Library' })).toBeEnabled()
  })

  it('Yes replaces and stamps: the row then reads In your Library with the note', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    env.store.seed('Components/part-1-000001.hew', { id: uuid(1), name: 'My own Part 1' })
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(sourceTile('Part 1'))
    fireEvent.click(within(await screen.findByTestId('source-item-detail')).getByRole('button', { name: 'Add to my Library' }))
    fireEvent.click(within(await screen.findByTestId('source-confirm')).getByRole('button', { name: 'Yes' }))
    await waitFor(() => expect(env.store.writes).toHaveLength(1))
    expect(noteOf(env.store.items.get('Components/part-1-000001.hew')!)?.source).toBe(SRC)
    await waitFor(() => expect(screen.getByTestId('source-item-detail')).toHaveAttribute('data-state', 'in_step'))
    expect(screen.getByTestId('source-item-state')).toHaveTextContent('In your Library')
    expect(within(screen.getByTestId('source-item-detail')).getByRole('button', { name: 'Add to my Library' })).toBeDisabled()
  })

  it('stays disabled for an item linked to another source', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    await env.addSource(SRC2, { name: 'Other', items: parts(1) })
    await env.sources.addItem(SRC2, uuid(1))
    await openDialog(bindingFor(env))
    await gotoSource('Hinges')
    fireEvent.click(sourceTile('Part 1'))
    const detail = await screen.findByTestId('source-item-detail')
    expect(detail).toHaveAttribute('data-state', 'in_library_from_other')
    expect(within(detail).getByRole('button', { name: 'Add to my Library' })).toBeDisabled()
  })

  it('stays disabled for an item already in step', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    await env.sources.addItem(SRC, uuid(1))
    await openDialog(bindingFor(env))
    await gotoSource()
    fireEvent.click(sourceTile('Part 1'))
    const detail = await screen.findByTestId('source-item-detail')
    expect(detail).toHaveAttribute('data-state', 'in_step')
    expect(within(detail).getByRole('button', { name: 'Add to my Library' })).toBeDisabled()
  })
})
