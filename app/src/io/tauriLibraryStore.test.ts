// @vitest-environment jsdom
/**
 * tauriLibraryStore.test - the desktop store's sources, source-cache and
 * edit-registry methods against a mocked Tauri bridge (package S9, design
 * 7.3). Item methods keep their own coverage elsewhere.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'

const mockInvoke = vi.hoisted(() => vi.fn())
const mockListen = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke: mockInvoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: mockListen }))

import { makeTauriLibraryStore } from './tauriLibraryStore'

const handlers = new Map<string, () => void>()

beforeEach(() => {
  mockInvoke.mockReset()
  mockListen.mockReset()
  handlers.clear()
  mockListen.mockImplementation(async (name: string, cb: () => void) => {
    handlers.set(name, cb)
    return () => handlers.delete(name)
  })
})

const buf = (...v: number[]) => new Uint8Array(v).buffer

describe('tauri store: sources file', () => {
  it('readSources invokes library_sources_read and returns bytes', async () => {
    mockInvoke.mockResolvedValue(buf(1, 2, 3))
    const out = await makeTauriLibraryStore().readSources!()
    expect(mockInvoke).toHaveBeenCalledWith('library_sources_read')
    expect(Array.from(out!)).toEqual([1, 2, 3])
  })

  it('readSources maps the not_found sentinel (string or Error) to null, and rethrows others', async () => {
    const store = makeTauriLibraryStore()
    mockInvoke.mockRejectedValueOnce('not_found')
    expect(await store.readSources!()).toBeNull()
    mockInvoke.mockRejectedValueOnce(new Error('not_found'))
    expect(await store.readSources!()).toBeNull()
    mockInvoke.mockRejectedValueOnce('disk on fire')
    await expect(store.readSources!()).rejects.toBe('disk on fire')
  })

  it('writeSources sends one source and its entry (null removes) to library_sources_write', async () => {
    const store = makeTauriLibraryStore()
    mockInvoke.mockResolvedValue(undefined)
    const entry = { source: 'github:a/a', kind: 'github', name: 'A' }
    await store.writeSources!('github:a/a', entry)
    await store.writeSources!('github:a/a', null)
    expect(mockInvoke.mock.calls).toEqual([
      ['library_sources_write', { source: 'github:a/a', entry }],
      ['library_sources_write', { source: 'github:a/a', entry: null }],
    ])
  })
})

describe('tauri store: source cache', () => {
  it('readSourceCache / writeSourceCache map to the cache commands, not_found is null', async () => {
    const store = makeTauriLibraryStore()
    mockInvoke.mockResolvedValueOnce(buf(9))
    expect(Array.from((await store.readSourceCache!('catalog'))!)).toEqual([9])
    mockInvoke.mockRejectedValueOnce('not_found')
    expect(await store.readSourceCache!('catalog')).toBeNull()
    mockInvoke.mockResolvedValueOnce(undefined)
    await store.writeSourceCache!('catalog', new Uint8Array([4, 5]))
    expect(mockInvoke.mock.calls).toEqual([
      ['library_source_cache_read', { key: 'catalog' }],
      ['library_source_cache_read', { key: 'catalog' }],
      ['library_source_cache_write', { key: 'catalog', contents: [4, 5] }],
    ])
  })
})

describe('tauri store: source cache removal', () => {
  it('removeSourceCache maps to the remove command', async () => {
    const store = makeTauriLibraryStore()
    mockInvoke.mockResolvedValueOnce(undefined)
    await store.removeSourceCache('thumb-0123abcd')
    expect(mockInvoke.mock.calls).toEqual([['library_source_cache_remove', { key: 'thumb-0123abcd' }]])
  })
})

describe('tauri store: notices', () => {
  it('library-sources-changed reaches subscribeSources only; library-changed reaches subscribe only', async () => {
    const store = makeTauriLibraryStore()
    const items = vi.fn()
    const sources = vi.fn()
    store.subscribe(items)
    store.subscribeSources!(sources)
    await vi.waitFor(() => expect(handlers.has('library-sources-changed')).toBe(true))
    await vi.waitFor(() => expect(handlers.has('library-changed')).toBe(true))

    handlers.get('library-sources-changed')!()
    expect(sources).toHaveBeenCalledTimes(1)
    expect(items).not.toHaveBeenCalled()

    handlers.get('library-changed')!()
    expect(items).toHaveBeenCalledTimes(1)
    expect(sources).toHaveBeenCalledTimes(1)
  })

  it('the unsubscribe function stops subscribeSources', async () => {
    const store = makeTauriLibraryStore()
    const sources = vi.fn()
    const off = store.subscribeSources!(sources)
    await vi.waitFor(() => expect(handlers.has('library-sources-changed')).toBe(true))
    off()
    handlers.get('library-sources-changed')?.()
    expect(sources).not.toHaveBeenCalled()
  })
})

describe('tauri store: edit registry', () => {
  it('editingNames reads library_edit_list', async () => {
    mockInvoke.mockResolvedValue(['Models/a-000001.hew'])
    expect(await makeTauriLibraryStore().editingNames!()).toEqual(['Models/a-000001.hew'])
    expect(mockInvoke).toHaveBeenCalledWith('library_edit_list')
  })

  it('registerEdit / unregisterEdit invoke the register and unregister commands', async () => {
    const store = makeTauriLibraryStore()
    mockInvoke.mockResolvedValue(undefined)
    await store.registerEdit!('Models/a-000001.hew')
    await store.unregisterEdit!('Models/a-000001.hew')
    expect(mockInvoke.mock.calls).toEqual([
      ['library_edit_register', { name: 'Models/a-000001.hew' }],
      ['library_edit_unregister', { name: 'Models/a-000001.hew' }],
    ])
  })
})
