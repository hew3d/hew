/**
 * A followed source the catalog removed can still be unfollowed (turning
 * following off needs no network); a removed source that is not followed
 * keeps Follow disabled. Harness as LibrarySources.review.test.tsx.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { LibraryDialog } from './LibraryDialog'
import type { SourcesBinding } from '../library/sourcesDeps'
import { makeEnv, parseFake, parts, SRC, type Env, type FakeStore } from '../library/sourcesTestKit'

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
vi.mock('../library/nativeChrome', () => ({
  nativeConfirm: async () => true,
  popupNativeMenu: async () => null,
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
      return {
        format_version: 1,
        objects: 1,
        materials: 0,
        components: 1,
        instances: 1,
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
void parseFake

let env: Env

function bindingFor(e: Env): SourcesBinding {
  return { sources: e.sources, available: () => e.fetcher.on, subscribe: () => () => {} }
}

beforeEach(() => {
  env = makeEnv()
  fx.store = env.store
  fx.listeners.clear()
  localStorage.removeItem('hew.library.view')
  localStorage.removeItem('hew.settings.checkSources')
  Object.defineProperty(window, 'matchMedia', { value: undefined, configurable: true, writable: true })
})
afterEach(() => {
  env.assertClean()
})

async function openRemovedSource(follow: boolean): Promise<void> {
  await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
  if (follow) await env.sources.follow(SRC)
  env.server.publishCatalog({ sources: [], removed: [{ source: SRC, severity: 'removed', reason: 'Copied.' }] })
  await env.sources.catalog()
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
  await waitFor(() => expect(screen.queryAllByTestId('library-source-row').length).toBeGreaterThan(0))
  fireEvent.click(screen.queryAllByTestId('library-source-row').find((r) => r.textContent?.includes('Hinges'))!)
  await screen.findByTestId('source-removed')
}

describe('Follow on a source the catalog removed', () => {
  it('a followed one can be unfollowed', async () => {
    await openRemovedSource(true)
    const follow = screen.getByTestId('source-follow')
    expect(follow).toBeEnabled()
    fireEvent.click(follow)
    await waitFor(() => expect(env.store.sourcesFile?.sources.find((s) => s.source === SRC)?.followed).toBe(false))
  })

  it('one that is not followed keeps Follow disabled', async () => {
    await openRemovedSource(false)
    expect(screen.getByTestId('source-follow')).toBeDisabled()
  })
})
