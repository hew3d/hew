/**
 * Package S10 oracle, part A: adding, removing, listing and checking a source,
 * the 5.1 state rows, the catalog and its removed list, the check scheduler
 * and the Off state (docs/design/shared-library-sources.md 2 decisions 19-21,
 * 29, 34; 5.1; 6.2; 6.3; 7.4).
 *
 * Fixtures: sourcesTestKit.ts (fake fetcher over a fake index server keyed by
 * URL, fake store, fake wasm layer, fake item bytes). Parts B and C:
 * sources.items.test.ts and sources.runs.test.ts. Failure reasons today: the
 * stub's createSources methods throw `unimplemented`.
 */
import { afterEach, beforeEach, describe, expect, expectTypeOf, it, vi } from 'vitest'
import type { CheckFailed, CheckOk, ItemRow, ItemState } from './sources'
import type { LibraryRemoteNote } from './types'
import { MAX_INDEX, MAX_RESOLVE, SRC, SRC2, bump, makeEnv, parts, uuid, type Env } from './sourcesTestKit'

let env: Env
beforeEach(() => {
  env = makeEnv()
})
afterEach(() => {
  env.assertClean()
  vi.useRealTimers()
})

const rowOf = (rows: ItemRow[], id: string): ItemRow => {
  const r = rows.find((x) => x.id === id)
  if (!r) throw new Error(`no row ${id}`)
  return r
}

async function rowsOf(source = SRC): Promise<ItemRow[]> {
  const r = await env.sources.itemStates(source)
  if (!('ok' in r) || !r.ok) throw new Error('itemStates failed')
  return r.rows
}

describe('addSource', () => {
  it('resolves the commit, reads the index at it, stores one entry and caches the index', async () => {
    env.server.publish(SRC, { name: 'Shop Hardware', license: 'CC0-1.0', items: parts(2) })
    const r = await env.sources.addSource(SRC)
    expect('ok' in r && r.ok).toBe(true)
    if (!('ok' in r) || !r.ok) return
    expect(r.existing).toBe(false)
    // The add also reads the catalog for the listed/unlisted flag: leave that request out.
    const own = env.fetcher.calls.filter((c) => !c.url.includes('catalog.'))
    expect(own.map((c) => c.maxBytes)).toEqual([MAX_RESOLVE, MAX_INDEX])
    expect(own[0].url).toContain('/commit/')
    expect(own[1].url).toContain('/c1/hew.json')
    expect(env.store.sourceWrites).toHaveLength(1)
    const w = env.store.sourceWrites[0]
    expect(w.source).toBe(SRC)
    expect(w.entry).toMatchObject({ source: SRC, kind: 'github', name: 'Shop Hardware', followed: false, addedAt: env.clock.iso(), skipped: [], kept: {} })
    expect(env.store.cacheWrites).toHaveLength(1)
    expect(env.store.items.size).toBe(0)
  })

  it('records a successful lastCheck, so the new source is not immediately due', async () => {
    env.server.publish(SRC, { name: 'Shop Hardware', items: parts(1) })
    await env.sources.addSource(SRC)
    expect(env.store.entryFor(SRC)?.lastCheck).toEqual({ at: env.clock.iso(), ok: true, code: null })
    expect(await env.sources.shouldAutoCheck(SRC)).toBe(false)
  })

  it('returns the index summary the Add dialog shows', async () => {
    const pub = env.server.publish(SRC, {
      name: 'Shop Hardware',
      license: 'CC-BY-4.0',
      header: { author: 'Example Maker', description: 'Hinges.' },
      items: [...parts(2), { id: uuid(9), name: 'Birch', category: 'material' }],
    })
    const r = await env.sources.addSource(SRC)
    if (!('ok' in r) || !r.ok) throw new Error('add failed')
    expect(r.summary).toMatchObject({ name: 'Shop Hardware', author: 'Example Maker', license: 'CC-BY-4.0', itemCount: 3, categories: { Components: 2, Materials: 1 } })
    expect(r.summary.totalBytes).toBe(pub.index.items.reduce((n, i) => n + i.bytes, 0))
    expect(r.host).toBe('github.com')
  })

  it('adding a reference already in the list changes nothing: no request, no write', async () => {
    env.server.publish(SRC, { name: 'Shop Hardware', items: parts(1) })
    await env.sources.addSource(SRC)
    env.fetcher.reset()
    env.store.resetLogs()
    const again = await env.sources.addSource(SRC)
    expect('ok' in again && again.ok && again.existing).toBe(true)
    expect(env.fetcher.calls).toHaveLength(0)
    expect(env.store.sourceWrites).toHaveLength(0)
    expect(env.store.cacheWrites).toHaveLength(0)
  })

  it('tries the candidates of an unknown host in order web, forgejo, gitlab and keeps the first valid index', async () => {
    const forgejo = 'forgejo:unknown.example.test/parts'
    env.server.publish(forgejo, { name: 'Forgejo parts', items: parts(1) })
    // The web candidate has nothing at its address (404): it counts as failed.
    const r = await env.sources.addSource('https://unknown.example.test/parts')
    expect('ok' in r && r.ok).toBe(true)
    if (!('ok' in r) || !r.ok) return
    expect(r.entry.source).toBe(forgejo)
    expect(r.entry.kind).toBe('forgejo')
    const urls = env.fetcher.urls()
    expect(urls[0]).toBe('https://unknown.example.test/parts/hew.json')
    expect(urls.some((u) => u.includes('gitlab:'))).toBe(false)
    expect(urls.some((u) => u.includes(forgejo))).toBe(true)
    expect(env.store.sourceWrites.map((w) => w.source)).toEqual([forgejo])
  })

  it('keeps the FIRST candidate whose response parses, never reading later ones', async () => {
    env.server.put('https://unknown.example.test/parts/hew.json', new TextEncoder().encode(JSON.stringify({ hew: 1, name: 'Web parts', items: [] })))
    env.server.publish('forgejo:unknown.example.test/parts', { name: 'Forgejo parts', items: [] })
    const r = await env.sources.addSource('https://unknown.example.test/parts')
    if (!('ok' in r) || !r.ok) throw new Error('add failed')
    expect(r.entry.kind).toBe('web')
    expect(env.fetcher.urls().filter((u) => u.includes('forgejo:'))).toHaveLength(0)
  })

  it('a candidate whose bytes are not a valid index counts as failed and the next is tried', async () => {
    env.server.put('https://unknown.example.test/parts/hew.json', new TextEncoder().encode('<html>nope</html>'))
    env.server.publish('forgejo:unknown.example.test/parts', { name: 'Forgejo parts', items: [] })
    const r = await env.sources.addSource('https://unknown.example.test/parts')
    if (!('ok' in r) || !r.ok) throw new Error('add failed')
    expect(r.entry.kind).toBe('forgejo')
  })

  it('gives each candidate 10 seconds: a hanging one is abandoned and the next is tried', async () => {
    vi.useFakeTimers()
    env.server.hang((u) => u === 'https://unknown.example.test/parts/hew.json')
    env.server.publish('forgejo:unknown.example.test/parts', { name: 'Forgejo parts', items: [] })
    const pending = env.sources.addSource('https://unknown.example.test/parts')
    await vi.advanceTimersByTimeAsync(9_999)
    expect(env.fetcher.urls().some((u) => u.includes('forgejo:'))).toBe(false)
    await vi.advanceTimersByTimeAsync(2)
    const r = await pending
    if (!('ok' in r) || !r.ok) throw new Error('add failed')
    expect(r.entry.kind).toBe('forgejo')
  })

  it('every candidate failing is no_source with each candidate code, and nothing is stored', async () => {
    const r = await env.sources.addSource('https://unknown.example.test/nothing-here')
    expect(r).toMatchObject({ ok: false, code: 'no_source' })
    expect((r as { causes?: string[] }).causes).toHaveLength(3)
    expect(env.store.sourceWrites).toHaveLength(0)
    expect(env.store.cacheWrites).toHaveLength(0)
  })

  it.each([
    ['network', 'network'],
    ['rate_limited', 'rate_limited'],
    ['timeout', 'timeout'],
  ] as const)('a single-candidate host failing with %s reports that code and stores nothing', async (fail, code) => {
    env.server.publish(SRC, { name: 'X', items: [] })
    env.server.fail((u) => u.includes('/hew.json'), fail)
    const r = await env.sources.addSource(SRC)
    expect(r).toMatchObject({ ok: false, code })
    expect(env.store.sourceWrites).toHaveLength(0)
    expect(env.store.cacheWrites).toHaveLength(0)
  })

  it('an http_status failure carries the status', async () => {
    env.server.publish(SRC, { name: 'X', items: [] })
    env.server.fail((u) => u.includes('/hew.json'), 'http_status', { status: 404 })
    expect(await env.sources.addSource(SRC)).toMatchObject({ ok: false, code: 'http_status', status: 404 })
  })

  it.each([
    ['index_too_new', JSON.stringify({ hew: 2, name: 'X', items: [] })],
    ['index_invalid', JSON.stringify({ hew: 1, items: [] })],
  ])('a single-candidate index that is %s is reported by that code', async (code, body) => {
    env.server.publish(SRC, { name: 'X', items: [] })
    env.server.put(env.server.urlFor(SRC, 'hew.json'), new TextEncoder().encode(body))
    expect(await env.sources.addSource(SRC)).toMatchObject({ ok: false, code })
    expect(env.store.sourceWrites).toHaveLength(0)
  })

  it('text that is not a source reference fails ref_invalid without a request', async () => {
    expect(await env.sources.addSource('not a source')).toMatchObject({ ok: false, code: 'ref_invalid' })
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('says listed when the catalog lists the source, unlisted when it does not', async () => {
    env.server.publishCatalog({ sources: [{ source: SRC, kind: 'github', name: 'Shop', items: 1, bytes: 10, categories: {}, stale: false, snapshot: 'sources/a.json' }] })
    env.server.publish(SRC, { name: 'Shop', items: [] })
    env.server.publish(SRC2, { name: 'Slides', items: [] })
    const a = await env.sources.addSource(SRC)
    const b = await env.sources.addSource(SRC2)
    expect('ok' in a && a.ok && a.listing).toBe('listed')
    expect('ok' in b && b.ok && b.listing).toBe('unlisted')
  })

  it('listing sets the @ref aside: a listed source added with a ref is listed', async () => {
    env.server.publishCatalog({ sources: [{ source: SRC, kind: 'github', name: 'Shop', items: 1, bytes: 10, categories: {}, stale: false, snapshot: 'sources/a.json' }] })
    env.server.publish(`${SRC}@v2`, { name: 'Shop', items: [] })
    const r = await env.sources.addSource(`${SRC}@v2`)
    expect('ok' in r && r.ok && r.listing).toBe('listed')
  })

  it('a multi-candidate probe that meets index_too_new from any candidate reports index_too_new, not no_source', async () => {
    const forgejo = 'forgejo:unknown.example.test/parts'
    env.server.publish(forgejo, { name: 'X', items: [] })
    env.server.put(env.server.urlFor(forgejo, 'hew.json'), new TextEncoder().encode(JSON.stringify({ hew: 2, name: 'X', items: [] })))
    const r = await env.sources.addSource('https://unknown.example.test/parts')
    expect(r).toMatchObject({ ok: false, code: 'index_too_new' })
    expect(env.store.sourceWrites).toHaveLength(0)
  })

  it('says unknown (not unlisted) when no catalog can be read', async () => {
    env.server.publish(SRC, { name: 'Shop', items: [] })
    env.server.fail((u) => u.includes('catalog.'), 'network')
    const r = await env.sources.addSource(SRC)
    expect('ok' in r && r.ok && r.listing).toBe('unknown')
  })

  it('with the catalog turned off it says unknown and makes no catalog request', async () => {
    env.assertClean()
    env = makeEnv({ catalogUrl: '' })
    env.server.publish(SRC, { name: 'Shop', items: [] })
    const r = await env.sources.addSource(SRC)
    expect('ok' in r && r.ok && r.listing).toBe('unknown')
    expect(env.fetcher.urls().some((u) => u.includes('catalog.'))).toBe(false)
  })

  it('reads the catalog once and reuses the cached copy for the next add', async () => {
    env.server.publishCatalog({ sources: [] })
    env.server.publish(SRC, { name: 'Shop', items: [] })
    env.server.publish(SRC2, { name: 'Slides', items: [] })
    await env.sources.addSource(SRC)
    await env.sources.addSource(SRC2)
    expect(env.fetcher.urls().filter((u) => u.includes('catalog.'))).toHaveLength(1)
  })

  it('is Off: resolves off:true and makes no request', async () => {
    env.fetcher.on = false
    expect(await env.sources.addSource(SRC)).toEqual({ off: true })
    expect(env.fetcher.calls).toHaveLength(0)
    expect(env.store.sourceWrites).toHaveLength(0)
  })
})

describe('removeSource and listSources', () => {
  it('removing writes null for that source only and leaves every item file alone', async () => {
    const pub = await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    await env.addSource(SRC2, { name: 'Slides', items: [] })
    const id = Object.keys(pub.sha)[0]
    env.store.seed('Components/part-1-000001.hew', { id, name: 'Part 1', remote: { source: SRC, name: 'Shop', path: 'p', sha256: pub.sha[id], fetchedAt: 't', edited: false } })
    env.store.resetLogs()
    const before = env.store.snapshot()
    expect(await env.sources.removeSource(SRC)).toEqual({ ok: true })
    expect(env.store.sourceWrites).toEqual([{ source: SRC, entry: null }])
    expect(env.store.writes).toHaveLength(0)
    expect(env.store.removes).toHaveLength(0)
    expect(env.store.items).toEqual(before)
    expect((await env.sources.listSources()).map((s) => s.entry.source)).toEqual([SRC2])
  })

  it('an item of a removed source becomes R-orphan; adding the same source again links it back', async () => {
    const pub = await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    const id = Object.keys(pub.sha)[0]
    env.store.seed('Components/part-1-000001.hew', { id, name: 'Part 1', remote: { source: SRC, name: 'Shop', path: 'Components/part-1.hew', sha256: pub.sha[id], fetchedAt: 't', edited: false } })
    await env.sources.removeSource(SRC)
    expect((await env.sources.localStates())[0]).toMatchObject({ id, state: 'orphan' })
    await env.sources.addSource(SRC)
    expect((await env.sources.localStates())[0]).toMatchObject({ id, state: 'in_step' })
  })

  it('listSources reads the file only: no request, entries with their flags', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.fetcher.reset()
    const list = await env.sources.listSources()
    expect(env.fetcher.calls).toHaveLength(0)
    expect(list).toHaveLength(1)
    expect(list[0]).toMatchObject({ followed: false, unreachable: false, removed: null })
    expect(list[0].entry).toMatchObject({ source: SRC, name: 'Shop' })
  })

  it('unfollow clears followed through writeSources', async () => {
    await env.addSource(SRC, { name: 'Shop', items: [] })
    await env.sources.follow(SRC)
    expect(env.store.entryFor(SRC)?.followed).toBe(true)
    expect(await env.sources.unfollow(SRC)).toEqual({ ok: true })
    expect(env.store.entryFor(SRC)?.followed).toBe(false)
  })

  it('removing or unfollowing a source not in the list fails unknown_source', async () => {
    expect(await env.sources.removeSource(SRC)).toMatchObject({ ok: false, code: 'unknown_source' })
    expect(await env.sources.unfollow(SRC)).toMatchObject({ ok: false, code: 'unknown_source' })
  })
})

describe('check', () => {
  it('resolves the commit first, then reads the index at that commit, caches it and records lastCheck', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(2) })
    env.server.publish(SRC, { name: 'Shop', commit: 'c2', items: parts(3) })
    env.fetcher.reset()
    env.store.resetLogs()
    env.clock.advance(7 * 3600_000)
    const r = await env.sources.check(SRC)
    const ok = r as CheckOk
    expect(ok.ok).toBe(true)
    expect(ok.commit).toBe('c2')
    expect(env.fetcher.calls.map((c) => c.maxBytes)).toEqual([MAX_RESOLVE, MAX_INDEX])
    expect(env.fetcher.urls()[0]).toContain('/commit/')
    expect(env.fetcher.urls()[1]).toContain('/c2/hew.json')
    expect(env.store.cacheWrites).toHaveLength(1)
    expect(env.store.entryFor(SRC)?.lastCheck).toEqual({ at: env.clock.iso(), ok: true, code: null })
    expect(ok.rows).toHaveLength(3)
  })

  it('computes every item\'s state in the same call (rows come back with the result)', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(2) })
    const r = (await env.sources.check(SRC)) as CheckOk
    expect(r.rows.map((x) => x.state)).toEqual(['not_in_library', 'not_in_library'])
    expect(r.counts).toEqual({ new: 0, changed: 0 })
  })

  it('a rate-limited resolve falls back to reading at the ref (no commit)', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.server.fail((u) => u.includes('/commit/'), 'rate_limited')
    env.fetcher.reset()
    const r = (await env.sources.check(SRC)) as CheckOk
    expect(r.ok).toBe(true)
    expect(r.commit).toBeUndefined()
    expect(env.fetcher.urls()[1]).toContain('/HEAD/hew.json')
  })

  it('a web source has no resolve request and no commit', async () => {
    const web = 'web:https://pages.example.test/hinges/hew.json'
    env.server.publish(web, { name: 'Pages', items: parts(1) })
    await env.sources.addSource(web)
    env.fetcher.reset()
    const r = (await env.sources.check(web)) as CheckOk
    expect(r.ok).toBe(true)
    expect(env.fetcher.urls()).toEqual(['https://pages.example.test/hinges/hew.json'])
  })

  it('an unreachable source: code returned, lastCheck records the failure, the last good listing stays with its time', async () => {
    const good = await env.addSource(SRC, { name: 'Shop', items: parts(2) })
    const goodAt = env.clock.iso()
    env.clock.advance(8 * 3600_000)
    env.server.fail((u) => u.includes('/hew.json'), 'network')
    const r = (await env.sources.check(SRC)) as CheckFailed
    expect(r).toMatchObject({ ok: false, code: 'network' })
    expect(r.cached?.rows).toHaveLength(2)
    expect(r.cached?.lastGoodAt).toBe(goodAt)
    expect(env.store.entryFor(SRC)?.lastCheck).toEqual({ at: env.clock.iso(), ok: false, code: 'network' })
    const [status] = await env.sources.listSources()
    expect(status.unreachable).toBe(true)
    const states = await env.sources.itemStates(SRC)
    expect(states).toMatchObject({ ok: true, lastGoodAt: goodAt })
    expect(Object.keys(good.sha)).toHaveLength(2)
  })

  it('S-unreachable is cleared by the next check that succeeds', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.server.fail((u) => u.includes('/hew.json'), 'network', { times: 1 })
    await env.sources.check(SRC)
    expect((await env.sources.listSources())[0].unreachable).toBe(true)
    await env.sources.check(SRC)
    expect((await env.sources.listSources())[0].unreachable).toBe(false)
  })

  it.each([
    ['index_too_new', JSON.stringify({ hew: 2, name: 'X', items: [] })],
    ['index_invalid', '{"oops"'],
  ])('an index that is %s fails the check with that code, S-unreachable, cached listing kept', async (code, body) => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.server.put(env.server.urlFor(SRC, 'hew.json'), new TextEncoder().encode(body))
    const r = (await env.sources.check(SRC)) as CheckFailed
    expect(r).toMatchObject({ ok: false, code })
    expect(r.cached?.rows).toHaveLength(1)
    expect(env.store.entryFor(SRC)?.lastCheck).toMatchObject({ ok: false, code })
  })

  it('an index larger than its cap (too_large from the fetcher) fails the check', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.server.fail((u) => u.includes('/hew.json'), 'too_large')
    expect(await env.sources.check(SRC)).toMatchObject({ ok: false, code: 'too_large' })
  })

  it('a source that was never added is unknown_source with no request', async () => {
    expect(await env.sources.check(SRC)).toMatchObject({ ok: false, code: 'unknown_source' })
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('runs under the source lock named by its canonical reference; a busy lock means no request', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.lockNames.length = 0
    env.fetcher.reset()
    await env.sources.check(SRC)
    expect(env.lockNames).toEqual([SRC])
    env.busyLocks.add(SRC)
    env.fetcher.reset()
    expect(await env.sources.check(SRC)).toMatchObject({ ok: false, code: 'busy' })
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('a fetcher that rejects with an ordinary Error (carrying a URL) is reported as network, with no URL', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.server.throwPlain((u) => u.includes('/hew.json'), new Error(`boom ${env.server.urlFor(SRC, 'hew.json')}`))
    expect(await env.sources.check(SRC)).toMatchObject({ ok: false, code: 'network' })
  })

  it('is Off: off:true, no request', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.fetcher.on = false
    env.fetcher.reset()
    expect(await env.sources.check(SRC)).toEqual({ off: true })
    expect(env.fetcher.calls).toHaveLength(0)
  })
})

describe('5.1 item states against one source', () => {
  const note = (over: Partial<LibraryRemoteNote> = {}): LibraryRemoteNote => ({ source: SRC, name: 'Shop', path: 'Components/part.hew', sha256: 'x'.repeat(64), fetchedAt: 't', edited: false, ...over })

  it('the state names are a closed string-literal union', () => {
    expectTypeOf<ItemState>().toEqualTypeOf<'not_in_library' | 'in_library' | 'in_library_from_other' | 'in_step' | 'newer' | 'gone' | 'orphan'>()
  })

  it('none / present in the index: Not in your Library', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    expect(rowOf(await rowsOf(), uuid(1))).toMatchObject({ state: 'not_in_library', file: null, edited: false, kept: false })
  })

  it('present, no note, in the index: In your Library', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.store.seed('Components/mine-000001.hew', { id: uuid(1), name: 'Mine' })
    expect(rowOf(await rowsOf(), uuid(1))).toMatchObject({ state: 'in_library', file: 'Components/mine-000001.hew' })
  })

  it('present, note names another source: In your Library from <that source>', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    await env.addSource(SRC2, { name: 'Slides', items: [] })
    env.store.seed('Components/part-1-000001.hew', { id: uuid(1), name: 'Part 1', remote: note({ source: SRC2, name: 'Slides' }) })
    expect(rowOf(await rowsOf(), uuid(1))).toMatchObject({ state: 'in_library_from_other', from: 'Slides' })
  })

  it('a note naming a source that is not in the list still reads from-other in this source\'s view', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.store.seed('Components/part-1-000001.hew', { id: uuid(1), name: 'Part 1', remote: note({ source: 'github:gone/away', name: 'Away' }) })
    expect(rowOf(await rowsOf(), uuid(1))).toMatchObject({ state: 'in_library_from_other', from: 'Away' })
  })

  it('note names S and the sha256 equals the index entry\'s: R-in-step', async () => {
    const pub = await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.store.seed('Components/part-1-000001.hew', { id: uuid(1), name: 'Part 1', remote: note({ sha256: pub.sha[uuid(1)] }) })
    expect(rowOf(await rowsOf(), uuid(1))).toMatchObject({ state: 'in_step', edited: false })
  })

  it('the note\'s hash differs from the entry\'s: R-newer, whether the note or the file on disk changed (only the note\'s hash counts)', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.store.seed('Components/part-1-000001.hew', { id: uuid(1), name: 'Part 1', remote: note({ sha256: 'a'.repeat(64) }) })
    expect(rowOf(await rowsOf(), uuid(1))).toMatchObject({ state: 'newer' })
  })

  it('the bytes on disk are never compared with the note: a different body with the matching note hash is still in step', async () => {
    const pub = await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.store.seed('Components/part-1-000001.hew', { id: uuid(1), name: 'Part 1', body: 'edited by hand', remote: note({ sha256: pub.sha[uuid(1)] }) })
    expect(rowOf(await rowsOf(), uuid(1))).toMatchObject({ state: 'in_step' })
  })

  it('note names S and the index has no entry: R-gone (the row stays, with no entry)', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.store.seed('Components/other-0000ff.hew', { id: uuid(255), name: 'Other', remote: note() })
    const rows = await rowsOf()
    expect(rowOf(rows, uuid(255))).toMatchObject({ state: 'gone', entry: null, file: 'Components/other-0000ff.hew' })
  })

  it('note names a source not in .sources.json: R-orphan (through localStates)', async () => {
    env.store.seed('Components/part-1-000001.hew', { id: uuid(1), name: 'Part 1', remote: note({ source: 'github:gone/away', name: 'Away' }) })
    const l = await env.sources.localStates()
    expect(l).toHaveLength(1)
    expect(l[0]).toMatchObject({ id: uuid(1), state: 'orphan', file: 'Components/part-1-000001.hew' })
    expect(l[0].note.name).toBe('Away')
  })

  it('localStates lists every local item with a note against its source\'s cached index, and skips items without one', async () => {
    const pub = await env.addSource(SRC, { name: 'Shop', items: parts(2) })
    env.store.seed('a.hew', { id: uuid(1), name: 'Part 1', remote: note({ sha256: pub.sha[uuid(1)] }) })
    env.store.seed('b.hew', { id: uuid(2), name: 'Part 2', remote: note({ sha256: 'f'.repeat(64) }) })
    env.store.seed('c.hew', { id: uuid(3), name: 'No note' })
    const l = await env.sources.localStates()
    expect(l.map((x) => [x.id, x.state]).sort()).toEqual([
      [uuid(1), 'in_step'],
      [uuid(2), 'newer'],
    ])
  })

  it('R-edited adds the flag to in_step, newer and gone rows', async () => {
    const pub = await env.addSource(SRC, { name: 'Shop', items: parts(2) })
    env.store.seed('a.hew', { id: uuid(1), name: 'Part 1', remote: note({ sha256: pub.sha[uuid(1)], edited: true }) })
    env.store.seed('b.hew', { id: uuid(2), name: 'Part 2', remote: note({ sha256: 'f'.repeat(64), edited: true }) })
    env.store.seed('c.hew', { id: uuid(9), name: 'Gone', remote: note({ edited: true }) })
    const rows = await rowsOf()
    expect(rowOf(rows, uuid(1))).toMatchObject({ state: 'in_step', edited: true })
    expect(rowOf(rows, uuid(2))).toMatchObject({ state: 'newer', edited: true })
    expect(rowOf(rows, uuid(9))).toMatchObject({ state: 'gone', edited: true })
  })

  it('Kept: the declined sha256 still reads newer but is flagged kept; a new sha256 offers it again', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.store.seed('a.hew', { id: uuid(1), name: 'Part 1', remote: note({ sha256: 'f'.repeat(64) }) })
    expect(await env.sources.keepMine(SRC, uuid(1))).toEqual({ ok: true })
    expect(rowOf(await rowsOf(), uuid(1))).toMatchObject({ state: 'newer', kept: true })
    // The publisher pushes another version: the declined hash no longer equals the entry's.
    env.server.publish(SRC, { name: 'Shop', commit: 'c2', items: [bump(parts(1)[0], 'v3')] })
    await env.sources.check(SRC)
    expect(rowOf(await rowsOf(), uuid(1))).toMatchObject({ state: 'newer', kept: false })
  })

  it('rows carry the id, name and category of the entry and the skip flag', async () => {
    await env.addSource(SRC, { name: 'Shop', items: [{ id: uuid(1), name: 'Birch', category: 'material' }] })
    await env.sources.noteDeleted(uuid(1), { source: SRC }) // not followed: no skip yet
    expect(rowOf(await rowsOf(), uuid(1))).toMatchObject({ name: 'Birch', category: 'Materials', skipped: false })
  })

  it('profile rows are left out while the profile preview is off', async () => {
    env.options.profiles = false
    await env.addSource(SRC, { name: 'Shop', items: [{ id: uuid(1), name: 'Profile', category: 'profile' }, { id: uuid(2), name: 'Part' }] })
    expect((await rowsOf()).map((r) => r.id)).toEqual([uuid(2)])
    env.options.profiles = true
    expect((await rowsOf()).map((r) => r.id).sort()).toEqual([uuid(1), uuid(2)])
  })

  it('itemStates for a source with no cached index fails not_checked', async () => {
    env.store.sourcesFile = {
      hew: 1,
      sources: [{ source: SRC, kind: 'github', name: 'Shop', followed: false, addedAt: 't', lastCheck: null, skipped: [], kept: {} }],
    }
    expect(await env.sources.itemStates(SRC)).toMatchObject({ ok: false, code: 'not_checked' })
  })
})

describe('catalog', () => {
  const entry = (source: string) => ({ source, kind: 'github' as const, name: source, items: 1, bytes: 10, categories: {}, stale: false, snapshot: 'sources/a.json' })

  it('fetches catalog.json with the 4 MiB cap, caches it and returns the sources', async () => {
    env.server.publishCatalog({ sources: [entry(SRC), entry(SRC2)] })
    const r = await env.sources.catalog()
    expect('ok' in r && r.ok).toBe(true)
    if (!('ok' in r) || !r.ok) return
    expect(r.catalog.sources.map((s) => s.source)).toEqual([SRC, SRC2])
    expect(env.fetcher.calls).toEqual([{ url: env.server.catalogUrl, maxBytes: MAX_INDEX }])
    expect(env.store.cacheWrites).toHaveLength(1)
  })

  it('an empty catalog is a success with no sources', async () => {
    env.server.publishCatalog({ sources: [] })
    const r = await env.sources.catalog()
    expect('ok' in r && r.ok && r.catalog.sources).toEqual([])
  })

  it('a configured-empty catalog URL is catalog_disabled with no request; catalogEnabled says so', async () => {
    env.assertClean()
    env = makeEnv({ catalogUrl: '' })
    expect(env.sources.catalogEnabled()).toBe(false)
    expect(await env.sources.catalog()).toMatchObject({ ok: false, code: 'catalog_disabled' })
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('catalogEnabled is true when a URL is configured', () => {
    expect(env.sources.catalogEnabled()).toBe(true)
  })

  it('a failed fetch keeps and returns the last good catalog', async () => {
    env.server.publishCatalog({ sources: [entry(SRC)] })
    await env.sources.catalog()
    env.server.fail((u) => u.includes('catalog.'), 'network')
    const r = await env.sources.catalog()
    expect(r).toMatchObject({ ok: false, code: 'network' })
    expect((r as { cached: { sources: unknown[] } }).cached.sources).toHaveLength(1)
  })

  it('a failed fetch with nothing cached has cached: null', async () => {
    env.server.fail((u) => u.includes('catalog.'), 'network')
    expect(await env.sources.catalog()).toMatchObject({ ok: false, code: 'network', cached: null })
  })

  it('a catalog that is not valid JSON is catalog_invalid', async () => {
    env.server.put(env.server.catalogUrl, new TextEncoder().encode('<html>'))
    expect(await env.sources.catalog()).toMatchObject({ ok: false, code: 'catalog_invalid' })
  })

  describe('catalogPreview', () => {
    const SNAP = 'sources/zzz-not-a-slug.json'
    const snapUrl = () => new URL(SNAP, env.server.catalogUrl).href
    const catEntry = () => ({ ...entry(SRC), snapshot: SNAP })
    const publishSnap = (n: number) => {
      const pub = env.server.publish(SRC, { name: 'Shop', items: parts(n) })
      env.server.put(snapUrl(), new TextEncoder().encode(JSON.stringify(pub.index)))
      return pub
    }

    it('follows the entry\'s snapshot path relative to the catalog folder, with the 4 MiB cap, caches it, and returns rows', async () => {
      publishSnap(2)
      env.fetcher.reset()
      const r = await env.sources.catalogPreview(catEntry())
      expect(r).toMatchObject({ ok: true, stale: false })
      expect(env.fetcher.calls).toEqual([{ url: snapUrl(), maxBytes: MAX_INDEX }])
      expect(env.store.cacheWrites).toHaveLength(1)
      expect('rows' in r && r.rows.map((x) => x.state)).toEqual(['not_in_library', 'not_in_library'])
    })

    it('computes the 5.1 states against the Library for a source that is not added', async () => {
      const pub = publishSnap(4)
      const n = (id: number, over: Partial<LibraryRemoteNote> = {}): LibraryRemoteNote => ({ source: SRC, name: 'Shop', path: 'p', sha256: pub.sha[uuid(id)], fetchedAt: 't', edited: false, ...over })
      env.store.seed('a.hew', { id: uuid(1), name: 'Part 1' })
      env.store.seed('b.hew', { id: uuid(2), name: 'Part 2', remote: n(2) })
      env.store.seed('c.hew', { id: uuid(3), name: 'Part 3', remote: n(3, { sha256: 'f'.repeat(64) }) })
      const r = await env.sources.catalogPreview(catEntry())
      if (!('ok' in r) || !r.ok) throw new Error('preview failed')
      expect(r.rows.map((x) => x.state)).toEqual(['in_library', 'in_step', 'newer', 'not_in_library'])
    })

    it('a failed fetch falls back to the cached copy (stale); with none it is the failure', async () => {
      expect(await env.sources.catalogPreview(catEntry())).toMatchObject({ ok: false, code: 'http_status' })
      publishSnap(1)
      await env.sources.catalogPreview(catEntry())
      env.server.fail((u) => u === snapUrl(), 'network')
      expect(await env.sources.catalogPreview(catEntry())).toMatchObject({ ok: true, stale: true })
    })

    it('a snapshot that is not a valid index falls back to the cache like a failed fetch', async () => {
      publishSnap(1)
      await env.sources.catalogPreview(catEntry())
      env.server.put(snapUrl(), new TextEncoder().encode('{"oops"'))
      expect(await env.sources.catalogPreview(catEntry())).toMatchObject({ ok: true, stale: true })
    })

    it('is Off even with a cached copy: off:true, no request', async () => {
      publishSnap(1)
      await env.sources.catalogPreview(catEntry())
      env.fetcher.on = false
      env.fetcher.reset()
      expect(await env.sources.catalogPreview(catEntry())).toEqual({ off: true })
      expect(env.fetcher.calls).toHaveLength(0)
    })

    it('with the catalog turned off it is catalog_disabled', async () => {
      env.assertClean()
      env = makeEnv({ catalogUrl: '' })
      expect(await env.sources.catalogPreview(catEntry())).toMatchObject({ ok: false, code: 'catalog_disabled' })
      expect(env.fetcher.calls).toHaveLength(0)
    })
  })

  it('is Off: off:true, no request', async () => {
    env.fetcher.on = false
    expect(await env.sources.catalog()).toEqual({ off: true })
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('applies the removed list: an added source in it is S-removed with its reason and link', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    await env.addSource(SRC2, { name: 'Slides', items: [] })
    env.server.publishCatalog({ removed: [{ source: SRC, severity: 'removed', reason: 'Took the files down.', link: 'https://example.com/why' }] })
    const r = await env.sources.catalog()
    expect('ok' in r && r.ok && r.removedSources).toEqual([{ source: SRC, severity: 'removed', reason: 'Took the files down.', link: 'https://example.com/why' }])
    const [a, b] = await env.sources.listSources()
    expect(a.removed).toEqual({ reason: 'Took the files down.', severity: 'removed', link: 'https://example.com/why' })
    expect(b.removed).toBeNull()
  })

  it('matches by canonical reference with any @ref set aside (the added source has a ref the removed entry lacks, and the reverse)', async () => {
    const pinned = `${SRC}@v2`
    await env.addSource(pinned, { name: 'Shop', items: [] })
    env.server.publishCatalog({ removed: [{ source: SRC, severity: 'critical', reason: 'Bad.' }] })
    await env.sources.catalog()
    expect((await env.sources.listSources())[0].removed).toMatchObject({ reason: 'Bad.', severity: 'critical' })
    await env.sources.removeSource(pinned)
    await env.addSource(SRC, { name: 'Shop', items: [] })
    env.server.publishCatalog({ removed: [{ source: `${SRC}@main`, severity: 'removed', reason: 'Other ref.' }] })
    await env.sources.catalog()
    expect((await env.sources.listSources())[0].removed).toMatchObject({ reason: 'Other ref.' })
  })

  it('a different repository is not matched', async () => {
    await env.addSource(SRC, { name: 'Shop', items: [] })
    env.server.publishCatalog({ removed: [{ source: 'github:acme/hinges-two', severity: 'removed', reason: 'x' }] })
    await env.sources.catalog()
    expect((await env.sources.listSources())[0].removed).toBeNull()
  })

  it('S-removed is cleared by a later catalog that no longer lists it', async () => {
    await env.addSource(SRC, { name: 'Shop', items: [] })
    env.server.publishCatalog({ removed: [{ source: SRC, severity: 'removed', reason: 'x' }] })
    await env.sources.catalog()
    expect((await env.sources.listSources())[0].removed).not.toBeNull()
    env.server.publishCatalog({ removed: [] })
    await env.sources.catalog()
    expect((await env.sources.listSources())[0].removed).toBeNull()
  })

  it('S-removed survives a catalog fetch that fails (the cached list still applies)', async () => {
    await env.addSource(SRC, { name: 'Shop', items: [] })
    env.server.publishCatalog({ removed: [{ source: SRC, severity: 'removed', reason: 'x' }] })
    await env.sources.catalog()
    env.server.fail((u) => u.includes('catalog.'), 'network')
    await env.sources.catalog()
    expect((await env.sources.listSources())[0].removed).not.toBeNull()
  })

  it('a removed source gives no bar counts', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(2) })
    await env.sources.follow(SRC)
    env.store.items.clear()
    env.server.publishCatalog({ removed: [{ source: SRC, severity: 'removed', reason: 'x' }] })
    await env.sources.catalog()
    expect(await env.sources.newCounts(SRC)).toEqual({ new: 0, changed: 0 })
  })

  it('check still works on a removed source (only add/insert/follow/update are unavailable)', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.server.publishCatalog({ removed: [{ source: SRC, severity: 'removed', reason: 'x' }] })
    await env.sources.catalog()
    expect(await env.sources.check(SRC)).toMatchObject({ ok: true })
  })
})

describe('check scheduler', () => {
  const HOUR = 3600_000

  it('is due when the last check is over 6 hours old, not before, not at exactly 6 hours', async () => {
    await env.addSource(SRC, { name: 'Shop', items: [] })
    env.clock.advance(6 * HOUR - 1)
    expect(await env.sources.shouldAutoCheck(SRC)).toBe(false)
    env.clock.advance(1)
    expect(await env.sources.shouldAutoCheck(SRC)).toBe(false)
    env.clock.advance(1)
    expect(await env.sources.shouldAutoCheck(SRC)).toBe(true)
  })

  it('is never due with the setting off, in Off, or for an unknown source', async () => {
    await env.addSource(SRC, { name: 'Shop', items: [] })
    env.clock.advance(24 * HOUR)
    env.options.autoCheck = false
    expect(await env.sources.shouldAutoCheck(SRC)).toBe(false)
    env.options.autoCheck = true
    expect(await env.sources.shouldAutoCheck(SRC)).toBe(true)
    env.fetcher.on = false
    expect(await env.sources.shouldAutoCheck(SRC)).toBe(false)
    env.fetcher.on = true
    expect(await env.sources.shouldAutoCheck('github:nobody/nothing')).toBe(false)
  })

  it('the age is that of the last check, whether it succeeded or not', async () => {
    await env.addSource(SRC, { name: 'Shop', items: [] })
    env.clock.advance(7 * HOUR)
    env.server.fail((u) => u.includes('/hew.json'), 'network')
    await env.sources.check(SRC)
    expect(await env.sources.shouldAutoCheck(SRC)).toBe(false)
  })

  it('the catalog follows the same rule, and only when at least one source is added', async () => {
    env.server.publishCatalog({ sources: [] })
    expect(await env.sources.shouldAutoCheckCatalog()).toBe(false) // no source added
    await env.addSource(SRC, { name: 'Shop', items: [] })
    await env.sources.catalog()
    expect(await env.sources.shouldAutoCheckCatalog()).toBe(false)
    env.clock.advance(6 * HOUR + 1)
    expect(await env.sources.shouldAutoCheckCatalog()).toBe(true)
    env.options.autoCheck = false
    expect(await env.sources.shouldAutoCheckCatalog()).toBe(false)
  })

  it('the catalog\'s last-fetch time survives a restart over the same Library', async () => {
    env.server.publishCatalog({ sources: [] })
    await env.addSource(SRC, { name: 'Shop', items: [] })
    await env.sources.catalog()
    const shared = { store: env.store, server: env.server, clock: env.clock }
    env.assertClean()
    env = makeEnv({ shared })
    expect(await env.sources.shouldAutoCheckCatalog()).toBe(false)
    env.clock.advance(6 * HOUR + 1)
    expect(await env.sources.shouldAutoCheckCatalog()).toBe(true)
  })

  it('autoCheck checks only the due sources, then the catalog under the same rule', async () => {
    env.server.publishCatalog({ sources: [] })
    await env.addSource(SRC, { name: 'Shop', items: [] })
    env.clock.advance(7 * HOUR)
    await env.addSource(SRC2, { name: 'Slides', items: [] }) // fresh, not due
    env.fetcher.reset()
    const r = await env.sources.autoCheck()
    expect(r).toEqual({ checked: [SRC], catalogFetched: true })
    expect(env.fetcher.urls().some((u) => u.includes(SRC2))).toBe(false)
    expect(env.fetcher.urls().some((u) => u.includes('catalog.'))).toBe(true)
  })

  it('autoCheck with nothing due, or the setting off, or no source added, makes no request', async () => {
    env.server.publishCatalog({ sources: [] })
    expect(await env.sources.autoCheck()).toEqual({ checked: [], catalogFetched: false }) // no sources
    await env.addSource(SRC, { name: 'Shop', items: [] })
    await env.sources.catalog()
    env.fetcher.reset()
    expect(await env.sources.autoCheck()).toEqual({ checked: [], catalogFetched: false }) // not due
    env.clock.advance(7 * HOUR)
    env.options.autoCheck = false
    expect(await env.sources.autoCheck()).toEqual({ checked: [], catalogFetched: false })
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('autoCheck is Off: off:true, no request', async () => {
    await env.addSource(SRC, { name: 'Shop', items: [] })
    env.clock.advance(7 * HOUR)
    env.fetcher.on = false
    env.fetcher.reset()
    expect(await env.sources.autoCheck()).toEqual({ off: true })
    expect(env.fetcher.calls).toHaveLength(0)
  })
})

describe('Off', () => {
  it('every method that needs the network resolves off:true with no request; the local ones still work', async () => {
    const pub = await env.addSource(SRC, { name: 'Shop', items: [{ ...parts(1)[0], thumbnail: new Uint8Array([1]) }] })
    const id = uuid(1)
    env.store.seed('Components/part-1-000001.hew', { id, name: 'Part 1', remote: { source: SRC, name: 'Shop', path: 'p', sha256: pub.sha[id], fetchedAt: 't', edited: false } })
    env.fetcher.on = false
    env.fetcher.reset()
    const s = env.sources
    const off = { off: true }
    expect(await s.addSource(SRC)).toEqual(off)
    expect(await s.check(SRC)).toEqual(off)
    expect(await s.catalog()).toEqual(off)
    expect(await s.addItem(SRC, id)).toEqual(off)
    expect(await s.updateItem(SRC, id)).toEqual(off)
    expect(await s.addAll(SRC)).toEqual(off)
    expect(await s.follow(SRC)).toEqual(off)
    expect(await s.updateAll(SRC)).toEqual(off)
    expect(await s.thumbnail(SRC, id)).toEqual(off)
    expect(await s.autoCheck()).toEqual(off)
    expect(env.fetcher.calls).toHaveLength(0)
    expect(env.store.writes).toHaveLength(0)
    // No network needed:
    expect((await s.listSources()).length).toBe(1)
    expect(await s.itemStates(SRC)).toMatchObject({ ok: true })
    expect((await s.localStates())[0]).toMatchObject({ id, state: 'in_step' })
    expect(await s.newCounts(SRC)).toEqual({ new: 0, changed: 0 })
    expect(await s.unlink(SRC, id)).toEqual({ ok: true })
    expect(await s.removeSource(SRC)).toEqual({ ok: true })
    expect(env.fetcher.calls).toHaveLength(0)
  })
})

describe('network hygiene', () => {
  it('every request in a full add + check goes through the fetcher with the 6.4 caps', async () => {
    env.server.publishCatalog({ sources: [] })
    await env.addSource(SRC, { name: 'Shop', items: parts(2) })
    await env.sources.check(SRC)
    await env.sources.catalog()
    const caps = new Set(env.fetcher.calls.map((c) => c.maxBytes))
    expect([...caps].sort((a, b) => a - b)).toEqual([MAX_RESOLVE, MAX_INDEX])
  })

  it('failures of every kind put no URL in the result, the thrown errors or the console (checked after each test as well)', async () => {
    env.server.publish(SRC, { name: 'Shop', items: parts(1) })
    for (const code of ['network', 'timeout', 'rate_limited', 'too_large', 'not_https', 'offline', 'too_many_redirects'] as const) {
      env.server.clearRules()
      env.server.fail(() => true, code)
      await env.sources.addSource(SRC)
    }
    env.server.clearRules()
    await env.sources.addSource(SRC)
    env.server.fail(() => true, 'http_status', { status: 500 })
    await env.sources.check(SRC)
    await env.sources.catalog()
    const joined = env.seen.join('\n')
    expect(joined).not.toMatch(/files\.example\.test|api\.example\.test|catalog\.example\.test/)
    expect(joined.length).toBeGreaterThan(0)
  })
})
