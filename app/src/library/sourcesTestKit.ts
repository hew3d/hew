/* oxlint-disable lingui/no-unlocalized-strings -- test fixtures, not UI copy */
// Shared fixtures for the sources-logic oracle (sources*.test.ts): a fake
// RemoteFetcher over a fake index server keyed by URL, a fake LibraryStore
// (in-memory items, `.sources.json` merge through writeSources, source cache),
// fake item tools over a tiny invented byte format, a fake wasm layer, and an
// environment that wires them into createSources and checks the standing
// rules after every test (no URL leaks, every request's maxBytes, no
// whole-file `.sources.json` write).
//
// Not a test file (vitest only runs *.test.ts) and not the implementation.

import { expect, vi } from 'vitest'
import { itemFileName } from './fileNaming'
import { RemoteFetchError, type RemoteFetchCode, type RemoteFetcher } from './remoteFetcher'
import {
  createSources,
  type CatalogData,
  type IndexCategory,
  type IndexItem,
  type ItemFileMeta,
  type SourceEntry,
  type SourceIndex,
  type SourceKind,
  type SourceRef,
  type Sources,
  type SourcesDeps,
  type SourceUrls,
  type StampPatch,
  type WasmError,
} from './sources'
import type { LibraryCategory, LibraryFileEntry, LibraryRemoteNote } from './types'

/** Every URL this kit builds contains this marker (the host of a `web` source
 * legitimately appears in its canonical reference, so the leak check looks for
 * the hosts of the git kinds and the catalog only). */
export const URL_MARK = 'example.test'
export const LEAK_MARKS = [`files.${URL_MARK}`, `api.${URL_MARK}`, `catalog.${URL_MARK}`]

export const MiB = 1024 * 1024
export const MAX_RESOLVE = 64 * 1024
export const MAX_INDEX = 4 * MiB
export const MAX_THUMB = 1 * MiB

const enc = new TextEncoder()
const dec = new TextDecoder()

const K = Uint32Array.from(
  '428a2f98 71374491 b5c0fbcf e9b5dba5 3956c25b 59f111f1 923f82a4 ab1c5ed5 d807aa98 12835b01 243185be 550c7dc3 72be5d74 80deb1fe 9bdc06a7 c19bf174 e49b69c1 efbe4786 0fc19dc6 240ca1cc 2de92c6f 4a7484aa 5cb0a9dc 76f988da 983e5152 a831c66d b00327c8 bf597fc7 c6e00bf3 d5a79147 06ca6351 14292967 27b70a85 2e1b2138 4d2c6dfc 53380d13 650a7354 766a0abb 81c2c92e 92722c85 a2bfe8a1 a81a664b c24b8b70 c76c51a3 d192e819 d6990624 f40e3585 106aa070 19a4c116 1e376c08 2748774c 34b0bcb5 391c0cb3 4ed8aa4a 5b9cca4f 682e6ff3 748f82ee 78a5636f 84c87814 8cc70208 90befffa a4506ceb bef9a3f7 c67178f2'.split(' ').map((h) => parseInt(h, 16)),
)

/** Synchronous SHA-256 as lowercase hex, with no dependency on Node types. */
export function sha256(data: Uint8Array): string {
  const h = Uint32Array.from([0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19])
  const len = data.length
  const padded = new Uint8Array((((len + 9 + 63) >> 6) << 6))
  padded.set(data)
  padded[len] = 0x80
  const view = new DataView(padded.buffer)
  view.setUint32(padded.length - 8, Math.floor((len * 8) / 0x100000000))
  view.setUint32(padded.length - 4, (len * 8) >>> 0)
  const w = new Uint32Array(64)
  const rotr = (x: number, n: number): number => (x >>> n) | (x << (32 - n))
  for (let off = 0; off < padded.length; off += 64) {
    for (let i = 0; i < 16; i++) w[i] = view.getUint32(off + i * 4)
    for (let i = 16; i < 64; i++) {
      const s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >>> 3)
      const s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >>> 10)
      w[i] = (w[i - 16] + s0 + w[i - 7] + s1) >>> 0
    }
    let [a, b, c, d, e, f, g, hh] = h
    for (let i = 0; i < 64; i++) {
      const t1 = (hh + (rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)) + ((e & f) ^ (~e & g)) + K[i] + w[i]) >>> 0
      const t2 = ((rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)) + ((a & b) ^ (a & c) ^ (b & c))) >>> 0
      hh = g
      g = f
      f = e
      e = (d + t1) >>> 0
      d = c
      c = b
      b = a
      a = (t1 + t2) >>> 0
    }
    h[0] += a; h[1] += b; h[2] += c; h[3] += d; h[4] += e; h[5] += f; h[6] += g; h[7] += hh
  }
  return Array.from(h, (x) => x.toString(16).padStart(8, '0')).join('')
}

// ---------------------------------------------------------- fake item format

/** A fake item file: JSON `{ lib: <hew.library keys>, body }`. */
export interface FakeItem {
  id: string
  name: string
  category?: LibraryCategory
  collection?: string
  keywords?: string[]
  body?: string
  remote?: LibraryRemoteNote
}

interface FakeFile {
  lib: Record<string, unknown>
  body: string
}

export function itemBytes(item: FakeItem): Uint8Array {
  const lib: Record<string, unknown> = {
    id: item.id,
    name: item.name,
    category: item.category ?? 'component',
    collection: item.collection ?? '',
    keywords: item.keywords ?? [],
  }
  if (item.remote !== undefined) lib.remote = item.remote
  const f: FakeFile = { lib, body: item.body ?? `body of ${item.name}` }
  return enc.encode(JSON.stringify(f))
}

export function parseFake(bytes: Uint8Array): FakeFile {
  return JSON.parse(dec.decode(bytes)) as FakeFile
}

export function noteOf(bytes: Uint8Array): LibraryRemoteNote | undefined {
  return parseFake(bytes).lib.remote as LibraryRemoteNote | undefined
}

export function collectionOf(bytes: Uint8Array): string {
  return (parseFake(bytes).lib.collection as string | undefined) ?? ''
}

export function metaOf(bytes: Uint8Array): ItemFileMeta {
  const l = parseFake(bytes).lib
  return {
    id: l.id as string,
    name: l.name as string,
    category: l.category as LibraryCategory,
    collection: (l.collection as string | undefined) ?? '',
    keywords: (l.keywords as string[] | undefined) ?? [],
    ...(l.remote !== undefined ? { remote: l.remote as LibraryRemoteNote } : {}),
  }
}

// ------------------------------------------------------------ fake wasm layer

const GIT_PREFIX: Record<string, SourceKind> = { 'github:': 'github', 'gitlab:': 'gitlab', 'forgejo:': 'forgejo' }

function splitRef(s: string): { body: string; gitRef: string | null } {
  const at = s.lastIndexOf('@')
  return at > 0 ? { body: s.slice(0, at), gitRef: s.slice(at + 1) } : { body: s, gitRef: null }
}

function gitRefOf(kind: SourceKind, text: string): SourceRef {
  const { body, gitRef } = splitRef(text.slice(kind.length + 1))
  const host = kind === 'github' ? 'github.com' : kind === 'gitlab' && !body.includes('.') ? 'gitlab.com' : body.split('/')[0]
  return { kind, host, repo: body, dir: '', gitRef, canonical: `${kind}:${body}${gitRef !== null ? `@${gitRef}` : ''}` }
}

function webRef(url: string): SourceRef {
  const u = new URL(url)
  const full = u.pathname.endsWith('/hew.json') ? url : `${url.replace(/\/$/, '')}/hew.json`
  const folder = new URL(full).pathname.replace(/hew\.json$/, '')
  return { kind: 'web', host: u.host, repo: folder, dir: '', gitRef: null, canonical: `web:${full}` }
}

export function fakeParseSourceRef(text: string): SourceRef[] | WasmError {
  const t = text.trim()
  for (const [prefix, kind] of Object.entries(GIT_PREFIX)) {
    if (t.startsWith(prefix) && /^[a-z]+:[A-Za-z0-9._-]+\/[A-Za-z0-9._/-]+(@[A-Za-z0-9._-]+)?$/.test(t)) return [gitRefOf(kind, t)]
  }
  if (t.startsWith('web:https://')) return [webRef(t.slice(4))]
  if (t.startsWith('https://unknown.example.test')) {
    const u = new URL(t)
    const path = u.pathname.replace(/\/$/, '').replace(/^\//, '')
    return [webRef(t), gitRefOf('forgejo', `forgejo:${u.host}/${path}`), gitRefOf('gitlab', `gitlab:${u.host}/${path}`)]
  }
  if (t.startsWith('https://')) return [webRef(t)]
  return { error: { code: 'ref_invalid', reason: 'not a source reference' } }
}

const noRef = (ref: SourceRef): string => (ref.gitRef !== null ? ref.canonical.slice(0, -(ref.gitRef.length + 1)) : ref.canonical)

export function fakeSourceUrls(ref: SourceRef, paths: string[], commit?: string): SourceUrls {
  if (ref.kind === 'web') {
    const folder = ref.canonical.slice(4).replace(/hew\.json$/, '')
    return { resolve: null, index: `${folder}hew.json`, files: paths.map((p) => `${folder}${p}`) }
  }
  const at = commit ?? ref.gitRef ?? 'HEAD'
  const base = `https://files.${URL_MARK}/${noRef(ref)}/${at}`
  return {
    resolve: `https://api.${URL_MARK}/${noRef(ref)}/commit/${ref.gitRef ?? 'HEAD'}`,
    index: `${base}/hew.json`,
    files: paths.map((p) => `${base}/${p}`),
  }
}

export function fakeParseIndex(bytes: Uint8Array): SourceIndex | WasmError {
  let j: unknown
  try {
    j = JSON.parse(dec.decode(bytes))
  } catch {
    return { error: { code: 'index_invalid', reason: 'not json' } }
  }
  const o = j as Partial<SourceIndex>
  if (typeof o.hew !== 'number') return { error: { code: 'index_invalid', reason: 'hew' } }
  if (o.hew > 1) return { error: { code: 'index_too_new', reason: 'newer' } }
  if (typeof o.name !== 'string' || !Array.isArray(o.items)) return { error: { code: 'index_invalid', reason: 'shape' } }
  return o as SourceIndex
}

// ------------------------------------------------------- fake index server

export interface PublishItem extends FakeItem {
  /** Path in the index; default `<Category>/<slug>.hew`. */
  path?: string
  /** Index category when it should differ from the file's (a wrong listing). */
  listedCategory?: IndexCategory
  /** Index id when it should differ from the file's (a wrong listing). */
  listedId?: string
  /** Index name when it should differ from the file's. */
  listedName?: string
  /** Index license for this entry. */
  license?: string | null
  thumbnail?: Uint8Array
  /** Bytes of the file to serve when they should differ from the item. */
  servedBytes?: Uint8Array
}

export interface PublishSpec {
  name: string
  license?: string | null
  commit?: string
  items: PublishItem[]
  /** Extra index fields. */
  header?: Partial<SourceIndex>
}

const INDEX_CAT: Record<LibraryCategory, IndexCategory> = {
  component: 'Components',
  material: 'Materials',
  model: 'Models',
  profile: 'Profiles',
}

export interface Published {
  index: SourceIndex
  /** id -> sha256 of the file the index lists. */
  sha: Record<string, string>
  /** id -> the bytes of the item as published. */
  bytes: Record<string, Uint8Array>
  commit: string | undefined
}

type Behavior =
  | { kind: 'fail'; code: RemoteFetchCode; status?: number; times: number }
  | { kind: 'hang' }
  | { kind: 'throw'; error: Error }

interface Rule {
  match: (url: string) => boolean
  behavior: Behavior
}

export class FakeServer {
  files = new Map<string, Uint8Array>()
  heads = new Map<string, string>()
  rules: Rule[] = []
  /** Every item size ever published: the allowed maxBytes of an item request. */
  itemSizes = new Set<number>()
  catalogUrl = `https://catalog.${URL_MARK}/data/catalog.json`

  private refOf(canonical: string): SourceRef {
    const refs = fakeParseSourceRef(canonical)
    if ('error' in refs) throw new Error(`bad canonical ${canonical}`)
    return refs[0]
  }

  urlFor(canonical: string, path: string, commit?: string): string {
    const ref = this.refOf(canonical)
    return fakeSourceUrls(ref, [path], commit ?? this.heads.get(canonical)).files[0]
  }

  /** Publish (or push a new commit of) a source. Returns what was published. */
  publish(canonical: string, spec: PublishSpec): Published {
    const ref = this.refOf(canonical)
    const commit = ref.kind === 'web' ? undefined : (spec.commit ?? this.heads.get(canonical) ?? 'c1')
    const entries: IndexItem[] = []
    const sha: Record<string, string> = {}
    const bytes: Record<string, Uint8Array> = {}
    const urls = (paths: string[]) => fakeSourceUrls(ref, paths, commit)
    // The ref (branch) points at the newest commit: serve every file there too.
    const put = (path: string, b: Uint8Array): void => {
      this.files.set(urls([path]).files[0], b)
      if (ref.kind !== 'web') this.files.set(fakeSourceUrls(ref, [path]).files[0], b)
    }
    for (const it of spec.items) {
      const category = it.category ?? 'component'
      const path = it.path ?? `${INDEX_CAT[category]}/${it.name.toLowerCase().replace(/[^a-z0-9]+/g, '-')}.hew`
      const b = itemBytes(it)
      const served = it.servedBytes ?? b
      sha[it.id] = sha256(b)
      bytes[it.id] = b
      this.itemSizes.add(b.length)
      this.itemSizes.add(served.length)
      const entry: IndexItem = {
        id: it.listedId ?? it.id,
        path,
        name: it.listedName ?? it.name,
        category: it.listedCategory ?? INDEX_CAT[category],
        sha256: sha[it.id],
        bytes: b.length,
        license: it.license ?? null,
        ...(it.keywords !== undefined ? { keywords: it.keywords } : {}),
        ...(it.collection !== undefined ? { collection: it.collection } : {}),
      }
      put(path, served)
      if (it.thumbnail !== undefined) {
        const tp = `thumbnails/${sha[it.id]}.png`
        entry.thumbnail = tp
        put(tp, it.thumbnail)
      }
      entries.push(entry)
    }
    const index: SourceIndex = { hew: 1, name: spec.name, license: spec.license ?? null, ...spec.header, items: entries }
    this.files.set(urls([]).index, enc.encode(JSON.stringify(index)))
    if (ref.kind !== 'web') this.files.set(fakeSourceUrls(ref, []).index, enc.encode(JSON.stringify(index)))
    if (commit !== undefined) {
      this.heads.set(canonical, commit)
      this.files.set(fakeSourceUrls(ref, []).resolve!, enc.encode(commit))
    }
    return { index, sha, bytes, commit }
  }

  publishCatalog(data: Partial<CatalogData> & { sources?: CatalogData['sources'] }): void {
    const full: CatalogData = { hew: 1, built: '2026-11-23T00:00:00Z', sources: [], removed: [], ...data }
    this.files.set(this.catalogUrl, enc.encode(JSON.stringify(full)))
  }

  put(url: string, bytes: Uint8Array): void {
    this.files.set(url, bytes)
  }

  /** Make requests matching `match` fail with `code`, `times` times (default always). */
  fail(match: (url: string) => boolean, code: RemoteFetchCode, opts: { status?: number; times?: number } = {}): void {
    this.rules.push({ match, behavior: { kind: 'fail', code, status: opts.status, times: opts.times ?? Infinity } })
  }

  hang(match: (url: string) => boolean): void {
    this.rules.push({ match, behavior: { kind: 'hang' } })
  }

  throwPlain(match: (url: string) => boolean, error: Error): void {
    this.rules.push({ match, behavior: { kind: 'throw', error } })
  }

  clearRules(): void {
    this.rules = []
  }
}

export interface Call {
  url: string
  maxBytes: number
}

export class FakeFetcher implements RemoteFetcher {
  on = true
  calls: Call[] = []
  inFlight = 0
  maxInFlight = 0
  constructor(
    private server: FakeServer,
    private events: string[],
  ) {}

  available(): boolean {
    return this.on
  }

  async fetchBytes(url: string, maxBytes: number): Promise<Uint8Array> {
    this.calls.push({ url, maxBytes })
    this.events.push('fetch')
    this.inFlight++
    this.maxInFlight = Math.max(this.maxInFlight, this.inFlight)
    try {
      await Promise.resolve()
      if (!this.on) throw new Error('fetcher used while Off')
      for (const rule of this.server.rules) {
        if (!rule.match(url)) continue
        const b = rule.behavior
        if (b.kind === 'hang') return await new Promise<Uint8Array>(() => undefined)
        if (b.kind === 'throw') throw b.error
        if (b.times > 0) {
          b.times -= 1
          throw new RemoteFetchError(b.code, b.status)
        }
      }
      const body = this.server.files.get(url)
      if (body === undefined) throw new RemoteFetchError('http_status', 404)
      if (body.length > maxBytes) throw new RemoteFetchError('too_large')
      return body
    } finally {
      this.inFlight--
    }
  }

  urls(): string[] {
    return this.calls.map((c) => c.url)
  }

  /** Calls whose URL contains `part`. */
  matching(part: string): Call[] {
    return this.calls.filter((c) => c.url.includes(part))
  }

  reset(): void {
    this.calls = []
    this.maxInFlight = 0
  }
}

// ------------------------------------------------------------- fake store

const ITEM_NAME = /^(?:(?:Components|Materials|Models|Profiles)\/)?[^./][^/]*\.hew$/
const CACHE_KEY = /^[a-z0-9][a-z0-9_-]{0,95}$/

export class FakeStore {
  items = new Map<string, Uint8Array>()
  sourcesFile: { hew: number; sources: SourceEntry[] } | null = null
  cache = new Map<string, Uint8Array>()
  editing: string[] = []
  /** Ordered log of every store call: `write <file>`, `remove <file>`, ... */
  log: string[] = []
  writes: string[] = []
  removes: string[] = []
  sourceWrites: Array<{ source: string; entry: SourceEntry | null }> = []
  cacheWrites: string[] = []
  failWrite = new Set<string>()
  failRemove = new Set<string>()
  failCacheRemove = false
  cacheRemoves: string[] = []
  constructor(public events: string[]) {}

  async list(): Promise<LibraryFileEntry[]> {
    this.log.push('list')
    return [...this.items.entries()].map(([fileName, b]) => ({ fileName, size: b.length, mtimeMs: 0 }))
  }

  async read(file: string): Promise<Uint8Array> {
    this.log.push(`read ${file}`)
    const b = this.items.get(file)
    if (b === undefined) throw new Error(`no such item ${file}`)
    return b
  }

  async write(file: string, bytes: Uint8Array): Promise<void> {
    this.log.push(`write ${file}`)
    this.events.push('write')
    if (!ITEM_NAME.test(file)) throw new Error(`invalid item name ${file}`)
    if (this.failWrite.has(file)) throw new Error('disk full')
    this.writes.push(file)
    this.items.set(file, bytes)
  }

  async remove(file: string): Promise<void> {
    this.log.push(`remove ${file}`)
    this.events.push('remove')
    if (this.failRemove.has(file)) throw new Error('locked')
    this.removes.push(file)
    this.items.delete(file)
  }

  async readSources(): Promise<Uint8Array | null> {
    this.log.push('readSources')
    return this.sourcesFile === null ? null : enc.encode(JSON.stringify(this.sourcesFile))
  }

  async writeSources(source: string, entry: Record<string, unknown> | null): Promise<void> {
    this.log.push(`writeSources ${source}`)
    if (entry !== null && entry.source !== source) throw new Error('entry does not name its source')
    // Re-read, change ONE source's entry, write back (design 7.3).
    const file = this.sourcesFile ?? { hew: 1, sources: [] }
    const rest = file.sources.filter((s) => s.source !== source)
    const at = file.sources.findIndex((s) => s.source === source)
    if (entry === null) {
      this.sourcesFile = { ...file, sources: rest }
    } else {
      const next = [...file.sources]
      if (at >= 0) next[at] = entry as unknown as SourceEntry
      else next.push(entry as unknown as SourceEntry)
      this.sourcesFile = { ...file, sources: next }
    }
    this.sourceWrites.push({ source, entry: entry as SourceEntry | null })
  }

  async readSourceCache(key: string): Promise<Uint8Array | null> {
    this.log.push(`readSourceCache ${key}`)
    if (!CACHE_KEY.test(key)) throw new Error(`invalid cache key ${key}`)
    return this.cache.get(key) ?? null
  }

  async writeSourceCache(key: string, bytes: Uint8Array): Promise<void> {
    this.log.push(`writeSourceCache ${key}`)
    if (!CACHE_KEY.test(key)) throw new Error(`invalid cache key ${key}`)
    this.cacheWrites.push(key)
    this.cache.set(key, bytes)
  }

  async removeSourceCache(key: string): Promise<void> {
    this.log.push(`removeSourceCache ${key}`)
    if (!CACHE_KEY.test(key)) throw new Error(`invalid cache key ${key}`)
    if (this.failCacheRemove) throw new Error('locked')
    this.cacheRemoves.push(key)
    this.cache.delete(key)
  }

  async editingNames(): Promise<string[]> {
    this.log.push('editingNames')
    return [...this.editing]
  }

  /** Put an item in the Library without going through write (a seeded fixture). */
  seed(file: string, item: FakeItem): string {
    this.items.set(file, itemBytes(item))
    return file
  }

  entryFor(source: string): SourceEntry | undefined {
    return this.sourcesFile?.sources.find((s) => s.source === source)
  }

  resetLogs(): void {
    this.log = []
    this.writes = []
    this.removes = []
    this.sourceWrites = []
    this.cacheWrites = []
    this.cacheRemoves = []
  }

  /** Snapshot of the item files (for "the Library is untouched"). */
  snapshot(): Map<string, Uint8Array> {
    return new Map(this.items)
  }
}

// ------------------------------------------------------------ environment

export interface EnvOptions {
  profiles?: boolean
  catalogUrl?: string
  autoCheck?: boolean
  storage?: { usage: number; quota: number } | null
  /** Reuse a store and server (simulates a restart over the same Library). */
  shared?: { store: FakeStore; server: FakeServer; clock?: Clock }
}

export interface Clock {
  t: number
  advance(ms: number): void
  iso(): string
}

export function makeClock(start = '2026-11-23T12:00:00.000Z'): Clock {
  const c: Clock = {
    t: Date.parse(start),
    advance(ms) {
      c.t += ms
    },
    iso: () => new Date(c.t).toISOString(),
  }
  return c
}

export interface Env {
  sources: Sources
  server: FakeServer
  fetcher: FakeFetcher
  store: FakeStore
  clock: Clock
  /** Ordered: fetch, readMeta, stamp, write, remove, sha. */
  events: string[]
  lockNames: string[]
  busyLocks: Set<string>
  options: { profiles: boolean; autoCheck: boolean; storage: { usage: number; quota: number } | null }
  stampCalls: Array<{ patch: StampPatch }>
  /** Everything the module returned or threw, as text (URL leak check). */
  seen: string[]
  consoleSpies: ReturnType<typeof vi.spyOn>[]
  /** Publish `spec` as `canonical` and add it as a source. */
  addSource(canonical: string, spec: PublishSpec): Promise<Published>
  assertClean(): void
}

function safeJson(v: unknown): string {
  try {
    return JSON.stringify(v, (_k, x) => (x instanceof Uint8Array ? `bytes(${x.length})` : x)) ?? ''
  } catch {
    return String(v)
  }
}

export function makeEnv(opts: EnvOptions = {}): Env {
  const events: string[] = []
  const clock = opts.shared?.clock ?? makeClock()
  const server = opts.shared?.server ?? new FakeServer()
  const store = opts.shared?.store ?? new FakeStore(events)
  store.events = events
  const fetcher = new FakeFetcher(server, events)
  const lockNames: string[] = []
  const busyLocks = new Set<string>()
  const stampCalls: Array<{ patch: StampPatch }> = []
  const options = { profiles: opts.profiles ?? true, autoCheck: opts.autoCheck ?? true, storage: opts.storage ?? null }
  const seen: string[] = []

  const deps: SourcesDeps = {
    fetcher,
    store,
    wasm: {
      parseIndex: fakeParseIndex,
      parseSourceRef: fakeParseSourceRef,
      sourceUrls: fakeSourceUrls,
      commitFromResponse: (_kind, body) => dec.decode(body).trim() || null,
    },
    items: {
      readMeta: async (bytes) => {
        events.push('readMeta')
        let f: FakeFile
        try {
          f = parseFake(bytes)
        } catch {
          throw new Error('not a Library item')
        }
        if (typeof f.lib?.id !== 'string') throw new Error('not a Library item')
        return metaOf(bytes)
      },
      stamp: async (bytes, patch) => {
        events.push('stamp')
        stampCalls.push({ patch })
        const f = parseFake(bytes)
        if ('remote' in patch) {
          if (patch.remote === null) delete f.lib.remote
          else f.lib.remote = patch.remote
        }
        if (patch.collection !== undefined) f.lib.collection = patch.collection
        return enc.encode(JSON.stringify(f))
      },
      sha256Hex: async (bytes) => {
        events.push('sha')
        return sha256(bytes)
      },
      itemFileName: (m) => itemFileName(m.name, m.id, m.category),
    },
    withSourceLock: async (name, fn) => {
      lockNames.push(name)
      if (busyLocks.has(name)) return { ran: false }
      return { ran: true, value: await fn() }
    },
    now: () => new Date(clock.t),
    catalogUrl: opts.catalogUrl ?? server.catalogUrl,
    profilesEnabled: () => options.profiles,
    autoCheckEnabled: () => options.autoCheck,
    storageEstimate: async () => options.storage,
  }

  const consoleSpies = (['log', 'info', 'warn', 'error', 'debug'] as const).map((m) => vi.spyOn(console, m).mockImplementation(() => undefined))

  // Wrap every method so what comes back (or is thrown) can be scanned for URLs.
  const real = createSources(deps)
  const wrapped = {} as Record<string, unknown>
  const text = (e: unknown): string => (e instanceof Error ? `${e.name}: ${e.message}` : String(e))
  for (const key of Object.keys(real) as Array<keyof Sources>) {
    const fn = real[key] as unknown as (...a: unknown[]) => unknown
    wrapped[key] = (...args: unknown[]) => {
      try {
        const r = fn(...args)
        if (r instanceof Promise) {
          return r.then(
            (v: unknown) => {
              seen.push(safeJson(v))
              return v
            },
            (e: unknown) => {
              seen.push(text(e))
              throw e
            },
          )
        }
        seen.push(safeJson(r))
        return r
      } catch (e) {
        seen.push(text(e))
        throw e
      }
    }
  }
  const sources = wrapped as unknown as Sources

  const env: Env = {
    sources,
    server,
    fetcher,
    store,
    clock,
    events,
    lockNames,
    busyLocks,
    options,
    stampCalls,
    seen,
    consoleSpies,
    async addSource(canonical, spec) {
      const pub = server.publish(canonical, spec)
      const r = await sources.addSource(canonical)
      if (!('ok' in r) || !r.ok) throw new Error(`fixture addSource failed: ${safeJson(r)}`)
      return pub
    },
    assertClean() {
      const allowed = new Set([MAX_RESOLVE, MAX_INDEX, MAX_THUMB, ...server.itemSizes])
      for (const c of fetcher.calls) {
        expect(c.url.startsWith('https://'), 'every request is https').toBe(true)
        expect(allowed.has(c.maxBytes), `maxBytes ${c.maxBytes} is one of 6.4's caps or an item's bytes`).toBe(true)
      }
      for (const s of seen) for (const m of LEAK_MARKS) expect(s, 'no URL in a result or a thrown error').not.toContain(m)
      for (const spy of consoleSpies) {
        for (const args of spy.mock.calls) for (const m of LEAK_MARKS) expect(safeJson(args), 'no URL in a log line').not.toContain(m)
      }
      for (const w of store.writes) expect(w.startsWith('.'), '.sources.json is never written as an item').toBe(false)
      for (const l of store.log) expect(l.startsWith('write .'), 'no whole-file write of a dot-file').toBe(false)
      consoleSpies.forEach((s) => s.mockRestore())
    },
  }
  return env
}

/** A tiny index with `n` components named `Part 1..n` and ids with distinct
 * first-six-hex prefixes. */
export function uuid(n: number): string {
  const hex = n.toString(16).padStart(6, '0')
  return `${hex}00-0000-4000-8000-${n.toString(16).padStart(12, '0')}`
}

export function parts(n: number, extra: Partial<PublishItem> = {}): PublishItem[] {
  return Array.from({ length: n }, (_v, i) => ({ id: uuid(i + 1), name: `Part ${i + 1}`, ...extra }))
}

/** A newer version of a published item (same id, new body). */
export function bump(item: PublishItem, body = 'v2'): PublishItem {
  return { ...item, body }
}

export const SRC = 'github:acme/hinges'
export const SRC2 = 'github:acme/slides'
