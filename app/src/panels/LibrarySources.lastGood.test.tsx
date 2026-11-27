/* oxlint-disable lingui/no-unlocalized-strings -- test fixtures */
/** After a failed refresh the source header shows the time of the last GOOD
 * check (5.2), a never-good source shows "Not checked yet", and the persistent
 * unreachable banner carries the HTTP status (6.5). */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { LibraryDialog } from './LibraryDialog'
import type { SourcesBinding } from '../library/sourcesDeps'
import { makeEnv, parts, SRC, type Env, type FakeStore } from '../library/sourcesTestKit'

const fx = vi.hoisted(() => ({ store: null as unknown as FakeStore, listeners: new Set<() => void>() }))

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
vi.mock('../library/nativeChrome', () => ({ nativeConfirm: async () => true, popupNativeMenu: async () => null }))
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
      return {
        format_version: 1, objects: 1, materials: 0, components: 1, instances: 1, groups: 0, world_sketches: 0,
        annotations: 0, guides: 0, first_component_name: null, first_component_sid: null, first_root_name: null,
        doc_attrs: { 'hew.library': f.lib }, material_entries: [], profile_chain_only: false,
      }
    },
    renderItemThumbnail: async () => null,
    updateItemMeta: async (bytes: Uint8Array) => bytes,
    readItemAsset: async () => new Uint8Array(),
    sha256Hex: async (bytes: Uint8Array) => kit.sha256(bytes),
  }
})

let env: Env

function bindingFor(e: Env): SourcesBinding {
  const listeners = new Set<() => void>()
  return {
    sources: e.sources,
    available: () => e.fetcher.on,
    subscribe: (l) => {
      listeners.add(l)
      return () => listeners.delete(l)
    },
  }
}

const sourceRows = (): HTMLElement[] => screen.queryAllByTestId('library-source-row')

async function openAndGoto(): Promise<void> {
  render(
    <LibraryDialog
      open
      onClose={vi.fn()}
      placements={{}}
      onInsert={vi.fn()}
      onOpenAsDocument={vi.fn()}
      onPaintWith={vi.fn()}
      onAddToPalette={vi.fn()}
      sources={bindingFor(env)}
    />,
  )
  await waitFor(() => expect(sourceRows().length).toBeGreaterThan(0))
  fireEvent.click(sourceRows().find((r) => r.textContent?.includes('Hinges'))!)
  await waitFor(() => screen.getByRole('listbox', { name: /^Items in/ }))
  await waitFor(() => expect(screen.queryByText('Loading…')).not.toBeInTheDocument())
}

beforeEach(() => {
  env = makeEnv()
  fx.store = env.store
  fx.listeners.clear()
  localStorage.removeItem('hew.library.view')
  localStorage.removeItem('hew.settings.checkSources')
  Object.defineProperty(window, 'matchMedia', { value: undefined, configurable: true, writable: true })
})
afterEach(() => env.assertClean())

describe('the header after a failed refresh', () => {
  it('shows the older good check, not "just now", and the banner keeps the status', async () => {
    env.clock.t = Date.now() - 30 * 60_000
    await env.addSource(SRC, { name: 'Hinges', items: parts(2) })
    env.clock.advance(30 * 60_000)
    await openAndGoto()
    env.server.fail((u) => u.includes('example.test') && !u.includes('catalog'), 'http_status', { status: 503 })
    fireEvent.click(screen.getByTestId('source-refresh'))
    const banner = await screen.findByTestId('source-unreachable')
    expect(await screen.findByTestId('source-message')).toHaveTextContent('(503)')
    await waitFor(() => expect(banner).toHaveTextContent('(503)'))
    expect(screen.getByTestId('source-checked')).toHaveTextContent('Checked 30 minutes ago')
    expect(screen.getByTestId('source-checked')).not.toHaveTextContent('just now')
  })

  it('a source that never had a good check says "Not checked yet"', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    const entry = env.store.entryFor(SRC)!
    const { lastGoodAt: _drop, ...rest } = entry
    await env.store.writeSources(SRC, { ...rest, lastCheck: { at: env.clock.iso(), ok: false, code: 'network' } })
    await openAndGoto()
    expect(screen.getByTestId('source-checked')).toHaveTextContent('Not checked yet')
  })
})
