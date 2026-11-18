/**
 * useLibraryLinks — what the check reads from the Library and hands the
 * kernel (v1.5 OW-2): manifest prints when the item has them, the file's
 * bytes when it does not (once), each file read once per session, nothing at
 * all when the check is off or the model links nothing, and "Later" per print.
 */
import { act, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useLibraryLinks, type UseLibraryLinksOptions } from './useLibraryLinks'
import { libraryStore, type LibraryStore } from '../io/libraryStore'
import type { Scene as WasmScene } from '../wasm/loader'
import type { LibraryItemSummary } from './types'

vi.mock('../io/libraryStore', () => ({ libraryStore: vi.fn() }))
vi.mock('./itemFiles', () => ({
  readItemSummary: vi.fn(async (bytes: Uint8Array): Promise<LibraryItemSummary> => JSON.parse(new TextDecoder().decode(bytes))),
  sha256Hex: vi.fn(async (bytes: Uint8Array) => `hash:${bytes.length}`),
}))

const enc = (o: unknown): Uint8Array => new TextEncoder().encode(JSON.stringify(o))

function summary(id: string, prints?: Record<string, string>): LibraryItemSummary {
  return {
    format_version: 1, objects: 0, materials: 0, components: 1, instances: 1, groups: 0, world_sketches: 0, annotations: 0, guides: 0,
    first_component_name: id, first_component_sid: '7', first_root_name: null, material_entries: [], profile_chain_only: false,
    doc_attrs: { 'hew.library': { id, ...(prints !== undefined ? { prints } : {}) } },
  }
}

interface Files {
  [fileName: string]: Uint8Array
}

function store(files: Files, reads: string[]): LibraryStore {
  return {
    available: () => true,
    list: async () => Object.entries(files).map(([fileName, b]) => ({ fileName, size: b.length, mtimeMs: 5 })),
    read: async (f: string) => {
      reads.push(f)
      return files[f]!
    },
    subscribe: () => () => {},
  } as unknown as LibraryStore
}

function fakeScene(rows: Array<Record<string, unknown>>, itemPrints: Record<string, unknown> = {}): { scene: WasmScene; calls: unknown[] } {
  const calls: unknown[] = []
  const scene = {
    library_status_json: (library: unknown) => {
      // The empty probe only learns which items the model names.
      if (Object.keys(library as object).length > 0) calls.push(library)
      return JSON.stringify({ links: rows, itemPrints })
    },
  } as unknown as WasmScene
  return { scene, calls }
}

const ROW = { definition: '1', item: 'a', state: 'library_newer', copies: 1, locked: false, mine: 'p1:m', theirs: 'p1:t', name: 'Leg' }

function options(scene: WasmScene | null, over: Partial<UseLibraryLinksOptions> = {}): UseLibraryLinksOptions {
  return { scene, docRev: 0, docGeneration: 0, enabled: true, openedKey: 0, hasLinks: true, ...over }
}

let reads: string[]
beforeEach(() => {
  reads = []
})

describe('useLibraryLinks', () => {
  it('hands the kernel the prints a manifest carries, without reading the file twice', async () => {
    vi.mocked(libraryStore).mockReturnValue(store({ 'Components/a.hew': enc(summary('a', { '7': 'p1:t' })) }, reads))
    const { scene, calls } = fakeScene([ROW])
    const { result, rerender } = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), { initialProps: options(scene) })
    await waitFor(() => expect(result.current.links).toHaveLength(1))
    expect(calls[0]).toEqual({ a: { prints: { '7': 'p1:t' } } })
    expect(reads).toEqual(['Components/a.hew'])
    // The panel opens: checked again, the cached file is not read again.
    rerender(options(scene, { openedKey: 2 }))
    await waitFor(() => expect(calls).toHaveLength(2))
    expect(reads).toEqual(['Components/a.hew'])
  })

  it('passes bytes for an item saved before prints existed, then the prints the kernel worked out', async () => {
    vi.mocked(libraryStore).mockReturnValue(store({ 'a.hew': enc(summary('a')) }, reads))
    const { scene, calls } = fakeScene([ROW], { a: { '7': 'p1:t' } })
    const { result, rerender } = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), { initialProps: options(scene) })
    await waitFor(() => expect(result.current.links).toHaveLength(1))
    expect(Object.keys((calls[0] as Record<string, Record<string, unknown>>).a!)).toEqual(['bytes'])
    rerender(options(scene, { openedKey: 1 }))
    await waitFor(() => expect(calls).toHaveLength(2))
    expect(calls[1]).toEqual({ a: { prints: { '7': 'p1:t' } } })
  })

  it('says nothing when the check is off or the model links nothing', async () => {
    vi.mocked(libraryStore).mockReturnValue(store({ 'a.hew': enc(summary('a', { '7': 'p1:t' })) }, reads))
    const { scene, calls } = fakeScene([ROW])
    const off = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), { initialProps: options(scene, { enabled: false }) })
    const none = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), { initialProps: options(scene, { hasLinks: false }) })
    await act(async () => {
      await Promise.resolve()
    })
    expect(calls).toHaveLength(0)
    expect(reads).toEqual([])
    expect(off.result.current.links).toEqual([])
    expect(none.result.current.pending).toEqual([])
  })

  it('"Later" hides what is newer now, and a newer print brings the notice back', async () => {
    vi.mocked(libraryStore).mockReturnValue(store({ 'a.hew': enc(summary('a', { '7': 'p1:t' })) }, reads))
    let rows = [ROW]
    const calls: unknown[] = []
    const scene = {
      library_status_json: (l: unknown) => {
        if (Object.keys(l as object).length > 0) calls.push(l)
        return JSON.stringify({ links: rows, itemPrints: {} })
      },
    } as unknown as WasmScene
    const { result, rerender } = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), { initialProps: options(scene) })
    await waitFor(() => expect(result.current.pending).toHaveLength(1))
    act(() => result.current.dismiss())
    expect(result.current.pending).toEqual([])
    rows = [{ ...ROW, theirs: 'p1:t2' }]
    rerender(options(scene, { openedKey: 1 }))
    await waitFor(() => expect(result.current.pending).toHaveLength(1))
  })

  it('knows which file an item lives in, and reads it fresh for an update', async () => {
    vi.mocked(libraryStore).mockReturnValue(store({ 'Components/a.hew': enc(summary('a', { '7': 'p1:t' })) }, reads))
    const { scene } = fakeScene([ROW])
    const { result } = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), { initialProps: options(scene) })
    await waitFor(() => expect(result.current.links).toHaveLength(1))
    expect(result.current.itemRef('a')).toMatchObject({ fileName: 'Components/a.hew', name: 'a' })
    const items = await result.current.readItems(['a', 'a', 'nope'])
    expect(Object.keys(items)).toEqual(['a'])
    expect(items.a!.contentHash).toMatch(/^hash:/)
  })

  it('"Later" is for one document: a new document starts with the notice back', async () => {
    vi.mocked(libraryStore).mockReturnValue(store({ 'a.hew': enc(summary('a', { '7': 'p1:t' })) }, reads))
    const { scene } = fakeScene([ROW])
    const { result, rerender } = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), { initialProps: options(scene) })
    await waitFor(() => expect(result.current.pending).toHaveLength(1))
    act(() => result.current.dismiss())
    expect(result.current.pending).toEqual([])
    // Same part, same print, but File > New / Open replaced the document.
    rerender(options(scene, { docGeneration: 1 }))
    await waitFor(() => expect(result.current.pending).toHaveLength(1))
    // The scene object itself being replaced does the same.
    act(() => result.current.dismiss())
    expect(result.current.pending).toEqual([])
    const { scene: next } = fakeScene([ROW])
    rerender(options(next, { docGeneration: 1 }))
    await waitFor(() => expect(result.current.pending).toHaveLength(1))
  })

  it('reads only the files of the items the model links', async () => {
    // Several items in the Library, saved the way the app names them (id suffix); the model links one.
    const files: Files = {}
    for (const id of ['aaaaaa11', 'bbbbbb22', 'cccccc33', 'dddddd44']) {
      files[`Components/item-${id.slice(0, 6)}.hew`] = enc(summary(id, { '7': 'p1:t' }))
    }
    vi.mocked(libraryStore).mockReturnValue(store(files, reads))
    const { scene, calls } = fakeScene([{ ...ROW, item: 'cccccc33' }])
    const { result } = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), { initialProps: options(scene) })
    await waitFor(() => expect(result.current.links).toHaveLength(1))
    expect(reads).toEqual(['Components/item-cccccc.hew'])
    expect(calls[0]).toEqual({ cccccc33: { prints: { '7': 'p1:t' } } })
  })

  it('still finds a linked item whose file is not named the way the app names them', async () => {
    const files: Files = {
      'Components/one.hew': enc(summary('aaaaaa11', { '7': 'p1:t' })),
      'Components/two.hew': enc(summary('bbbbbb22', { '7': 'p1:t' })),
    }
    vi.mocked(libraryStore).mockReturnValue(store(files, reads))
    const { scene, calls } = fakeScene([{ ...ROW, item: 'bbbbbb22' }])
    const { result } = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), { initialProps: options(scene) })
    await waitFor(() => expect(result.current.links).toHaveLength(1))
    expect(Object.keys(calls[0] as object)).toEqual(['bbbbbb22'])
    expect(result.current.itemRef('bbbbbb22')?.fileName).toBe('Components/two.hew')
  })

  it('two files with one id resolve to the first in the Library listing, as the API does', async () => {
    const copy = enc(summary('a', { '7': 'p1:t' }))
    vi.mocked(libraryStore).mockReturnValue({
      ...store({ 'Components/first.hew': copy, 'Components/second.hew': copy }, reads),
      // The second is newer; the listing order, not the clock, decides.
      list: async () => [
        { fileName: 'Components/first.hew', size: copy.length, mtimeMs: 5 },
        { fileName: 'Components/second.hew', size: copy.length, mtimeMs: 99 },
      ],
    } as unknown as LibraryStore)
    const { scene } = fakeScene([ROW])
    const { result } = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), { initialProps: options(scene) })
    await waitFor(() => expect(result.current.links).toHaveLength(1))
    expect(result.current.itemRef('a')?.fileName).toBe('Components/first.hew')
    expect(reads).toEqual(['Components/first.hew'])
  })

  it('reports the item root definition the file names', async () => {
    vi.mocked(libraryStore).mockReturnValue(store({ 'a.hew': enc(summary('a', { '7': 'p1:t' })) }, reads))
    const { scene } = fakeScene([ROW])
    const { result } = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), { initialProps: options(scene) })
    await waitFor(() => expect(result.current.links).toHaveLength(1))
    expect(result.current.itemRef('a')?.rootSid).toBe('7')
  })

  it('keeps the last answer while a group or component is open (the kernel scopes its placements to the session)', async () => {
    vi.mocked(libraryStore).mockReturnValue(store({ 'a.hew': enc(summary('a', { '7': 'p1:t' })) }, reads))
    let rows: Array<Record<string, unknown>> = [ROW]
    const scene = {
      library_status_json: () => JSON.stringify({ links: rows, itemPrints: {} }),
    } as unknown as WasmScene
    const { result, rerender } = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), { initialProps: options(scene) })
    await waitFor(() => expect(result.current.pending).toHaveLength(1))

    // A part opens: the kernel now answers "nothing", and no placements show.
    rows = []
    rerender(options(scene, { sessionOpen: true, hasLinks: false, docRev: 1 }))
    await act(async () => {
      await new Promise((r) => setTimeout(r, 1500))
    })
    expect(result.current.links).toHaveLength(1)
    expect(result.current.pending).toHaveLength(1)
    expect(result.current.byDefinition.get('1')?.state).toBe('library_newer')
    // A forced check (an update just ran) keeps it too.
    await act(async () => {
      await result.current.refreshNow()
    })
    expect(result.current.links).toHaveLength(1)
    // The update can still read the item's file while the part is open.
    expect(Object.keys(await result.current.readItems(['a']))).toEqual(['a'])

    // Closed again: the answer is read afresh.
    rerender(options(scene, { sessionOpen: false, hasLinks: true, docRev: 1 }))
    await waitFor(() => expect(result.current.links).toHaveLength(0))
  })

  it('with the setting off: no check on its own and no bar, but a panel opening or a forced check still checks', async () => {
    vi.mocked(libraryStore).mockReturnValue(store({ 'a.hew': enc(summary('a', { '7': 'p1:t' })) }, reads))
    const { scene, calls } = fakeScene([ROW])
    const { result, rerender } = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), { initialProps: options(scene, { enabled: false }) })
    await act(async () => {
      await Promise.resolve()
    })
    // A document opening, an edit and the quiet that follows: nothing runs.
    rerender(options(scene, { enabled: false, docRev: 1 }))
    await act(async () => {
      await new Promise((r) => setTimeout(r, 1500))
    })
    expect(calls).toHaveLength(0)
    expect(result.current.links).toEqual([])

    // The Components panel opens: the check runs, the markers have their answer, the bar stays down.
    rerender(options(scene, { enabled: false, docRev: 1, openedKey: 2 }))
    await waitFor(() => expect(result.current.links).toHaveLength(1))
    expect(result.current.byDefinition.get('1')?.state).toBe('library_newer')
    expect(result.current.pending).toEqual([])
    expect(result.current.itemRef('a')?.fileName).toBe('a.hew')

    // The Save popover asks (refreshNow): it runs too.
    const before = calls.length
    await act(async () => {
      await result.current.refreshNow()
    })
    expect(calls.length).toBe(before + 1)

    // Turning it on brings the bar.
    rerender(options(scene, { enabled: true, docRev: 1, openedKey: 2 }))
    await waitFor(() => expect(result.current.pending).toHaveLength(1))
  })

  it('with the setting off and a panel open, a new document, an insert and an edit keep the markers current', async () => {
    vi.mocked(libraryStore).mockReturnValue(store({ 'a.hew': enc(summary('a', { '7': 'p1:t' })) }, reads))
    const { scene, calls } = fakeScene([ROW])
    const { result, rerender } = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), {
      initialProps: options(scene, { enabled: false, openedKey: 2 }),
    })
    await waitFor(() => expect(result.current.links).toHaveLength(1))
    const afterOpen = calls.length

    // An edit: the quiet debounce re-checks while the panel stays open.
    rerender(options(scene, { enabled: false, openedKey: 2, docRev: 1 }))
    await waitFor(() => expect(calls.length).toBeGreaterThan(afterOpen), { timeout: 3000 })

    // A new document: the markers are re-checked, not cleared.
    const second = fakeScene([ROW])
    rerender(options(second.scene, { enabled: false, openedKey: 2, docRev: 1, docGeneration: 1 }))
    await waitFor(() => expect(second.calls.length).toBeGreaterThan(0))
    await waitFor(() => expect(result.current.links).toHaveLength(1))

    // An insert (the model now names Library parts): checked at once.
    const third = fakeScene([ROW])
    rerender(options(third.scene, { enabled: false, openedKey: 2, docRev: 1, docGeneration: 1, hasLinks: false }))
    rerender(options(third.scene, { enabled: false, openedKey: 2, docRev: 1, docGeneration: 1, hasLinks: true }))
    await waitFor(() => expect(third.calls.length).toBeGreaterThan(0))
    expect(result.current.pending).toEqual([]) // never raises the bar

    // The panel closes: edits go quiet again.
    rerender(options(third.scene, { enabled: false, openedKey: 0, docRev: 2, docGeneration: 1 }))
    const quiet = third.calls.length
    await act(async () => {
      await new Promise((r) => setTimeout(r, 1500))
    })
    expect(third.calls.length).toBe(quiet)
  })
})
