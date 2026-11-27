/**
 * webLibrarySources.test - package S9 (design 6.5, 7.3, 4.1): the web
 * store's sources file, source cache, own notice, `migrateInto` copy,
 * tightened item-name rule and Web-Lock edit registry, run against BOTH
 * backends (browser storage / OPFS, and a bound folder).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { makeWebLibraryStore } from './webLibraryStore'
import { FakeDir } from '../test/opfsFake'
import { fakeLocks } from '../test/locksFake'

const enc = new TextEncoder()
const dec = new TextDecoder()
const bytes = (...v: number[]) => new Uint8Array(v)

function entry(source: string, name = 'N') {
  return {
    source,
    kind: 'github',
    name,
    followed: false,
    addedAt: '2026-11-23T00:00:00Z',
    lastCheck: { at: '2026-11-23T00:00:00Z', ok: true, code: null },
    skipped: [] as string[],
    kept: {} as Record<string, string>,
  }
}

/** A channel that, unlike the shared fake, carries message data and records
 * what was posted. `emit(data)` delivers `{ data }` the way a real
 * BroadcastChannel message event does; `emit()` is a bare notice. */
function dataChannel() {
  const handlers = new Set<(e: { data?: unknown }) => void>()
  const posted: unknown[] = []
  const channel = {
    addEventListener: (_t: string, fn: (e: { data?: unknown }) => void) => handlers.add(fn),
    postMessage: (m: unknown) => void posted.push(m),
  } as unknown as BroadcastChannel
  return {
    channel,
    posted,
    emit: (data?: unknown) => handlers.forEach((h) => h({ data })),
  }
}

/** The message type of a posted value, whether sent bare or as `{ type }`. */
const typeOf = (m: unknown): unknown =>
  m !== null && typeof m === 'object' ? (m as { type?: unknown }).type : m

interface Backend {
  label: string
  /** Root directory the library lives in, once the store is set up. */
  root: () => FakeDir
  make: (channel?: BroadcastChannel | null) => Promise<ReturnType<typeof makeWebLibraryStore>>
}

function backends(): Backend[] {
  return [
    (() => {
      const opfs = new FakeDir('opfs-root')
      return {
        label: 'browser storage (OPFS)',
        root: () => opfs.dirs.get('library')!,
        make: async (channel = null) =>
          makeWebLibraryStore({
            idb: undefined,
            opfsRoot: async () => opfs as unknown as FileSystemDirectoryHandle,
            pickDirectory: null,
            requestPersist: async () => true,
            channel,
          }),
      }
    })(),
    (() => {
      const folder = new FakeDir('Bound')
      return {
        label: 'a bound folder',
        root: () => folder,
        make: async (channel = null) => {
          const store = makeWebLibraryStore({
            idb: undefined,
            opfsRoot: async () => new FakeDir('unused-opfs') as unknown as FileSystemDirectoryHandle,
            pickDirectory: async () => folder as unknown as FileSystemDirectoryHandle,
            requestPersist: async () => true,
            channel,
          })
          await store.chooseFolder()
          return store
        },
      }
    })(),
  ]
}

afterEach(() => vi.unstubAllGlobals())

describe.each([0, 1])('sources on backend %i (0 = browser storage, 1 = bound folder)', (index) => {
  // A fresh, empty backend per test: no state leaks between cases.
  let b: Backend
  beforeEach(() => {
    b = backends()[index]
  })

  it('readSources is null before anything is written', async () => {
    const store = await b.make()
    expect(await store.readSources!()).toBeNull()
  })

  it('writeSources / readSources round trip in `.sources.json` at the library root', async () => {
    const store = await b.make()
    await store.writeSources!('github:a/a', entry('github:a/a', 'A'))
    expect(b.root().files.has('.sources.json')).toBe(true)
    const parsed = JSON.parse(dec.decode((await store.readSources!())!))
    expect(parsed.hew).toBe(1)
    expect(parsed.sources).toEqual([entry('github:a/a', 'A')])
  })

  it('a write changes ONE source entry: others survive, null removes', async () => {
    const store = await b.make()
    await store.writeSources!('github:a/a', entry('github:a/a', 'A'))
    await store.writeSources!('github:b/b', entry('github:b/b', 'B'))
    const b2 = { ...entry('github:b/b', 'B'), followed: true, skipped: ['id-1'] }
    await store.writeSources!('github:b/b', b2)
    let parsed = JSON.parse(dec.decode((await store.readSources!())!))
    expect(parsed.sources).toEqual([entry('github:a/a', 'A'), b2])
    await store.writeSources!('github:a/a', null)
    parsed = JSON.parse(dec.decode((await store.readSources!())!))
    expect(parsed.sources).toEqual([b2])
  })

  it('a write re-reads the file: an entry another tab added meanwhile survives', async () => {
    const store = await b.make()
    await store.writeSources!('github:a/a', entry('github:a/a', 'A'))
    // Another tab adds C straight to the shared file.
    const file = JSON.parse(dec.decode((await store.readSources!())!))
    file.sources.push(entry('github:c/c', 'C'))
    b.root().files.set('.sources.json', { bytes: enc.encode(JSON.stringify(file)), mtime: 1 })
    await store.writeSources!('github:b/b', entry('github:b/b', 'B'))
    const refs = JSON.parse(dec.decode((await store.readSources!())!)).sources.map(
      (s: { source: string }) => s.source,
    )
    expect(refs).toEqual(['github:a/a', 'github:c/c', 'github:b/b'])
  })

  it('a corrupt `.sources.json` is refused, never overwritten', async () => {
    const store = await b.make()
    await store.writeSources!('github:a/a', entry('github:a/a'))
    b.root().files.set('.sources.json', { bytes: enc.encode('{broken'), mtime: 1 })
    await expect(store.writeSources!('github:b/b', entry('github:b/b'))).rejects.toThrow()
    expect(dec.decode(b.root().files.get('.sources.json')!.bytes)).toBe('{broken')
  })

  it('an entry naming a different source than the argument is refused', async () => {
    const store = await b.make()
    await expect(store.writeSources!('github:other/other', entry('github:a/a'))).rejects.toThrow()
    expect(b.root().files.has('.sources.json')).toBe(false)
  })

  it('never raises the item notice (subscribe callback, broadcast)', async () => {
    const ch = dataChannel()
    const store = await b.make(ch.channel)
    const items = vi.fn()
    store.subscribe(items)
    const before = ch.posted.length
    await store.writeSources!('github:a/a', entry('github:a/a'))
    await store.writeSourceCache!('catalog', bytes(1))
    expect(await store.readSources!()).not.toBeNull()
    await store.readSourceCache!('catalog')
    expect(items).not.toHaveBeenCalled()
    expect(ch.posted.slice(before).map(typeOf)).not.toContain('changed')
  })

  it('raises its own notice: local subscribeSources callback and a library-sources-changed broadcast', async () => {
    const ch = dataChannel()
    const store = await b.make(ch.channel)
    const sources = vi.fn()
    store.subscribeSources!(sources)
    const before = ch.posted.length
    await store.writeSources!('github:a/a', entry('github:a/a'))
    expect(sources).toHaveBeenCalledTimes(1)
    expect(ch.posted.slice(before).map(typeOf)).toEqual(['library-sources-changed'])
  })

  it('hears another tab: a library-sources-changed message reaches subscribeSources only; a bare message reaches subscribe only', async () => {
    const ch = dataChannel()
    const store = await b.make(ch.channel)
    const items = vi.fn()
    const sources = vi.fn()
    store.subscribe(items)
    store.subscribeSources!(sources)
    ch.emit('library-sources-changed')
    expect(sources).toHaveBeenCalledTimes(1)
    expect(items).not.toHaveBeenCalled()
    ch.emit('changed')
    expect(items).toHaveBeenCalledTimes(1)
    expect(sources).toHaveBeenCalledTimes(1)
  })

  it('subscribeSources returns an unsubscribe', async () => {
    const store = await b.make()
    const sources = vi.fn()
    const off = store.subscribeSources!(sources)
    off()
    await store.writeSources!('github:a/a', entry('github:a/a'))
    expect(sources).not.toHaveBeenCalled()
  })

  it('source cache: miss is null, round trip lives under `.sources/<key>`, bad keys reject', async () => {
    const store = await b.make()
    expect(await store.readSourceCache!('catalog')).toBeNull()
    await store.writeSourceCache!('catalog', bytes(7, 8))
    await store.writeSourceCache!('index-0123abcd', bytes(1))
    expect(Array.from((await store.readSourceCache!('catalog'))!)).toEqual([7, 8])
    expect([...b.root().dirs.get('.sources')!.files.keys()].sort()).toEqual(['catalog', 'index-0123abcd'])
    for (const bad of ['../escape', 'a/b', '..', '', 'UPPER', 'a.b', '.hidden', 'x'.repeat(97)]) {
      await expect(store.writeSourceCache!(bad, bytes(1)), bad).rejects.toThrow(/invalid source cache key/)
      await expect(store.readSourceCache!(bad), bad).rejects.toThrow(/invalid source cache key/)
    }
  })

  it('removeSourceCache deletes one entry, tolerates an absent one and a missing cache, rejects bad keys', async () => {
    const store = await b.make()
    await store.removeSourceCache('thumb-0123abcd') // no cache folder yet
    await store.writeSourceCache!('catalog', bytes(1))
    await store.writeSourceCache!('thumb-0123abcd', bytes(2))
    await store.removeSourceCache('thumb-0123abcd')
    await store.removeSourceCache('thumb-0123abcd') // already gone
    expect([...b.root().dirs.get('.sources')!.files.keys()]).toEqual(['catalog'])
    for (const bad of ['../escape', 'a/b', '..', '', 'UPPER', 'a.b']) {
      await expect(store.removeSourceCache(bad), bad).rejects.toThrow(/invalid source cache key/)
    }
  })

  it('the sources file, the cache and dot-named files never appear in list()', async () => {
    const store = await b.make()
    await store.write('Models/real-000001.hew', bytes(1))
    await store.writeSources!('github:a/a', entry('github:a/a'))
    await store.writeSourceCache!('catalog', bytes(1))
    b.root().files.set('.hidden.hew', { bytes: bytes(1), mtime: 1 })
    expect((await store.list()).map((e) => e.fileName)).toEqual(['Models/real-000001.hew'])
  })

  it('item names: a leading dot and a missing `.hew` are refused, like the Rust rule (4.1)', async () => {
    const store = await b.make()
    for (const bad of [
      '.sources.json',
      '.hidden.hew',
      'Components/.sources.json',
      'Models/.x.hew',
      '.sources/x.hew',
      'notes.txt',
      'Models/readme',
      'chair',
    ]) {
      await expect(store.write(bad, bytes(1)), bad).rejects.toThrow(/invalid library item name/)
    }
    expect(b.root().files.has('.sources.json')).toBe(false)
    // Valid names still work, upper-case extension included (Rust accepts it).
    await store.write('Models/ok-000001.hew', bytes(1))
    await store.write('UPPER.HEW', bytes(1))
    expect((await store.list()).map((e) => e.fileName).sort()).toEqual(['Models/ok-000001.hew', 'UPPER.HEW'])
  })
})

describe('migrateInto copies the sources list and cache', () => {
  async function setup(pre?: (folder: FakeDir) => void) {
    const opfs = new FakeDir('opfs-root')
    const folder = new FakeDir('Picked')
    pre?.(folder)
    const make = () =>
      makeWebLibraryStore({
        idb: undefined,
        opfsRoot: async () => opfs as unknown as FileSystemDirectoryHandle,
        pickDirectory: async () => folder as unknown as FileSystemDirectoryHandle,
        requestPersist: async () => true,
        channel: null,
      })
    const store = make()
    await store.write('Models/kept-000001.hew', bytes(1))
    await store.writeSources!('github:a/a', entry('github:a/a', 'A'))
    await store.writeSourceCache!('catalog', bytes(5, 5))
    await store.writeSourceCache!('thumb-0123abcd', bytes(6))
    return { store, folder }
  }

  it('binding a folder carries `.sources.json` and `.sources/` along with the items', async () => {
    const { store, folder } = await setup()
    await store.chooseFolder()
    expect(folder.files.has('.sources.json')).toBe(true)
    expect([...folder.dirs.get('.sources')!.files.keys()].sort()).toEqual(['catalog', 'thumb-0123abcd'])
    expect(JSON.parse(dec.decode((await store.readSources!())!)).sources).toEqual([entry('github:a/a', 'A')])
    expect(Array.from((await store.readSourceCache!('catalog'))!)).toEqual([5, 5])
    expect((await store.list()).map((e) => e.fileName)).toEqual(['Models/kept-000001.hew'])
  })

  it('an existing `.sources.json` or cache file in the target wins; nothing is overwritten', async () => {
    const theirs = JSON.stringify({ hew: 1, sources: [entry('github:t/t', 'Theirs')] })
    const { store, folder } = await setup(async (f) => {
      f.files.set('.sources.json', { bytes: enc.encode(theirs), mtime: 1 })
      const d = await f.getDirectoryHandle('.sources', { create: true })
      d.files.set('catalog', { bytes: bytes(9), mtime: 1 })
    })
    await store.chooseFolder()
    expect(dec.decode(folder.files.get('.sources.json')!.bytes)).toBe(theirs)
    expect(Array.from(folder.dirs.get('.sources')!.files.get('catalog')!.bytes)).toEqual([9])
    // The cache file the target lacked is still copied.
    expect(folder.dirs.get('.sources')!.files.has('thumb-0123abcd')).toBe(true)
  })
})

describe('web edit registry: Web Lock hew-library-edit:<file>', () => {
  it('editingNames lists the held edit locks by item name, ignoring other locks', async () => {
    const locks = fakeLocks()
    vi.stubGlobal('navigator', { locks })
    const store = await backends()[0].make()
    expect(await store.editingNames!()).toEqual([])

    let releaseA!: () => void
    let releaseB!: () => void
    void locks.request('hew-library-edit:Models/a-000001.hew', () => new Promise<void>((r) => (releaseA = r)))
    void locks.request('hew-library-edit:Components/b-000002.hew', () => new Promise<void>((r) => (releaseB = r)))
    void locks.request('hew-library-source:github:a/a', () => new Promise<void>(() => {}))
    await Promise.resolve()
    expect((await store.editingNames!()).sort()).toEqual(['Components/b-000002.hew', 'Models/a-000001.hew'])

    releaseA()
    await vi.waitFor(async () => expect(await store.editingNames!()).toEqual(['Components/b-000002.hew']))
    releaseB()
    await vi.waitFor(async () => expect(await store.editingNames!()).toEqual([]))
  })

  it('without Web Locks support the answer is an empty list, not an error', async () => {
    vi.stubGlobal('navigator', {})
    const store = await backends()[0].make()
    expect(await store.editingNames!()).toEqual([])
  })
})
