/**
 * Thumbnail cache sweep: a cached source thumbnail is kept only while some
 * listed source's cached index still references it. `removeSource` and each
 * successful `check` delete the `thumb-*` entries nothing references any more;
 * a check touches no other cache entry, and a removal deletes only the removed
 * source's own idx-* and snap-* besides (never the catalog).
 *
 * Fixtures: sourcesTestKit.ts.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { SRC, SRC2, makeEnv, parts, sha256, uuid, type Env, type PublishItem } from './sourcesTestKit'

const PNG = new Uint8Array([137, 80, 78, 71, 1, 2, 3])
const PNG2 = new Uint8Array([137, 80, 78, 71, 9, 9, 9])

let env: Env
beforeEach(() => {
  env = makeEnv()
})
afterEach(() => {
  env.assertClean()
})

const withThumb = (n: number, png = PNG): PublishItem => ({ id: uuid(n), name: `Part ${n}`, thumbnail: png })
const cacheKeys = (): string[] => [...env.store.cache.keys()].sort()
const thumbKeys = (): string[] => cacheKeys().filter((k) => k.startsWith('thumb-'))
const otherKeys = (): string[] => cacheKeys().filter((k) => !k.startsWith('thumb-'))

async function warm(source: string, ids: number[]): Promise<void> {
  for (const n of ids) expect(await env.sources.thumbnail(source, uuid(n))).toMatchObject({ ok: true })
}

describe('thumbnail cache sweep', () => {
  it('removing a source deletes its thumbs and its index copy and leaves every other cache entry alone', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: [withThumb(1), withThumb(2, PNG2)] })
    await warm(SRC, [1, 2])
    expect(thumbKeys()).toHaveLength(2)
    const others = otherKeys()
    expect(others.length).toBeGreaterThan(0)
    expect(await env.sources.removeSource(SRC)).toEqual({ ok: true })
    expect(thumbKeys()).toEqual([])
    const idx = `idx-${sha256(new TextEncoder().encode(SRC)).slice(0, 24)}`
    expect(others).toContain(idx)
    expect(otherKeys()).toEqual(others.filter((k) => k !== idx))
  })

  it("removing one source keeps the other source's thumbs", async () => {
    await env.addSource(SRC, { name: 'Hinges', items: [withThumb(1)] })
    await env.addSource(SRC2, { name: 'Slides', items: [withThumb(1)] })
    await warm(SRC, [1])
    await warm(SRC2, [1])
    const both = thumbKeys()
    expect(both).toHaveLength(2)
    await env.sources.removeSource(SRC)
    const left = thumbKeys()
    expect(left).toHaveLength(1)
    expect(both).toContain(left[0])
    // The survivor still serves from the cache with no request.
    env.fetcher.reset()
    expect(await env.sources.thumbnail(SRC2, uuid(1))).toMatchObject({ ok: true })
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('a failed removal sweeps nothing', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: [withThumb(1)] })
    await warm(SRC, [1])
    const before = thumbKeys()
    expect(await env.sources.removeSource('github:acme/unknown')).toMatchObject({ ok: false })
    expect(thumbKeys()).toEqual(before)
  })

  it('a check that drops an item deletes that item’s thumb and keeps the others', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: [withThumb(1), withThumb(2, PNG2)] })
    await warm(SRC, [1, 2])
    const before = thumbKeys()
    expect(before).toHaveLength(2)
    const others = otherKeys()
    env.server.publish(SRC, { name: 'Hinges', items: [withThumb(1)] })
    const r = await env.sources.check(SRC)
    expect(r).toMatchObject({ ok: true })
    const after = thumbKeys()
    expect(after).toHaveLength(1)
    expect(before).toContain(after[0])
    // The kept thumb is the one still listed: served from the cache.
    env.fetcher.reset()
    expect(await env.sources.thumbnail(SRC, uuid(1))).toMatchObject({ ok: true })
    expect(env.fetcher.calls.filter((c) => c.url.includes('thumbnails/'))).toHaveLength(0)
    // idx-* is rewritten by the check; nothing else changed.
    expect(otherKeys()).toEqual(others)
  })

  it('a check that drops nothing deletes nothing', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: [withThumb(1)] })
    await warm(SRC, [1])
    const before = thumbKeys()
    expect(await env.sources.check(SRC)).toMatchObject({ ok: true })
    expect(thumbKeys()).toEqual(before)
  })

  it('a failed check deletes nothing', async () => {
    await env.addSource(SRC, { name: 'Hinges', items: [withThumb(1)] })
    await warm(SRC, [1])
    const before = thumbKeys()
    env.server.publish(SRC, { name: 'Hinges', items: parts(1) })
    env.server.fail(() => true, 'network')
    expect(await env.sources.check(SRC)).toMatchObject({ ok: false })
    expect(thumbKeys()).toEqual(before)
  })

  /** Check a source that dropped item 2, then remove it; what the store was asked and did. */
  async function dropThenRemove(failDelete: boolean) {
    await env.addSource(SRC, { name: 'Hinges', items: [withThumb(1), withThumb(2, PNG2)] })
    await warm(SRC, [1, 2])
    env.store.failCacheRemove = failDelete
    const mark = env.store.log.length
    env.server.publish(SRC, { name: 'Hinges', items: [withThumb(1)] })
    const checked = await env.sources.check(SRC)
    const attemptsAfterCheck = env.store.log.slice(mark).filter((l) => l.startsWith('removeSourceCache thumb-')).length
    const removed = await env.sources.removeSource(SRC)
    const attempts = env.store.log.slice(mark).filter((l) => l.startsWith('removeSourceCache thumb-')).length
    return { checked, removed, attemptsAfterCheck, attempts, deleted: env.store.cacheRemoves.length }
  }

  it('control: with a working store the same check and removal do delete thumbs', async () => {
    const r = await dropThenRemove(false)
    expect(r.checked).toMatchObject({ ok: true })
    expect(r.removed).toEqual({ ok: true })
    expect(r.attemptsAfterCheck).toBeGreaterThan(0)
    expect(r.deleted).toBeGreaterThan(0)
    expect(thumbKeys()).toEqual([])
  })

  it('a store that cannot delete a cache entry does not fail the removal or the check, and was asked to', async () => {
    const r = await dropThenRemove(true)
    expect(r.checked).toMatchObject({ ok: true })
    expect(r.removed).toEqual({ ok: true })
    // The sweep tried (after the check, and again on removal) and nothing was deleted.
    expect(r.attemptsAfterCheck).toBeGreaterThan(0)
    expect(r.attempts).toBeGreaterThan(r.attemptsAfterCheck)
    expect(r.deleted).toBe(0)
  })
})
