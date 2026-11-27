/* oxlint-disable lingui/no-unlocalized-strings -- test fixtures */
/**
 * Package S10, review round 2: reproducing tests for the findings fixed after
 * the follow-up adversarial review (F1, F2, F3, F6, F8, F9, F10, F11). Each
 * block names its finding. Fixtures: sourcesTestKit.ts.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { sourceErrorMessage } from './sourceMessages'
import type { CatalogSourceEntry } from './sources'
import { SRC, SRC2, fakeParseSourceRef, fakeSourceUrls, makeEnv, parts, sha256, uuid, type Env, type PublishItem } from './sourcesTestKit'

let env: Env
beforeEach(() => {
  env = makeEnv()
})
afterEach(() => {
  env.assertClean()
})

const enc = new TextEncoder()
const PNG = new Uint8Array([137, 80, 78, 71, 1, 2, 3])
const PNG2 = new Uint8Array([137, 80, 78, 71, 9, 9, 9])

const keyOf = (prefix: string, text: string): string => `${prefix}-${sha256(enc.encode(text)).slice(0, 24)}`
const idxKey = (source: string): string => keyOf('idx', source)
const snapKey = (source: string): string => keyOf('snap', source)
const cacheKeys = (): string[] => [...env.store.cache.keys()].sort()
const thumbKeys = (): string[] => cacheKeys().filter((k) => k.startsWith('thumb-'))
const withThumb = (n: number, png = PNG): PublishItem => ({ id: uuid(n), name: `Part ${n}`, thumbnail: png })

/** Lets every pending microtask (every fake store and fetcher step) run. */
const settle = (): Promise<void> => new Promise((r) => setTimeout(r, 0))

const quiet = (): void => {
  env.fetcher.reset()
  env.store.resetLogs()
  env.events.length = 0
}

const followed = (on: boolean): void => {
  const e = env.store.sourcesFile?.sources.find((s) => s.source === SRC)
  if (e === undefined) throw new Error('no entry')
  e.followed = on
}

/** Runs `hook` inside the first request whose URL contains `part`, before it answers. */
function duringFetch(part: string, hook: () => Promise<unknown>): void {
  const real = env.fetcher.fetchBytes.bind(env.fetcher)
  let fired = false
  env.fetcher.fetchBytes = async (url: string, maxBytes: number): Promise<Uint8Array> => {
    if (!fired && url.includes(part)) {
      fired = true
      await hook()
    }
    return real(url, maxBytes)
  }
}

// --------------------------------------------------------------------- F2

describe('F2: a resolve answer with no usable commit is an error, not an unpinned read', () => {
  const resolveUrl = (): string => {
    const refs = fakeParseSourceRef(SRC)
    if ('error' in refs) throw new Error('bad ref')
    return fakeSourceUrls(refs[0], []).resolve!
  }

  it('a check whose resolve answers 200 with no commit fails resolve_invalid and reads nothing at the bare ref', async () => {
    await env.addSource(SRC, { name: 'Hinges', commit: 'c1', items: parts(1) })
    env.server.put(resolveUrl(), new Uint8Array(0))
    quiet()
    const r = await env.sources.check(SRC)
    expect(r).toMatchObject({ ok: false, code: 'resolve_invalid' })
    expect(env.fetcher.matching('/hew.json')).toHaveLength(0)
    expect(env.store.entryFor(SRC)?.lastCheck).toMatchObject({ ok: false, code: 'resolve_invalid' })
    // The pinned commit of the last good read is kept.
    expect(env.store.entryFor(SRC)?.commit).toBe('c1')
  })

  it('adding a source whose resolve answers 200 with no commit fails and stores nothing', async () => {
    env.server.publish(SRC, { name: 'Hinges', commit: 'c1', items: parts(1) })
    env.server.put(resolveUrl(), new Uint8Array(0))
    expect(await env.sources.addSource(SRC)).toMatchObject({ ok: false, code: 'resolve_invalid' })
    expect(env.fetcher.matching('/hew.json')).toHaveLength(0)
    expect(env.store.entryFor(SRC)).toBeUndefined()
  })

  it('reads as an address with no Hew source', () => {
    expect(sourceErrorMessage('resolve_invalid', { host: 'codeberg.org' })).toBe('This address does not have a Hew source.')
  })
})

// --------------------------------------------------------------------- F9

describe('F9: a source removed during a check’s final write stays removed', () => {
  it('a removal that lands between the check’s last entry read and its write is not undone', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    env.server.publish(SRC, { name: 'Hinges', commit: 'c2', items: parts(2) })
    const realWrite = env.store.writeSourceCache.bind(env.store)
    const realRead = env.store.readSources.bind(env.store)
    let armed = false
    let fired = false
    let removal: Promise<unknown> | null = null
    env.store.writeSourceCache = async (key, bytes) => {
      await realWrite(key, bytes)
      if (key === idxKey(SRC)) armed = true
    }
    env.store.readSources = async () => {
      const snap = await realRead()
      if (armed && !fired) {
        fired = true
        removal = env.sources.removeSource(SRC)
        await settle()
      }
      return snap
    }
    await env.sources.check(SRC)
    expect(fired).toBe(true)
    expect(await removal).toEqual({ ok: true })
    expect(await env.sources.listSources()).toEqual([])
    expect(env.store.entryFor(SRC)).toBeUndefined()
  })
})

// --------------------------------------------------------------------- F8

describe('F8: a source removed during a check leaves no index cache behind', () => {
  it('a removal between the check’s entry re-read and its index write leaves no idx entry', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    env.server.publish(SRC, { name: 'Hinges', commit: 'c2', items: parts(2) })
    const realRead = env.store.readSourceCache.bind(env.store)
    let armed = false
    let fired = false
    let removal: Promise<unknown> | null = null
    env.store.readSourceCache = async (key) => {
      const got = await realRead(key)
      if (armed && !fired && key === idxKey(SRC)) {
        fired = true
        removal = env.sources.removeSource(SRC)
        await settle()
      }
      return got
    }
    armed = true
    await env.sources.check(SRC)
    expect(fired).toBe(true)
    expect(await removal).toEqual({ ok: true })
    expect(env.store.entryFor(SRC)).toBeUndefined()
    expect(cacheKeys()).not.toContain(idxKey(SRC))
  })
})

// -------------------------------------------------------------------- F10

describe('F10: removing a source deletes its index and snapshot cache entries, keeping the catalog', () => {
  it('idx-* and snap-* of the removed source are gone; the catalog and another source’s idx stay', async () => {
    const entry: CatalogSourceEntry = { source: SRC, kind: 'github', name: 'Hinges', snapshot: 'snapshots/hinges.json' } as CatalogSourceEntry
    env.server.publishCatalog({ sources: [entry] })
    env.server.put(new URL('snapshots/hinges.json', new URL('.', env.server.catalogUrl)).href, enc.encode(JSON.stringify({ hew: 1, name: 'Hinges', items: [] })))
    expect(await env.sources.catalog()).toMatchObject({ ok: true })
    expect(await env.sources.catalogPreview(entry)).toMatchObject({ ok: true })
    await env.addSource(SRC, { name: 'Hinges', items: parts(1) })
    await env.addSource(SRC2, { name: 'Slides', items: parts(1) })
    expect(cacheKeys()).toEqual(expect.arrayContaining([idxKey(SRC), snapKey(SRC), idxKey(SRC2), 'catalog']))
    expect(await env.sources.removeSource(SRC)).toEqual({ ok: true })
    expect(cacheKeys()).not.toContain(idxKey(SRC))
    expect(cacheKeys()).not.toContain(snapKey(SRC))
    expect(cacheKeys()).toEqual(expect.arrayContaining([idxKey(SRC2), 'catalog']))
  })
})

// --------------------------------------------------------------------- F1

describe('F1: a thumbnail fetched across a removal or a dropping check leaves no cache entry', () => {
  it('a source removed while its thumbnail is in flight gets no thumb entry', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: [withThumb(1)] })
    duringFetch('thumbnails/', () => env.sources.removeSource(SRC))
    expect(await env.sources.thumbnail(SRC, uuid(1))).toMatchObject({ ok: true })
    expect(env.store.entryFor(SRC)).toBeUndefined()
    expect(thumbKeys()).toEqual([])
  })

  it('an item a check drops while its thumbnail is in flight gets no thumb entry', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: [withThumb(1), withThumb(2, PNG2)] })
    duringFetch('thumbnails/', async () => {
      env.server.publish(SRC, { name: 'Hinges', commit: 'c2', items: [withThumb(1)] })
      expect(await env.sources.check(SRC)).toMatchObject({ ok: true })
    })
    expect(await env.sources.thumbnail(SRC, uuid(2))).toMatchObject({ ok: true })
    expect(thumbKeys()).toEqual([])
  })

  it('a thumbnail still listed is cached as before', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: [withThumb(1)] })
    expect(await env.sources.thumbnail(SRC, uuid(1))).toMatchObject({ ok: true })
    expect(thumbKeys()).toHaveLength(1)
  })
})

// --------------------------------------------------------------------- F3

describe('F3: a thumbnail whose delete failed is reclaimed by a later sweep', () => {
  it('a failed delete on a check is retried by the next check, though the index has moved on', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: [withThumb(1), withThumb(2, PNG2)] })
    for (const n of [1, 2]) expect(await env.sources.thumbnail(SRC, uuid(n))).toMatchObject({ ok: true })
    const both = thumbKeys()
    expect(both).toHaveLength(2)
    env.store.failCacheRemove = true
    env.server.publish(SRC, { name: 'Hinges', commit: 'c2', items: [withThumb(1)] })
    expect(await env.sources.check(SRC)).toMatchObject({ ok: true })
    expect(thumbKeys()).toHaveLength(2)
    env.store.failCacheRemove = false
    env.server.publish(SRC, { name: 'Hinges', commit: 'c3', items: [withThumb(1)] })
    expect(await env.sources.check(SRC)).toMatchObject({ ok: true })
    const left = thumbKeys()
    expect(left).toHaveLength(1)
    expect(both).toContain(left[0])
    // The kept one is item 1's, still served from the cache.
    env.fetcher.reset()
    expect(await env.sources.thumbnail(SRC, uuid(1))).toMatchObject({ ok: true })
    expect(env.fetcher.matching('thumbnails/')).toHaveLength(0)
  })

  it('a failed delete on a removal is retried by a later check of another source', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: [withThumb(1)] })
    await env.addSource(SRC2, { name: 'Slides', items: [withThumb(1, PNG2)] })
    expect(await env.sources.thumbnail(SRC, uuid(1))).toMatchObject({ ok: true })
    expect(await env.sources.thumbnail(SRC2, uuid(1))).toMatchObject({ ok: true })
    const mine = thumbKeys()
    env.store.failCacheRemove = true
    expect(await env.sources.removeSource(SRC)).toEqual({ ok: true })
    env.store.failCacheRemove = false
    expect(await env.sources.check(SRC2)).toMatchObject({ ok: true })
    const left = thumbKeys()
    expect(left).toHaveLength(1)
    expect(mine).toContain(left[0])
    expect(cacheKeys()).not.toContain(idxKey(SRC))
    expect(cacheKeys()).toContain(idxKey(SRC2))
    // Nothing is pending any more: only the live source's entries remain.
    expect(cacheKeys().filter((k) => k !== idxKey(SRC2) && !k.startsWith('thumb-'))).toEqual([])
  })
})

// --------------------------------------------------------------------- F6

describe('F6: a stored http_status failure with no status reads without a code', () => {
  it('names the host and no number', () => {
    expect(sourceErrorMessage('http_status', { host: 'codeberg.org', web: true })).toBe('codeberg.org answered with an error. Try again later.')
  })

  it('with no host, still no number', () => {
    expect(sourceErrorMessage('http_status', {})).toBe('The site answered with an error. Try again later.')
  })

  it('a status still reads with its code', () => {
    expect(sourceErrorMessage('http_status', { host: 'codeberg.org', status: 503 })).toBe('codeberg.org answered with an error (503). Try again later.')
  })
})

// -------------------------------------------------------------------- F11

describe('F11: a failed check’s cached rows reflect the entry as it is after the fetch', () => {
  it('a delete during a failing check shows as skipped in the cached rows', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: parts(2) })
    followed(true)
    env.server.fail((url) => url.endsWith('/hew.json'), 'network')
    duringFetch('/hew.json', () => env.sources.noteDeleted(uuid(1), { source: SRC }))
    const r = await env.sources.check(SRC)
    expect(r).toMatchObject({ ok: false, code: 'network' })
    if (!('cached' in r) || r.cached === undefined) throw new Error('no cached rows')
    expect(r.cached.rows.find((row) => row.id === uuid(1))?.skipped).toBe(true)
    expect(r.cached.rows.find((row) => row.id === uuid(2))?.skipped).toBe(false)
  })
})
