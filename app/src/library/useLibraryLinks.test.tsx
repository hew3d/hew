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

  it('checks while a part is open: an edit inside it reads as changed here', async () => {
    vi.mocked(libraryStore).mockReturnValue(store({ 'a.hew': enc(summary('a', { '7': 'p1:t' })) }, reads))
    let rows: Array<Record<string, unknown>> = [{ ...ROW, state: 'in_step' }]
    const scene = {
      library_status_json: () => JSON.stringify({ links: rows, itemPrints: {} }),
    } as unknown as WasmScene
    const { result, rerender } = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), { initialProps: options(scene) })
    await waitFor(() => expect(result.current.links).toHaveLength(1))

    // A part is open and the first check has run; now only an edit (docRev)
    // changes anything, so the debounced pass is what must pick it up.
    rerender(options(scene, { sessionOpen: true }))
    await act(async () => {
      await new Promise((r) => setTimeout(r, 100))
    })
    rows = [{ ...ROW, state: 'edited_here' }]
    expect(result.current.byDefinition.get('1')?.state).toBe('in_step')
    rerender(options(scene, { sessionOpen: true, docRev: 1 }))
    await waitFor(() => expect(result.current.byDefinition.get('1')?.state).toBe('edited_here'), { timeout: 4000 })
    // The update can still read the item's file while the part is open.
    expect(Object.keys(await result.current.readItems(['a']))).toEqual(['a'])
  })

  it('an edit makes one kernel pass and lists the folder no more; a Library change still lists it', async () => {
    const files: Files = { 'a.hew': enc(summary('a', { '7': 'p1:t' })) }
    let lists = 0
    let notify: () => void = () => {}
    vi.mocked(libraryStore).mockReturnValue({
      ...store(files, reads),
      list: async () => {
        lists += 1
        return Object.entries(files).map(([fileName, b]) => ({ fileName, size: b.length, mtimeMs: 5 }))
      },
      subscribe: (cb: () => void) => {
        notify = cb
        return () => {}
      },
    } as unknown as LibraryStore)
    const { scene, calls } = fakeScene([ROW])
    let probes = 0
    const counted = {
      library_status_json: (library: unknown) => {
        if (Object.keys(library as object).length === 0) probes += 1
        return scene.library_status_json(library as never)
      },
    } as unknown as WasmScene
    const { result, rerender } = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), { initialProps: options(counted) })
    await waitFor(() => expect(result.current.links).toHaveLength(1))
    expect([lists, probes, calls.length]).toEqual([1, 1, 1])

    rerender(options(counted, { docRev: 1 }))
    await waitFor(() => expect(calls).toHaveLength(2), { timeout: 4000 })
    expect([lists, probes]).toEqual([1, 1])

    act(() => notify())
    await waitFor(() => expect(lists).toBe(2), { timeout: 4000 })
  })

  it('an edit-triggered pass does not discard a full scan that is still reading', async () => {
    // The first scan finishes, so items are known. The file then changes on
    // disk and a full scan starts; its read is parked while an edit's quiet
    // pass fires. The new prints must still land.
    let prints = 'p1:old'
    let fileBytes = enc(summary('a', { '7': prints }))
    let mtime = 5
    let gate: (() => void) | null = null
    let park = false
    let notify: () => void = () => {}
    vi.mocked(libraryStore).mockReturnValue({
      available: () => true,
      list: async () => [{ fileName: 'a.hew', size: fileBytes.length, mtimeMs: mtime }],
      read: async () => {
        if (park) await new Promise<void>((r) => (gate = r))
        return fileBytes
      },
      subscribe: (cb: () => void) => {
        notify = cb
        return () => {}
      },
    } as unknown as LibraryStore)
    const scene = {
      library_status_json: (library: Record<string, { prints?: Record<string, string> }>) => {
        const got = library.a?.prints?.['7']
        return JSON.stringify({ links: [{ ...ROW, theirs: got ?? 'p1:t', state: got === 'p1:new' ? 'library_newer' : 'in_step' }], itemPrints: {} })
      },
    } as unknown as WasmScene
    const { result, rerender } = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), { initialProps: options(scene) })
    await waitFor(() => expect(result.current.links[0]?.state).toBe('in_step'))

    prints = 'p1:new'
    fileBytes = enc(summary('a', { '7': prints }))
    mtime = 6
    park = true
    act(() => notify())
    await waitFor(() => expect(gate).not.toBeNull(), { timeout: 4000 })
    // The edit's quiet pass runs while the full scan's read is parked.
    rerender(options(scene, { docRev: 1 }))
    await act(async () => {
      await new Promise((r) => setTimeout(r, 1600))
    })
    park = false
    act(() => gate!())
    await waitFor(() => expect(result.current.links[0]?.state).toBe('library_newer'), { timeout: 4000 })
  })

  it('an edit that makes the answer name a new item falls back to a full scan', async () => {
    vi.mocked(libraryStore).mockReturnValue(
      store({ 'a.hew': enc(summary('a', { '7': 'p1:t' })), 'b.hew': enc(summary('b', { '7': 'p1:u' })) }, reads),
    )
    let rows: Array<Record<string, unknown>> = [ROW]
    const requests: unknown[] = []
    const scene = {
      library_status_json: (library: unknown) => {
        requests.push(library)
        return JSON.stringify({ links: rows, itemPrints: {} })
      },
    } as unknown as WasmScene
    const { result, rerender } = renderHook((o: UseLibraryLinksOptions) => useLibraryLinks(o), { initialProps: options(scene) })
    await waitFor(() => expect(result.current.links).toHaveLength(1))
    rows = [ROW, { ...ROW, definition: '2', item: 'b', theirs: 'p1:u' }]
    rerender(options(scene, { docRev: 1 }))
    await waitFor(() => expect(result.current.links).toHaveLength(2), { timeout: 4000 })
    expect(result.current.itemRef('b')?.fileName).toBe('b.hew')
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
