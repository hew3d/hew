/**
 * Package S10 oracle, part C: runs over a whole source and thumbnails
 * (docs/design/shared-library-sources.md 5.2 "Update all" and "Add, Insert,
 * Add all, Follow, Update" rows, 7.4 "Add all / Follow", "Follow", "Thumbnails",
 * decisions 21, 29, 30, 34): addAll, follow, newCounts, updateAll, thumbnail.
 *
 * Fixtures: sourcesTestKit.ts. Part A: sources.test.ts. Part B:
 * sources.items.test.ts.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { itemFileName } from './fileNaming'
import type { RunProgress, RunSummary, UpdateAllResult } from './sources'
import { MAX_THUMB, SRC, collectionOf, makeEnv, noteOf, parseFake, parts, sha256, uuid, type Env, type PublishItem, type Published } from './sourcesTestKit'
import type { LibraryRemoteNote } from './types'

let env: Env
beforeEach(() => {
  env = makeEnv()
})
afterEach(() => {
  env.assertClean()
})

/** The kit's index path and minted file name for "Part n". */
const pathOf = (n: number): string => `Components/part-${n}.hew`
const fileOf = (n: number): string => itemFileName(`Part ${n}`, uuid(n), 'component')
const fileCalls = (n: number): number => env.fetcher.calls.filter((c) => c.url.endsWith(`/${pathOf(n)}`)).length
const anyFileCalls = (): number => env.fetcher.calls.filter((c) => c.url.includes('/Components/')).length
const quiet = (): void => {
  env.fetcher.reset()
  env.store.resetLogs()
  env.events.length = 0
}

async function withSource(items: PublishItem[]): Promise<Published> {
  const pub = await env.addSource(SRC, { name: 'Shop Hardware', items })
  quiet()
  return pub
}

function setEntry(over: Record<string, unknown>): void {
  const entry = env.store.entryFor(SRC)
  if (!entry) throw new Error('no entry')
  env.store.sourcesFile = { hew: 1, sources: [{ ...entry, ...over }] }
}

function note(n: number, pub: Published, over: Partial<LibraryRemoteNote> = {}): LibraryRemoteNote {
  return { source: SRC, name: 'Shop Hardware', path: pathOf(n), sha256: pub.sha[uuid(n)], commit: 'c1', fetchedAt: '2026-01-01T00:00:00.000Z', edited: false, ...over }
}

const summaryOf = (r: unknown): RunSummary => {
  const x = r as { ok?: boolean; summary?: RunSummary }
  if (!x.ok || !x.summary) throw new Error(`no summary: ${JSON.stringify(r)}`)
  return x.summary
}

describe('addAll', () => {
  it('adds every item not in the Library, one at a time: one resolve, one index, then one request and one write per item', async () => {
    await withSource(parts(3))
    const progress: RunProgress[] = []
    const r = await env.sources.addAll(SRC, { onProgress: (p) => progress.push({ ...p }) })
    expect(summaryOf(r)).toMatchObject({ added: 3, failed: 0, retry: [], notTried: [], cancelled: false, stopped: null })
    expect(env.fetcher.calls.filter((c) => c.url.includes('/commit/'))).toHaveLength(1)
    expect(env.fetcher.calls.filter((c) => c.url.includes('/hew.json'))).toHaveLength(1)
    expect(anyFileCalls()).toBe(3)
    expect(env.fetcher.maxInFlight).toBe(1)
    expect(env.store.writes.sort()).toEqual([fileOf(1), fileOf(2), fileOf(3)])
    expect(env.events.filter((e) => e === 'write')).toHaveLength(3)
    expect(env.store.removes).toEqual([])
    for (const n of [1, 2, 3]) expect(noteOf(env.store.items.get(fileOf(n))!)).toMatchObject({ source: SRC, commit: 'c1', edited: false })
  })

  it('reports progress per item and ends at done == total', async () => {
    await withSource(parts(3))
    const progress: RunProgress[] = []
    await env.sources.addAll(SRC, { onProgress: (p) => progress.push({ ...p }) })
    expect(progress.length).toBeGreaterThanOrEqual(3)
    expect(progress.map((p) => p.done)).toEqual([...progress.map((p) => p.done)].sort((a, b) => a - b))
    expect(progress.at(-1)).toMatchObject({ done: 3, total: 3, added: 3, failed: 0 })
  })

  it('runs under the source lock', async () => {
    await withSource(parts(1))
    env.lockNames.length = 0
    await env.sources.addAll(SRC)
    expect(env.lockNames).toEqual([SRC])
  })

  it('a busy lock means busy and no request', async () => {
    await withSource(parts(1))
    env.busyLocks.add(SRC)
    expect(await env.sources.addAll(SRC)).toMatchObject({ ok: false, code: 'busy' })
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('leaves alone what is already in the Library (with or without a note) and what is on the skip list', async () => {
    const pub = await withSource(parts(5))
    env.store.seed(fileOf(1), { id: uuid(1), name: 'Part 1' })
    env.store.seed(fileOf(2), { id: uuid(2), name: 'Part 2', remote: note(2, pub, { source: 'github:other/src', name: 'Other' }) })
    setEntry({ skipped: [uuid(3)] })
    const r = await env.sources.addAll(SRC)
    expect(summaryOf(r)).toMatchObject({ added: 2, failed: 0 })
    expect(env.store.writes.sort()).toEqual([fileOf(4), fileOf(5)])
    expect([1, 2, 3].map(fileCalls)).toEqual([0, 0, 0])
  })

  it('skips profiles while the profile preview is off, and says how many', async () => {
    await withSource([{ id: uuid(1), name: 'Part 1' }, { id: uuid(2), name: 'Profile', category: 'profile' }])
    env.options.profiles = false
    const s = summaryOf(await env.sources.addAll(SRC))
    expect(s).toMatchObject({ added: 1, profilesSkipped: 1 })
    expect(env.store.writes).toEqual([fileOf(1)])
    expect(env.fetcher.calls.some((c) => c.url.includes('Profiles/'))).toBe(false)
  })

  it('takes profiles when the preview is on', async () => {
    await withSource([{ id: uuid(1), name: 'Part 1' }, { id: uuid(2), name: 'Profile', category: 'profile' }])
    const s = summaryOf(await env.sources.addAll(SRC))
    expect(s).toMatchObject({ added: 2, profilesSkipped: 0 })
  })

  it('Cancel keeps what has landed and starts nothing more', async () => {
    await withSource(parts(5))
    const ctl = new AbortController()
    const r = await env.sources.addAll(SRC, {
      signal: ctl.signal,
      onProgress: (p) => {
        if (p.done === 2) ctl.abort()
      },
    })
    const s = summaryOf(r)
    expect(s).toMatchObject({ added: 2, failed: 0, cancelled: true, stopped: null })
    expect(s.notTried).toEqual([uuid(3), uuid(4), uuid(5)])
    expect(env.store.writes.sort()).toEqual([fileOf(1), fileOf(2)])
    expect(anyFileCalls()).toBe(2)
    expect(env.store.removes).toEqual([])
  })

  it('a failed item is recorded and the run goes on', async () => {
    await withSource(parts(3))
    env.server.fail((u) => u.endsWith(`/${pathOf(2)}`), 'http_status', { status: 404 })
    const s = summaryOf(await env.sources.addAll(SRC))
    expect(s).toMatchObject({ added: 2, failed: 1, retry: [uuid(2)], stopped: null, cancelled: false })
    expect(s.failures).toEqual([{ id: uuid(2), code: 'http_status' }])
    expect(env.store.writes.sort()).toEqual([fileOf(1), fileOf(3)])
    expect(env.store.items.has(fileOf(2))).toBe(false)
  })

  it('an item that fails the hash check twice is a failed row, not the end of the run', async () => {
    await withSource(parts(3))
    env.server.put(env.server.urlFor(SRC, pathOf(2), 'c1'), new TextEncoder().encode('{"lib":{"id":"x"},"body":"wrong"}'))
    const s = summaryOf(await env.sources.addAll(SRC))
    expect(s).toMatchObject({ added: 2, failed: 1 })
    expect(s.failures).toEqual([{ id: uuid(2), code: 'hash_mismatch' }])
  })

  it('three network failures in a row stop the run; the rest is not tried', async () => {
    await withSource(parts(5))
    for (const n of [1, 2, 3]) env.server.fail((u) => u.endsWith(`/${pathOf(n)}`), 'network')
    const s = summaryOf(await env.sources.addAll(SRC))
    expect(s).toMatchObject({ added: 0, failed: 3, stopped: 'network', cancelled: false })
    expect(s.retry).toEqual([uuid(1), uuid(2), uuid(3)])
    expect(s.notTried).toEqual([uuid(4), uuid(5)])
    expect([4, 5].map(fileCalls)).toEqual([0, 0])
  })

  it('network failures that are not in a row do not stop it', async () => {
    await withSource(parts(5))
    for (const n of [1, 2, 4, 5]) env.server.fail((u) => u.endsWith(`/${pathOf(n)}`), 'network')
    const s = summaryOf(await env.sources.addAll(SRC))
    expect(s).toMatchObject({ added: 1, failed: 4, stopped: null })
    expect(s.notTried).toEqual([])
  })

  it('one rate_limited stops the run at once; that item is recorded as failed', async () => {
    await withSource(parts(4))
    env.server.fail((u) => u.endsWith(`/${pathOf(2)}`), 'rate_limited')
    const s = summaryOf(await env.sources.addAll(SRC))
    expect(s).toMatchObject({ added: 1, failed: 1, stopped: 'rate_limited' })
    expect(s.failures).toEqual([{ id: uuid(2), code: 'rate_limited' }])
    expect(s.notTried).toEqual([uuid(3), uuid(4)])
    expect([3, 4].map(fileCalls)).toEqual([0, 0])
  })

  it('Retry: running again with the retry ids adds only those', async () => {
    await withSource(parts(4))
    env.server.fail((u) => u.endsWith(`/${pathOf(2)}`) || u.endsWith(`/${pathOf(4)}`), 'http_status', { status: 500 })
    const first = summaryOf(await env.sources.addAll(SRC))
    expect(first.retry).toEqual([uuid(2), uuid(4)])
    env.server.clearRules()
    quiet()
    const second = summaryOf(await env.sources.addAll(SRC, { ids: first.retry }))
    expect(second).toMatchObject({ added: 2, failed: 0 })
    expect(env.store.writes.sort()).toEqual([fileOf(2), fileOf(4)])
    expect([1, 3].map(fileCalls)).toEqual([0, 0])
  })

  it('web: refuses when the browser has less free space than the items need, with both sizes, before any item request', async () => {
    const pub = await withSource(parts(3))
    const total = pub.index.items.reduce((n, i) => n + i.bytes, 0)
    env.options.storage = { usage: 1000, quota: 1000 + total - 1 }
    const r = await env.sources.addAll(SRC)
    expect(r).toMatchObject({ ok: false, code: 'not_enough_space', needed: total, free: total - 1 })
    expect(anyFileCalls()).toBe(0)
    expect(env.store.writes).toEqual([])
  })

  it('web: an exact fit is allowed, and so is an estimate the browser cannot give', async () => {
    const pub = await withSource(parts(2))
    const total = pub.index.items.reduce((n, i) => n + i.bytes, 0)
    env.options.storage = { usage: 0, quota: total }
    expect(summaryOf(await env.sources.addAll(SRC))).toMatchObject({ added: 2 })
    env.store.items.clear()
    env.options.storage = null
    expect(summaryOf(await env.sources.addAll(SRC))).toMatchObject({ added: 2 })
  })

  it('a removed source is unavailable: source_removed, no item request, no write', async () => {
    await withSource(parts(2))
    env.server.publishCatalog({ removed: [{ source: SRC, severity: 'removed', reason: 'No.' }] })
    await env.sources.catalog()
    quiet()
    expect(await env.sources.addAll(SRC)).toMatchObject({ ok: false, code: 'source_removed', reason: 'No.' })
    expect(anyFileCalls()).toBe(0)
    expect(env.store.writes).toEqual([])
  })

  it('an unreachable source fails the run with the fetch code, adds nothing, and keeps the cached listing', async () => {
    await withSource(parts(2))
    env.server.fail((u) => u.includes('/hew.json'), 'network')
    expect(await env.sources.addAll(SRC)).toMatchObject({ ok: false, code: 'network' })
    expect(env.store.writes).toEqual([])
    expect(await env.sources.itemStates(SRC)).toMatchObject({ ok: true })
  })

  it('a source not in the list is unknown_source', async () => {
    expect(await env.sources.addAll(SRC)).toMatchObject({ ok: false, code: 'unknown_source' })
  })

  it('is Off: off:true, no request', async () => {
    await withSource(parts(1))
    env.fetcher.on = false
    expect(await env.sources.addAll(SRC)).toEqual({ off: true })
    expect(env.fetcher.calls).toHaveLength(0)
  })
})

describe('follow', () => {
  it('sets followed through writeSources, then adds everything', async () => {
    await withSource(parts(2))
    const s = summaryOf(await env.sources.follow(SRC))
    expect(s).toMatchObject({ added: 2, failed: 0 })
    expect(env.store.entryFor(SRC)?.followed).toBe(true)
    const followedAt = env.store.log.indexOf(`writeSources ${SRC}`)
    const firstWrite = env.store.log.findIndex((l) => l.startsWith('write Components/'))
    expect(followedAt).toBeGreaterThan(-1)
    expect(followedAt).toBeLessThan(firstWrite)
  })

  it('runs under the source lock; a busy lock means busy and no item request', async () => {
    await withSource(parts(1))
    env.busyLocks.add(SRC)
    expect(await env.sources.follow(SRC)).toMatchObject({ ok: false, code: 'busy' })
    expect(env.lockNames).toContain(SRC)
    expect(anyFileCalls()).toBe(0)
  })

  it('when the storage pre-check refuses, followed is NOT set', async () => {
    await withSource(parts(2))
    env.options.storage = { usage: 0, quota: 1 }
    expect(await env.sources.follow(SRC)).toMatchObject({ ok: false, code: 'not_enough_space' })
    expect(env.store.entryFor(SRC)?.followed).toBe(false)
  })

  it('when the lock is busy, followed is NOT set', async () => {
    await withSource(parts(1))
    env.busyLocks.add(SRC)
    expect(await env.sources.follow(SRC)).toMatchObject({ ok: false, code: 'busy' })
    expect(env.store.entryFor(SRC)?.followed).toBe(false)
  })

  it('a removed source cannot be followed, and is not marked followed', async () => {
    await withSource(parts(1))
    env.server.publishCatalog({ removed: [{ source: SRC, severity: 'removed', reason: 'No.' }] })
    await env.sources.catalog()
    expect(await env.sources.follow(SRC)).toMatchObject({ ok: false, code: 'source_removed' })
    expect(env.store.entryFor(SRC)?.followed).toBe(false)
  })

  it('Cancel keeps what landed and the source stays followed', async () => {
    await withSource(parts(4))
    const ctl = new AbortController()
    const s = summaryOf(
      await env.sources.follow(SRC, {
        signal: ctl.signal,
        onProgress: (p) => {
          if (p.done === 1) ctl.abort()
        },
      }),
    )
    expect(s).toMatchObject({ added: 1, cancelled: true })
    expect(env.store.writes).toEqual([fileOf(1)])
    expect(env.store.entryFor(SRC)?.followed).toBe(true)
  })

  it('follows with profiles skipped while the preview is off', async () => {
    await withSource([{ id: uuid(1), name: 'Part 1' }, { id: uuid(2), name: 'Profile', category: 'profile' }])
    env.options.profiles = false
    expect(summaryOf(await env.sources.follow(SRC))).toMatchObject({ added: 1, profilesSkipped: 1 })
  })

  it('after a later check, items not in the Library and not skipped count as new; the check result carries the counts', async () => {
    await withSource(parts(2))
    await env.sources.follow(SRC)
    env.server.publish(SRC, { name: 'Shop Hardware', commit: 'c2', items: parts(5) })
    const r = await env.sources.check(SRC)
    expect(r).toMatchObject({ ok: true, counts: { new: 3, changed: 0 } })
    expect(await env.sources.newCounts(SRC)).toEqual({ new: 3, changed: 0 })
  })

  it('a source that is not followed has no new count', async () => {
    await withSource(parts(2))
    env.server.publish(SRC, { name: 'Shop Hardware', commit: 'c2', items: parts(5) })
    await env.sources.check(SRC)
    expect((await env.sources.newCounts(SRC)).new).toBe(0)
  })

  it('is Off: off:true and the source is not marked followed', async () => {
    await withSource(parts(1))
    env.fetcher.on = false
    expect(await env.sources.follow(SRC)).toEqual({ off: true })
    expect(env.store.entryFor(SRC)?.followed).toBe(false)
  })
})

describe('newCounts', () => {
  it('new = in the index, not in the Library at all (any note, or none), not skipped; profiles excluded while the preview is off', async () => {
    const pub = await withSource([...parts(5), { id: uuid(6), name: 'Profile', category: 'profile' }])
    setEntry({ followed: true, skipped: [uuid(5)] })
    env.store.seed(fileOf(1), { id: uuid(1), name: 'Part 1' })
    env.store.seed(fileOf(2), { id: uuid(2), name: 'Part 2', remote: note(2, pub, { source: 'github:other/src', name: 'Other' }) })
    // Part 3, Part 4: new. Part 5: skipped. Profile: new while the preview is on.
    expect(await env.sources.newCounts(SRC)).toEqual({ new: 3, changed: 0 })
    env.options.profiles = false
    expect(await env.sources.newCounts(SRC)).toEqual({ new: 2, changed: 0 })
  })

  it('changed = R-newer rows that are not Kept; in-step and gone rows are not counted', async () => {
    const pub = await withSource(parts(4))
    env.store.seed(fileOf(1), { id: uuid(1), name: 'Part 1', remote: note(1, pub, { sha256: 'a'.repeat(64) }) }) // newer
    env.store.seed(fileOf(2), { id: uuid(2), name: 'Part 2', remote: note(2, pub, { sha256: 'b'.repeat(64) }) }) // newer, will be kept
    env.store.seed(fileOf(3), { id: uuid(3), name: 'Part 3', remote: note(3, pub) }) // in step
    env.store.seed(fileOf(9), { id: uuid(9), name: 'Part 9', remote: note(9, pub, { sha256: 'c'.repeat(64) }) }) // gone
    expect(await env.sources.newCounts(SRC)).toEqual({ new: 0, changed: 2 })
    await env.sources.keepMine(SRC, uuid(2))
    expect(await env.sources.newCounts(SRC)).toEqual({ new: 0, changed: 1 })
  })

  it('an unknown source counts nothing', async () => {
    expect(await env.sources.newCounts('github:nobody/nothing')).toEqual({ new: 0, changed: 0 })
  })
})

describe('updateAll', () => {
  /** Six items linked at version 1, version 2 published and checked. Local state:
   * 1 plain newer; 2 newer and edited; 3 newer and kept; 4 newer, its file fails
   * to fetch; 5 newer, open for editing; 6 plain newer. */
  async function six(): Promise<{ pub1: Published; pub2: Published }> {
    const pub1 = await env.addSource(SRC, { name: 'Shop Hardware', items: parts(6) })
    for (let n = 1; n <= 6; n++) {
      env.store.seed(fileOf(n), { id: uuid(n), name: `Part ${n}`, collection: 'Mine', remote: note(n, pub1, { edited: n === 2 }) })
    }
    const pub2 = env.server.publish(SRC, { name: 'Shop Hardware', commit: 'c2', items: parts(6, { body: 'v2' }) })
    await env.sources.check(SRC)
    await env.sources.keepMine(SRC, uuid(3))
    env.server.fail((u) => u.endsWith(`/c2/${pathOf(4)}`), 'http_status', { status: 404 })
    env.store.editing = [fileOf(5)]
    quiet()
    return { pub1, pub2 }
  }

  const rowsOf = (r: UpdateAllResult) => {
    if (!('ok' in r) || !r.ok) throw new Error(`updateAll failed: ${JSON.stringify(r)}`)
    return r
  }

  it('updates the R-newer rows that are not edited and not Kept; each row reports its own result; one failure does not stop the rest', async () => {
    const { pub2 } = await six()
    const r = rowsOf(await env.sources.updateAll(SRC))
    expect(r.rows.map((x) => x.id)).toEqual([uuid(1), uuid(4), uuid(5), uuid(6)])
    expect(r.rows[0].result).toMatchObject({ ok: true, file: fileOf(1) })
    expect(r.rows[1].result).toMatchObject({ ok: false, code: 'http_status', status: 404 })
    expect(r.rows[2].result).toMatchObject({ ok: false, code: 'open_for_editing' })
    expect(r.rows[3].result).toMatchObject({ ok: true, file: fileOf(6) })
    expect(env.store.writes.sort()).toEqual([fileOf(1), fileOf(6)])
    expect(noteOf(env.store.items.get(fileOf(1))!)).toMatchObject({ sha256: pub2.sha[uuid(1)], commit: 'c2' })
    expect(collectionOf(env.store.items.get(fileOf(1))!)).toBe('Mine')
  })

  it('edited rows are left for Review and never replaced in bulk; kept rows are not touched', async () => {
    await six()
    const before2 = env.store.items.get(fileOf(2))
    const before3 = env.store.items.get(fileOf(3))
    const r = rowsOf(await env.sources.updateAll(SRC))
    expect(r.needsReview).toEqual([uuid(2)])
    expect(r.rows.map((x) => x.id)).not.toContain(uuid(2))
    expect(r.rows.map((x) => x.id)).not.toContain(uuid(3))
    expect(env.store.items.get(fileOf(2))).toBe(before2)
    expect(env.store.items.get(fileOf(3))).toBe(before3)
    expect(parseFake(env.store.items.get(fileOf(6))!).body).toBe('v2')
  })

  it('does one fresh check first, reading the index once, then one request per updated file', async () => {
    await six()
    await env.sources.updateAll(SRC)
    expect(env.fetcher.calls.filter((c) => c.url.includes('/hew.json'))).toHaveLength(1)
    expect(env.fetcher.calls.filter((c) => c.url.includes('/commit/'))).toHaveLength(1)
    expect(env.fetcher.maxInFlight).toBe(1)
  })

  it('runs under the source lock; a busy lock means busy and no request', async () => {
    await six()
    env.busyLocks.add(SRC)
    expect(await env.sources.updateAll(SRC)).toMatchObject({ ok: false, code: 'busy' })
    expect(env.fetcher.calls).toHaveLength(0)
    expect(env.lockNames.at(-1)).toBe(SRC)
  })

  it('a removed source is unavailable: source_removed', async () => {
    await six()
    env.server.publishCatalog({ removed: [{ source: SRC, severity: 'removed', reason: 'No.' }] })
    await env.sources.catalog()
    quiet()
    expect(await env.sources.updateAll(SRC)).toMatchObject({ ok: false, code: 'source_removed', reason: 'No.' })
    expect(env.store.writes).toEqual([])
  })

  it('an unreachable source fails with the fetch code and changes nothing', async () => {
    await six()
    env.server.fail((u) => u.includes('/hew.json'), 'network')
    expect(await env.sources.updateAll(SRC)).toMatchObject({ ok: false, code: 'network' })
    expect(env.store.writes).toEqual([])
  })

  it('with nothing to update it succeeds with no rows', async () => {
    const pub = await withSource(parts(1))
    env.store.seed(fileOf(1), { id: uuid(1), name: 'Part 1', remote: note(1, pub) })
    expect(rowsOf(await env.sources.updateAll(SRC))).toMatchObject({ rows: [], needsReview: [] })
    expect(env.store.writes).toEqual([])
  })

  it('is Off: off:true, no request', async () => {
    await six()
    env.fetcher.on = false
    expect(await env.sources.updateAll(SRC)).toEqual({ off: true })
    expect(env.fetcher.calls).toHaveLength(0)
  })
})

describe('thumbnails', () => {
  const PNG = new Uint8Array([137, 80, 78, 71, 1, 2, 3])

  it('an entry with no thumbnail yields null bytes (the placeholder) and no request', async () => {
    await withSource(parts(1))
    expect(await env.sources.thumbnail(SRC, uuid(1))).toEqual({ ok: true, bytes: null })
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('fetches the bytes through the fetcher with the 1 MiB cap at the checked commit, caches them in .sources/, and returns bytes (not a URL)', async () => {
    const pub = await withSource([{ id: uuid(1), name: 'Part 1', thumbnail: PNG }])
    const r = await env.sources.thumbnail(SRC, uuid(1))
    expect(r).toMatchObject({ ok: true })
    const bytes = (r as { bytes: Uint8Array }).bytes
    expect(bytes).toBeInstanceOf(Uint8Array)
    expect([...bytes]).toEqual([...PNG])
    expect(env.fetcher.calls).toEqual([{ url: env.server.urlFor(SRC, `thumbnails/${pub.sha[uuid(1)]}.png`, 'c1'), maxBytes: MAX_THUMB }])
    expect(env.store.cacheWrites).toHaveLength(1)
    expect(env.store.writes).toEqual([])
  })

  it('the second ask is served from the cache: no request, no second cache write', async () => {
    await withSource([{ id: uuid(1), name: 'Part 1', thumbnail: PNG }])
    await env.sources.thumbnail(SRC, uuid(1))
    env.fetcher.reset()
    const again = await env.sources.thumbnail(SRC, uuid(1))
    expect([...(again as { bytes: Uint8Array }).bytes]).toEqual([...PNG])
    expect(env.fetcher.calls).toHaveLength(0)
    expect(env.store.cacheWrites).toHaveLength(1)
  })

  it('the cache survives a restart over the same Library', async () => {
    await withSource([{ id: uuid(1), name: 'Part 1', thumbnail: PNG }])
    await env.sources.thumbnail(SRC, uuid(1))
    const shared = { store: env.store, server: env.server, clock: env.clock }
    env.assertClean()
    env = makeEnv({ shared })
    const r = await env.sources.thumbnail(SRC, uuid(1))
    expect([...(r as { bytes: Uint8Array }).bytes]).toEqual([...PNG])
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('a failed fetch is returned as its code and caches nothing', async () => {
    await withSource([{ id: uuid(1), name: 'Part 1', thumbnail: PNG }])
    env.server.fail((u) => u.endsWith('.png'), 'network')
    expect(await env.sources.thumbnail(SRC, uuid(1))).toMatchObject({ ok: false, code: 'network' })
    expect(env.store.cacheWrites).toHaveLength(0)
  })

  it('a thumbnail over 1 MiB is too_large and is not cached', async () => {
    await withSource([{ id: uuid(1), name: 'Part 1', thumbnail: new Uint8Array(MAX_THUMB + 1) }])
    expect(await env.sources.thumbnail(SRC, uuid(1))).toMatchObject({ ok: false, code: 'too_large' })
    expect(env.store.cacheWrites).toHaveLength(0)
  })

  it('an id not in the index is item_not_found; a source not in the list is unknown_source', async () => {
    await withSource(parts(1))
    expect(await env.sources.thumbnail(SRC, uuid(9))).toMatchObject({ ok: false, code: 'item_not_found' })
    expect(await env.sources.thumbnail('github:nobody/nothing', uuid(1))).toMatchObject({ ok: false, code: 'unknown_source' })
  })

  it('is Off when it would need the network', async () => {
    await withSource([{ id: uuid(1), name: 'Part 1', thumbnail: PNG }])
    env.fetcher.on = false
    expect(await env.sources.thumbnail(SRC, uuid(1))).toEqual({ off: true })
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('Off also covers a cached thumbnail: off:true with no request', async () => {
    await withSource([{ id: uuid(1), name: 'Part 1', thumbnail: PNG }])
    await env.sources.thumbnail(SRC, uuid(1))
    env.fetcher.on = false
    env.fetcher.reset()
    expect(await env.sources.thumbnail(SRC, uuid(1))).toEqual({ off: true })
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('the thumbnail bytes are what the publisher served (sha-addressed path)', async () => {
    const pub = await withSource([{ id: uuid(1), name: 'Part 1', thumbnail: PNG }])
    expect(pub.index.items[0].thumbnail).toBe(`thumbnails/${pub.sha[uuid(1)]}.png`)
    expect(sha256(PNG)).toHaveLength(64)
  })
})
