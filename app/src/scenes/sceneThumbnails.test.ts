/**
 * sceneThumbnails — pure key/eviction-rule coverage plus an in-memory fake
 * `IDBFactory` exercising the real save/load/evict path end to end,
 * mirroring `io/recents.ts`'s own `recents.test.ts` fake (this repo has no
 * `fake-indexeddb` dependency — see that file's own doc comment on why a
 * hand-rolled fake is the established pattern here).
 */
import { describe, it, expect, vi } from 'vitest'
import {
  saveThumbnails,
  loadThumbnails,
  selectThumbnailEvictions,
  thumbnailKey,
  MAX_ENTRIES,
  type ScenesThumbnailEntry,
  type ThumbnailMeta,
} from './sceneThumbnails'

// ---------------------------------------------------------------------------
// thumbnailKey / selectThumbnailEvictions — pure logic
// ---------------------------------------------------------------------------

describe('thumbnailKey', () => {
  it('joins contentHash and sid with a colon', () => {
    expect(thumbnailKey('abc123', 7)).toBe('abc123:7')
  })

  it('is distinct for different sids under the same hash', () => {
    expect(thumbnailKey('abc123', 1)).not.toBe(thumbnailKey('abc123', 2))
  })
})

function meta(key: string, timestamp: number): ThumbnailMeta {
  return { key, timestamp }
}

describe('selectThumbnailEvictions', () => {
  it('evicts nothing under the cap', () => {
    const entries = [meta('a', 3), meta('b', 2), meta('c', 1)]
    expect(selectThumbnailEvictions(entries, 10)).toEqual([])
  })

  it('evicts the oldest entries once the cap is exceeded', () => {
    const entries = [meta('newest', 5), meta('mid', 4), meta('oldest', 3)]
    expect(selectThumbnailEvictions(entries, 2)).toEqual(['oldest'])
  })

  it('sorts by timestamp regardless of input order', () => {
    const entries = [meta('oldest', 1), meta('newest', 3), meta('mid', 2)]
    expect(selectThumbnailEvictions(entries, 1)).toEqual(['mid', 'oldest'])
  })

  it('uses the exported MAX_ENTRIES as the default cap', () => {
    const entries = Array.from({ length: MAX_ENTRIES + 1 }, (_, i) => meta(`e${i}`, i))
    expect(selectThumbnailEvictions(entries)).toEqual(['e0'])
  })
})

// ---------------------------------------------------------------------------
// Fake IDBFactory — just enough of the async request/transaction protocol
// for this module's getAll -> put* -> delete* -> oncomplete sequence.
// ---------------------------------------------------------------------------

interface FakeRequest<T> {
  onsuccess: (() => void) | null
  onerror: (() => void) | null
  result: T | undefined
  error: Error | null
}

function makeRequest<T>(): FakeRequest<T> {
  return { onsuccess: null, onerror: null, result: undefined, error: null }
}

function resolveRequest<T>(req: FakeRequest<T>, result: T): void {
  req.result = result
  Promise.resolve().then(() => req.onsuccess?.())
}

class FakeObjectStore {
  constructor(private data: Map<string, ScenesThumbnailEntry>) {}
  getAll(): FakeRequest<ScenesThumbnailEntry[]> {
    const req = makeRequest<ScenesThumbnailEntry[]>()
    resolveRequest(req, Array.from(this.data.values()))
    return req
  }
  put(value: ScenesThumbnailEntry): FakeRequest<string> {
    const req = makeRequest<string>()
    this.data.set(value.key, value)
    resolveRequest(req, value.key)
    return req
  }
  delete(key: string): FakeRequest<undefined> {
    const req = makeRequest<undefined>()
    this.data.delete(key)
    resolveRequest(req, undefined)
    return req
  }
}

class FakeTransaction {
  oncomplete: (() => void) | null = null
  onerror: (() => void) | null = null
  constructor(private store: FakeObjectStore) {
    // See recents.test.ts's own FakeTransaction doc comment: a handful of
    // chained microtask hops gives the module's read -> compute -> write
    // sequence room to queue every request before this "commits".
    Promise.resolve().then(() => Promise.resolve()).then(() => Promise.resolve()).then(() => Promise.resolve()).then(() => {
      this.oncomplete?.()
    })
  }
  objectStore(_name: string): FakeObjectStore {
    return this.store
  }
}

class FakeDatabase {
  private data = new Map<string, ScenesThumbnailEntry>()
  private store = new FakeObjectStore(this.data)
  objectStoreNames = { contains: (name: string) => name === 'thumbnails' }
  createObjectStore(_name: string, _opts: unknown): FakeObjectStore {
    return this.store
  }
  transaction(_names: string | string[], _mode: string): FakeTransaction {
    return new FakeTransaction(this.store)
  }
  close(): void {}
}

function makeFakeIndexedDB(): IDBFactory {
  const databases = new Map<string, FakeDatabase>()
  return {
    open(name: string) {
      const req = makeRequest<FakeDatabase>() as unknown as IDBOpenDBRequest
      Promise.resolve().then(() => {
        let db = databases.get(name)
        const isNew = db === undefined
        if (db === undefined) {
          db = new FakeDatabase()
          databases.set(name, db)
        }
        ;(req as unknown as FakeRequest<FakeDatabase>).result = db
        if (isNew) req.onupgradeneeded?.(new Event('upgradeneeded') as unknown as IDBVersionChangeEvent)
        req.onsuccess?.(new Event('success'))
      })
      return req
    },
  } as unknown as IDBFactory
}

function makeFailingIndexedDB(): IDBFactory {
  return {
    open() {
      const req = makeRequest<never>() as unknown as IDBOpenDBRequest
      Promise.resolve().then(() => {
        ;(req as unknown as FakeRequest<never>).error = new Error('boom')
        req.onerror?.(new Event('error'))
      })
      return req
    },
  } as unknown as IDBFactory
}

// ---------------------------------------------------------------------------
// saveThumbnails / loadThumbnails — end-to-end against the fake
// ---------------------------------------------------------------------------

describe('saveThumbnails / loadThumbnails', () => {
  it('round-trips a thumbnail map for one document', async () => {
    const idb = makeFakeIndexedDB()
    await saveThumbnails('hash-a', new Map([[1, 'data:image/jpeg;base64,AAA'], [2, 'data:image/jpeg;base64,BBB']]), idb)
    const loaded = await loadThumbnails('hash-a', idb)
    expect(loaded.get(1)).toBe('data:image/jpeg;base64,AAA')
    expect(loaded.get(2)).toBe('data:image/jpeg;base64,BBB')
  })

  it('keys by contentHash + sid — different documents never collide, even with the same sid', async () => {
    const idb = makeFakeIndexedDB()
    await saveThumbnails('hash-a', new Map([[1, 'data:A']]), idb)
    await saveThumbnails('hash-b', new Map([[1, 'data:B']]), idb)
    expect((await loadThumbnails('hash-a', idb)).get(1)).toBe('data:A')
    expect((await loadThumbnails('hash-b', idb)).get(1)).toBe('data:B')
  })

  it('a later save under the same hash+sid overwrites the earlier one', async () => {
    const idb = makeFakeIndexedDB()
    await saveThumbnails('hash-a', new Map([[1, 'data:old']]), idb)
    await saveThumbnails('hash-a', new Map([[1, 'data:new']]), idb)
    const loaded = await loadThumbnails('hash-a', idb)
    expect(loaded.size).toBe(1)
    expect(loaded.get(1)).toBe('data:new')
  })

  it('loadThumbnails returns an empty map for an unknown hash', async () => {
    const idb = makeFakeIndexedDB()
    await saveThumbnails('hash-a', new Map([[1, 'data:A']]), idb)
    expect((await loadThumbnails('hash-unknown', idb)).size).toBe(0)
  })

  it('evicts the globally oldest entries once MAX_ENTRIES is exceeded', async () => {
    const idb = makeFakeIndexedDB()
    // A tight loop can land on the same Date.now() ms repeatedly — pin a
    // strictly increasing fake clock so "oldest" is unambiguous (mirrors
    // recents.test.ts's own "evicts the oldest entry" test, same reasoning).
    const now = vi.spyOn(Date, 'now')
    for (let i = 0; i < MAX_ENTRIES + 1; i++) {
      now.mockReturnValueOnce(1000 + i)
      await saveThumbnails(`hash-${i}`, new Map([[1, `data:${i}`]]), idb)
    }
    now.mockRestore()
    let total = 0
    for (let i = 0; i < MAX_ENTRIES + 1; i++) {
      total += (await loadThumbnails(`hash-${i}`, idb)).size
    }
    expect(total).toBe(MAX_ENTRIES)
    // The very first write is the oldest and falls off the cap.
    expect((await loadThumbnails('hash-0', idb)).size).toBe(0)
    expect((await loadThumbnails(`hash-${MAX_ENTRIES}`, idb)).size).toBe(1)
  })

  it('saveThumbnails is a no-op for an empty map (no IDB touched, no throw)', async () => {
    const idb = makeFakeIndexedDB()
    await expect(saveThumbnails('hash-a', new Map(), idb)).resolves.toBeUndefined()
    expect((await loadThumbnails('hash-a', idb)).size).toBe(0)
  })

  it('resolves to an empty map when indexedDB is unavailable', async () => {
    expect((await loadThumbnails('hash-a', undefined)).size).toBe(0)
  })

  it('saveThumbnails never throws when indexedDB is unavailable', async () => {
    await expect(saveThumbnails('hash-a', new Map([[1, 'data:A']]), undefined)).resolves.toBeUndefined()
  })

  it('swallows an indexedDB.open() failure on save without throwing', async () => {
    const idb = makeFailingIndexedDB()
    await expect(saveThumbnails('hash-a', new Map([[1, 'data:A']]), idb)).resolves.toBeUndefined()
  })

  it('swallows an indexedDB.open() failure on load, returning an empty map', async () => {
    const idb = makeFailingIndexedDB()
    expect((await loadThumbnails('hash-a', idb)).size).toBe(0)
  })
})
