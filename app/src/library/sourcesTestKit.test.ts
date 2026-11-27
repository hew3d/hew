/**
 * Sanity checks of the sources oracle's own fixtures (sourcesTestKit.ts), so a
 * failure in sources*.test.ts can be trusted to be the implementation's. These
 * pass today.
 */
import { describe, expect, it } from 'vitest'
import { RemoteFetchError } from './remoteFetcher'
import { FakeServer, FakeFetcher, FakeStore, SRC, fakeParseIndex, fakeParseSourceRef, fakeSourceUrls, itemBytes, metaOf, noteOf, parts, sha256, uuid } from './sourcesTestKit'

describe('kit: fake index server and fetcher', () => {
  it('serves the index and files at the commit and at the ref, and the resolve response', async () => {
    const server = new FakeServer()
    const pub = server.publish(SRC, { name: 'Shop', items: parts(2) })
    const f = new FakeFetcher(server, [])
    const refs = fakeParseSourceRef(SRC)
    if ('error' in refs) throw new Error('parse')
    const at = fakeSourceUrls(refs[0], ['Components/part-1.hew'], 'c1')
    const byRef = fakeSourceUrls(refs[0], ['Components/part-1.hew'])
    expect(sha256(await f.fetchBytes(at.files[0], 1 << 20))).toBe(pub.sha[uuid(1)])
    expect(sha256(await f.fetchBytes(byRef.files[0], 1 << 20))).toBe(pub.sha[uuid(1)])
    expect(new TextDecoder().decode(await f.fetchBytes(at.resolve!, 100))).toBe('c1')
    const idx = fakeParseIndex(await f.fetchBytes(at.index, 1 << 20))
    expect('error' in idx).toBe(false)
  })

  it('rejects like a real fetcher: 404 as http_status, a long body as too_large, rules by code', async () => {
    const server = new FakeServer()
    server.publish(SRC, { name: 'Shop', items: parts(1) })
    const f = new FakeFetcher(server, [])
    await expect(f.fetchBytes('https://nothing.example.test/x', 10)).rejects.toMatchObject({ code: 'http_status', status: 404 })
    const url = server.urlFor(SRC, 'Components/part-1.hew', 'c1')
    await expect(f.fetchBytes(url, 5)).rejects.toBeInstanceOf(RemoteFetchError)
    await expect(f.fetchBytes(url, 5)).rejects.toMatchObject({ code: 'too_large' })
    server.fail((u) => u === url, 'rate_limited', { times: 1 })
    await expect(f.fetchBytes(url, 1 << 20)).rejects.toMatchObject({ code: 'rate_limited' })
    expect((await f.fetchBytes(url, 1 << 20)).length).toBeGreaterThan(5)
  })

  it('the unknown-host address gives the three candidates in order web, forgejo, gitlab', () => {
    const r = fakeParseSourceRef('https://unknown.example.test/parts')
    if ('error' in r) throw new Error('parse')
    expect(r.map((x) => x.kind)).toEqual(['web', 'forgejo', 'gitlab'])
  })

  it('a @ref is split off into gitRef and kept in the canonical form', () => {
    const r = fakeParseSourceRef('github:acme/hinges@v2')
    if ('error' in r) throw new Error('parse')
    expect(r[0]).toMatchObject({ gitRef: 'v2', canonical: 'github:acme/hinges@v2', kind: 'github' })
  })
})

describe('kit: fake store', () => {
  it('merges one source entry at a time and refuses a mismatched entry', async () => {
    const store = new FakeStore([])
    const e = (source: string) => ({ source, kind: 'github', name: source, followed: false, addedAt: 't', lastCheck: null, skipped: [], kept: {} })
    await store.writeSources('a', e('a'))
    await store.writeSources('b', e('b'))
    await store.writeSources('a', { ...e('a'), followed: true })
    expect(store.sourcesFile?.sources.map((s) => [s.source, s.followed])).toEqual([
      ['a', true],
      ['b', false],
    ])
    await store.writeSources('a', null)
    expect(store.sourcesFile?.sources.map((s) => s.source)).toEqual(['b'])
    await expect(store.writeSources('a', e('b'))).rejects.toThrow()
  })

  it('validates item names and cache keys like the real store', async () => {
    const store = new FakeStore([])
    await expect(store.write('.sources.json', new Uint8Array(1))).rejects.toThrow()
    await expect(store.write('Components/a/b.hew', new Uint8Array(1))).rejects.toThrow()
    await store.write('Components/a-000001.hew', new Uint8Array(1))
    await expect(store.writeSourceCache('Bad_Key', new Uint8Array(1))).rejects.toThrow()
    await store.writeSourceCache('idx-ok_1', new Uint8Array(1))
  })
})

describe('kit: fake item bytes', () => {
  it('round-trips meta and the remote note', () => {
    const note = { source: SRC, name: 'n', path: 'p', sha256: 'x', fetchedAt: 't', edited: false }
    const b = itemBytes({ id: uuid(1), name: 'Part 1', collection: 'C', remote: note })
    expect(metaOf(b)).toMatchObject({ id: uuid(1), name: 'Part 1', category: 'component', collection: 'C' })
    expect(noteOf(b)).toEqual(note)
  })
})

describe('kit: sha256', () => {
  it('matches the known vectors', () => {
    const e = new TextEncoder()
    expect(sha256(e.encode(''))).toBe('e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855')
    expect(sha256(e.encode('abc'))).toBe('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad')
    expect(sha256(e.encode('a'.repeat(100)))).toBe('2816597888e4a0d3a36b82b83316ab32680eb8f00f8cd3b904d681246d285a0e')
  })
})
