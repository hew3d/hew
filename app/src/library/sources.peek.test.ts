/**
 * `peekSource`: what Add would find, read-only (S11b). It writes neither
 * `.sources.json` nor the cache, so the Add dialog's Cancel has nothing to undo.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { SRC, SRC2, makeEnv, parts, type Env } from './sourcesTestKit'

let env: Env
beforeEach(() => {
  env = makeEnv()
})
afterEach(() => {
  env.assertClean()
})

const catalogWith = (sources: string[]): void =>
  env.server.publishCatalog({
    sources: sources.map((source) => ({ source, kind: 'github', name: 'x', items: 1, bytes: 1, categories: {}, stale: false, snapshot: 'sources/x.json' })),
  })

describe('peekSource', () => {
  it('returns the summary, host and listing flag and writes nothing', async () => {
    catalogWith([SRC])
    env.server.publish(SRC, { name: 'Shop Hardware', license: 'MIT', items: parts(3) })
    const r = await env.sources.peekSource(SRC)
    expect(r).toMatchObject({ ok: true, existing: false, source: SRC, listing: 'listed' })
    if (!('ok' in r) || !r.ok) return
    expect(r.summary).toMatchObject({ name: 'Shop Hardware', license: 'MIT', itemCount: 3 })
    expect(r.host.length).toBeGreaterThan(0)
    expect(env.store.sourceWrites).toEqual([])
    expect(env.store.cacheWrites).toEqual([])
    expect(await env.sources.listSources()).toEqual([])
  })

  it('flags a source the catalog does not list as unlisted', async () => {
    catalogWith([SRC2])
    env.server.publish(SRC, { name: 'Shop Hardware', items: parts(1) })
    expect(await env.sources.peekSource(SRC)).toMatchObject({ ok: true, listing: 'unlisted' })
  })

  it('does not write the catalog cache when it has to fetch the catalog', async () => {
    catalogWith([SRC])
    env.server.publish(SRC, { name: 'Shop Hardware', items: parts(1) })
    await env.sources.peekSource(SRC)
    expect(env.fetcher.calls.some((c) => c.url.includes('catalog.'))).toBe(true)
    expect(env.store.cacheWrites).toEqual([])
  })

  it('addSource after a peek does the writing', async () => {
    catalogWith([])
    env.server.publish(SRC, { name: 'Shop Hardware', items: parts(1) })
    await env.sources.peekSource(SRC)
    expect(env.store.sourceWrites).toEqual([])
    const added = await env.sources.addSource(SRC)
    expect(added).toMatchObject({ ok: true, existing: false })
    expect(env.store.sourceWrites).toHaveLength(1)
    expect((await env.sources.listSources()).map((s) => s.entry.source)).toEqual([SRC])
  })

  it('reports an existing source from its cached index without a request or a write', async () => {
    catalogWith([])
    env.server.publish(SRC, { name: 'Shop Hardware', items: parts(2) })
    await env.sources.addSource(SRC)
    const writes = env.store.sourceWrites.length
    const cacheWrites = env.store.cacheWrites.length
    env.fetcher.calls.length = 0
    const r = await env.sources.peekSource(SRC)
    expect(r).toMatchObject({ ok: true, existing: true, source: SRC })
    expect(env.store.sourceWrites).toHaveLength(writes)
    expect(env.store.cacheWrites).toHaveLength(cacheWrites)
  })

  it('refuses a reference that is not one and a source that is not there, writing nothing', async () => {
    expect(await env.sources.peekSource('not a source')).toMatchObject({ ok: false, code: 'ref_invalid' })
    env.server.put('https://nothing.example.test/hew.json', new TextEncoder().encode('<html>'))
    const r = await env.sources.peekSource('https://nothing.example.test/hew.json')
    expect(r).toMatchObject({ ok: false })
    expect(env.store.sourceWrites).toEqual([])
    expect(env.store.cacheWrites).toEqual([])
  })

  it('refuses an index from a newer Hew', async () => {
    env.server.publish(SRC, { name: 'Future', header: { hew: 2 } as never, items: [] })
    expect(await env.sources.peekSource(SRC)).toMatchObject({ ok: false, code: 'index_too_new' })
    expect(env.store.sourceWrites).toEqual([])
  })
})
