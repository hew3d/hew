/**
 * recents.test — pure eviction-rule coverage plus an in-memory fake
 * `IDBFactory` exercising the real read/write/evict path end to end,
 * injected the same way `shellMode.test.ts` injects a fake `Storage`
 * (recents.ts's `idb` parameter) rather than touching the global
 * `indexedDB`.
 *
 * The fake is a deliberately minimal simulation of IndexedDB's async
 * request/transaction machinery — just enough of `open`/`getAll`/`put`/
 * `delete`/`oncomplete` to drive recents.ts's exact call sequence
 * correctly, not a spec-complete implementation.
 */
import { describe, it, expect, vi } from 'vitest'
import { recordRecent, listRecents, selectEvictions, MAX_ENTRIES, MAX_TOTAL_BYTES, type RecentEntry, type RecentMeta } from './recents'

// ---------------------------------------------------------------------------
// selectEvictions — pure logic
// ---------------------------------------------------------------------------

function meta(id: string, timestamp: number, byteLength = 10): RecentMeta {
  return { id, timestamp, byteLength }
}

describe('selectEvictions', () => {
  it('evicts nothing when under both caps', () => {
    const entries = [meta('a', 3), meta('b', 2), meta('c', 1)]
    expect(selectEvictions(entries, 20, 1000)).toEqual([])
  })

  it('evicts the oldest entries once the count cap is exceeded', () => {
    const entries = [meta('newest', 5), meta('mid', 4), meta('oldest', 3)]
    expect(selectEvictions(entries, 2, 1000)).toEqual(['oldest'])
  })

  it('evicts entries once the cumulative byte cap is exceeded, oldest first', () => {
    // newest=40, mid=40 (total 80, still <=100), oldest=40 (total 120 > 100) → oldest evicted.
    const entries = [meta('newest', 3, 40), meta('mid', 2, 40), meta('oldest', 1, 40)]
    expect(selectEvictions(entries, 20, 100)).toEqual(['oldest'])
  })

  it('evicts every entry older than the one that breached the byte cap', () => {
    const entries = [meta('a', 5, 60), meta('b', 4, 60), meta('c', 3, 60), meta('d', 2, 60)]
    // a=60 (ok), b=120 (>100, evict b and everything older: b,c,d).
    expect(selectEvictions(entries, 20, 100)).toEqual(['b', 'c', 'd'])
  })

  it('sorts by timestamp regardless of input order', () => {
    const entries = [meta('oldest', 1), meta('newest', 3), meta('mid', 2)]
    expect(selectEvictions(entries, 1, 1000)).toEqual(['mid', 'oldest'])
  })

  it('uses the exported MAX_ENTRIES/MAX_TOTAL_BYTES as defaults', () => {
    const entries = Array.from({ length: MAX_ENTRIES + 1 }, (_, i) => meta(`e${i}`, i, 1))
    const evicted = selectEvictions(entries)
    expect(evicted).toEqual(['e0']) // the single oldest, past the 20-entry cap
    expect(selectEvictions([meta('big', 1, MAX_TOTAL_BYTES + 1)])).toEqual(['big'])
  })
})

// ---------------------------------------------------------------------------
// Fake IDBFactory — just enough of the async request/transaction protocol
// for recents.ts's getAll → put → delete* → oncomplete sequence.
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
  constructor(private data: Map<string, RecentEntry>) {}
  getAll(): FakeRequest<RecentEntry[]> {
    const req = makeRequest<RecentEntry[]>()
    resolveRequest(req, Array.from(this.data.values()))
    return req
  }
  put(value: RecentEntry): FakeRequest<string> {
    const req = makeRequest<string>()
    this.data.set(value.id, value)
    resolveRequest(req, value.id)
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
    // Real IDB transactions auto-commit once the call stack that started
    // them returns to the event loop with no further requests queued.
    // recents.ts's whole read→compute→write sequence resolves across a
    // handful of microtask turns (each `await requestToPromise(...)`
    // ticks once); a few chained `Promise.resolve()` hops give it more
    // than enough room to finish queuing every request before this fires.
    Promise.resolve().then(() => Promise.resolve()).then(() => Promise.resolve()).then(() => Promise.resolve()).then(() => {
      this.oncomplete?.()
    })
  }
  objectStore(_name: string): FakeObjectStore {
    return this.store
  }
}

class FakeDatabase {
  private data = new Map<string, RecentEntry>()
  private store = new FakeObjectStore(this.data)
  objectStoreNames = { contains: (name: string) => name === 'recents' }
  createObjectStore(_name: string, _opts: unknown): FakeObjectStore {
    return this.store
  }
  transaction(_names: string | string[], _mode: string): FakeTransaction {
    return new FakeTransaction(this.store)
  }
  close(): void {}
}

/** A working fake `IDBFactory` — one persistent in-memory database, shared
 *  across every `open()` call the way a real browser's per-origin database
 *  is shared across connections. */
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

/** A fake `IDBFactory` whose `open()` always fails — for resilience tests:
 *  every recents.ts operation must degrade gracefully rather than throw. */
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
// recordRecent / listRecents — end-to-end against the fake
// ---------------------------------------------------------------------------

describe('recordRecent / listRecents', () => {
  it('records an open and lists it back', async () => {
    const idb = makeFakeIndexedDB()
    const bytes = new Uint8Array([1, 2, 3])
    await recordRecent(bytes, 'wall-clock.hew', 'open', idb)
    const all = await listRecents(idb)
    expect(all).toHaveLength(1)
    expect(all[0].name).toBe('wall-clock.hew')
    expect(all[0].source).toBe('open')
    expect(all[0].bytes).toEqual(bytes)
  })

  it('lists newest first', async () => {
    const idb = makeFakeIndexedDB()
    // Two calls in the same test tick can land on the same Date.now() ms —
    // pin an increasing fake clock so "newest" is unambiguous, matching how
    // two real opens are never truly simultaneous.
    const now = vi.spyOn(Date, 'now')
    now.mockReturnValueOnce(1000)
    await recordRecent(new Uint8Array([1]), 'first.hew', 'open', idb)
    now.mockReturnValueOnce(2000)
    await recordRecent(new Uint8Array([2]), 'second.hew', 'open', idb)
    now.mockRestore()
    const all = await listRecents(idb)
    expect(all.map((e) => e.name)).toEqual(['second.hew', 'first.hew'])
  })

  it('evicts the oldest entry once MAX_ENTRIES is exceeded', async () => {
    const idb = makeFakeIndexedDB()
    const now = vi.spyOn(Date, 'now')
    for (let i = 0; i < MAX_ENTRIES + 1; i++) {
      now.mockReturnValueOnce(1000 + i)
      await recordRecent(new Uint8Array([i]), `doc-${i}.hew`, 'open', idb)
    }
    now.mockRestore()
    const all = await listRecents(idb)
    expect(all).toHaveLength(MAX_ENTRIES)
    expect(all.map((e) => e.name)).not.toContain('doc-0.hew')
    expect(all.map((e) => e.name)).toContain(`doc-${MAX_ENTRIES}.hew`)
  })

  it('refuses to write a single entry larger than MAX_TOTAL_BYTES', async () => {
    const idb = makeFakeIndexedDB()
    const huge = new Uint8Array(MAX_TOTAL_BYTES + 1)
    await recordRecent(huge, 'too-big.hew', 'open', idb)
    expect(await listRecents(idb)).toEqual([])
  })

  it('resolves to an empty list when indexedDB is unavailable', async () => {
    expect(await listRecents(undefined)).toEqual([])
  })

  it('recordRecent is a no-op (never throws) when indexedDB is unavailable', async () => {
    await expect(recordRecent(new Uint8Array([1]), 'a.hew', 'open', undefined)).resolves.toBeUndefined()
  })

  it('swallows an indexedDB.open() failure without throwing', async () => {
    const idb = makeFailingIndexedDB()
    await expect(recordRecent(new Uint8Array([1]), 'a.hew', 'open', idb)).resolves.toBeUndefined()
    expect(await listRecents(idb)).toEqual([])
  })
})

// ---------------------------------------------------------------------------
// partCount — additive schema (design_handoff_shop_mode/README.md §8, empty
// state Recents rows: "2 hours ago · 9 parts")
// ---------------------------------------------------------------------------

describe('recordRecent / listRecents — partCount', () => {
  it('round-trips a recorded part count', async () => {
    const idb = makeFakeIndexedDB()
    await recordRecent(new Uint8Array([1]), 'table.hew', 'open', idb, 9)
    const all = await listRecents(idb)
    expect(all[0].partCount).toBe(9)
  })

  it('omits partCount entirely when not passed — not just stored as 0', async () => {
    const idb = makeFakeIndexedDB()
    await recordRecent(new Uint8Array([1]), 'table.hew', 'open', idb)
    const all = await listRecents(idb)
    expect(all[0].partCount).toBeUndefined()
    expect('partCount' in all[0]).toBe(false)
  })

  it('lists a mix of entries with and without partCount without dropping either', async () => {
    const idb = makeFakeIndexedDB()
    await recordRecent(new Uint8Array([1]), 'with-count.hew', 'open', idb, 4)
    await recordRecent(new Uint8Array([2]), 'legacy-no-count.hew', 'open', idb)
    const byName = new Map((await listRecents(idb)).map((e) => [e.name, e]))
    expect(byName.get('with-count.hew')?.partCount).toBe(4)
    expect(byName.get('legacy-no-count.hew')?.partCount).toBeUndefined()
  })
})

// ---------------------------------------------------------------------------
// contentHash dedupe (playtest fix 3) — re-recording the SAME bytes updates
// the existing row instead of stacking a duplicate.
// ---------------------------------------------------------------------------

describe('recordRecent — content-identity dedupe', () => {
  it('records a SHA-256 contentHash for a new entry', async () => {
    const idb = makeFakeIndexedDB()
    await recordRecent(new Uint8Array([1, 2, 3]), 'a.hew', 'open', idb)
    const all = await listRecents(idb)
    expect(all[0].contentHash).toMatch(/^[0-9a-f]{64}$/)
  })

  it('re-recording identical bytes updates the SAME row instead of inserting a second one', async () => {
    const idb = makeFakeIndexedDB()
    const bytes = new Uint8Array([1, 2, 3])
    const now = vi.spyOn(Date, 'now')
    now.mockReturnValueOnce(1000)
    await recordRecent(bytes, 'table.hew', 'open', idb)
    const firstId = (await listRecents(idb))[0].id

    now.mockReturnValueOnce(5000)
    await recordRecent(bytes, 'table.hew', 'open', idb)
    now.mockRestore()

    const all = await listRecents(idb)
    expect(all).toHaveLength(1)
    expect(all[0].id).toBe(firstId)
    expect(all[0].timestamp).toBe(5000)
  })

  it('re-opening the same content under a NEW name bumps it to the top and refreshes the name', async () => {
    const idb = makeFakeIndexedDB()
    const bytes = new Uint8Array([9, 9, 9])
    await recordRecent(bytes, 'old-name.hew', 'open', idb)
    await recordRecent(new Uint8Array([1]), 'other.hew', 'open', idb) // unrelated content
    await recordRecent(bytes, 'renamed.hew', 'open', idb) // same content, re-saved under a new name

    const all = await listRecents(idb)
    expect(all).toHaveLength(2) // still just 2 distinct documents, not 3 rows
    expect(all[0].name).toBe('renamed.hew') // newest first
    expect(all.map((e) => e.name)).not.toContain('old-name.hew')
  })

  it('different content never dedupes, even with the same name', async () => {
    const idb = makeFakeIndexedDB()
    await recordRecent(new Uint8Array([1]), 'model.hew', 'open', idb)
    await recordRecent(new Uint8Array([2]), 'model.hew', 'open', idb)
    expect(await listRecents(idb)).toHaveLength(2)
  })

  it('a partCount/thumbnail omitted on a dedupe-matching call preserves the PRIOR call\'s value', async () => {
    const idb = makeFakeIndexedDB()
    const bytes = new Uint8Array([4, 4, 4])
    await recordRecent(bytes, 'bench.hew', 'open', idb, 7, 'data:image/jpeg;base64,AAA')
    // A follow-up call for the SAME content that doesn't repeat partCount/
    // thumbnail (mirrors nothing in this app today, but proves the fallback
    // itself) must not blank out what the first call already recorded.
    await recordRecent(bytes, 'bench.hew', 'open', idb)
    const all = await listRecents(idb)
    expect(all).toHaveLength(1)
    expect(all[0].partCount).toBe(7)
    expect(all[0].thumbnail).toBe('data:image/jpeg;base64,AAA')
  })

  it('a partCount/thumbnail explicitly passed on a dedupe-matching call overwrites the prior value', async () => {
    const idb = makeFakeIndexedDB()
    const bytes = new Uint8Array([5, 5, 5])
    await recordRecent(bytes, 'bench.hew', 'open', idb, 3, 'data:image/jpeg;base64,OLD')
    await recordRecent(bytes, 'bench.hew', 'open', idb, 9, 'data:image/jpeg;base64,NEW')
    const all = await listRecents(idb)
    expect(all[0].partCount).toBe(9)
    expect(all[0].thumbnail).toBe('data:image/jpeg;base64,NEW')
  })

  it('an entry with no contentHash (pre-fix, or hashing unavailable) never dedupe-matches anything', async () => {
    const idb = makeFakeIndexedDB()
    const bytes = new Uint8Array([6, 6, 6])
    // Seed a "legacy" row directly, bypassing recordRecent's own hashing —
    // simulates a record written before this fix shipped.
    const db = await new Promise<IDBDatabase>((resolve) => {
      const req = idb.open('hew-recents', 2)
      req.onsuccess = () => resolve(req.result)
    })
    await new Promise<void>((resolve) => {
      const tx = db.transaction('recents', 'readwrite')
      tx.objectStore('recents').put({ id: 'legacy-1', bytes, name: 'legacy.hew', timestamp: 500, source: 'open' })
      tx.oncomplete = () => resolve()
    })
    db.close()

    await recordRecent(bytes, 'legacy.hew', 'open', idb)
    const all = await listRecents(idb)
    // A fresh row for the same bytes, NOT an update of the hash-less legacy
    // one — the legacy row survives untouched, matching "missing hash never
    // dedupes" (RecentEntry.contentHash's own doc comment).
    expect(all).toHaveLength(2)
    expect(all.some((e) => e.id === 'legacy-1')).toBe(true)
  })
})

// ---------------------------------------------------------------------------
// thumbnail — additive schema, same freshness contract as partCount
// (playtest fix 4).
// ---------------------------------------------------------------------------

describe('recordRecent / listRecents — thumbnail', () => {
  it('round-trips a recorded thumbnail data URL', async () => {
    const idb = makeFakeIndexedDB()
    await recordRecent(new Uint8Array([1]), 'chair.hew', 'open', idb, 3, 'data:image/jpeg;base64,ZZZ')
    const all = await listRecents(idb)
    expect(all[0].thumbnail).toBe('data:image/jpeg;base64,ZZZ')
  })

  it('omits thumbnail entirely when not passed', async () => {
    const idb = makeFakeIndexedDB()
    await recordRecent(new Uint8Array([1]), 'chair.hew', 'open', idb)
    const all = await listRecents(idb)
    expect(all[0].thumbnail).toBeUndefined()
    expect('thumbnail' in all[0]).toBe(false)
  })
})
