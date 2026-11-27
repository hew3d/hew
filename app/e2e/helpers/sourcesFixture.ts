/**
 * Fake Library sources for the browser suite (docs/design/shared-library-sources.md
 * section 11). Every response the app reads from a source host or from the
 * catalog is served from here through `page.route`, and every request that
 * leaves the app's own origin is logged, so a spec can check the privacy lines.
 *
 * The item bytes are REAL `.hew` library files, made by the kernel
 * (`hew-cli run` then `hew.library.save`) and committed under
 * `e2e/fixtures/sources/`, with the minted ids in `ids.json`. Each index entry's
 * sha256 and byte count are computed here from those bytes, so a served index
 * always matches the files it lists unless a spec changes one on purpose.
 *
 * Regenerating the fixtures is a one-off (it needs target/debug/hew-cli): for
 * each item, `hew-cli run` a `hew.doc.transact` that draws a rectangle,
 * extrudes it and runs `hew.component.create` with `members`; then
 * `hew-cli dispatch hew.library.save` with `category: "component"` and
 * `return_bytes: true`. Write the base64 `bytes_base64` to `part-NN.hew` and
 * the reply's `id` into `ids.json`. A component is needed: the kernel files a
 * plain solid as a model, and a source index lists it as a component.
 *
 * A NEWER version of an item (`part-NN-v2.hew`) must carry the SAME
 * `hew.library.id` as `part-NN.hew`, or the app refuses it as
 * `listing_incorrect`. Make it with a replacing save against a library that
 * holds the committed original, never against some other library:
 *   mkdir -p lib/Components; cp part-01.hew lib/Components/
 *   (same script as above, taller extrusion) hew-cli run q.json --out q.hew
 *   HEW_LIBRARY_DIR=$PWD/lib hew-cli dispatch hew.library.save \
 *     '{"name":"Part 01","category":"component","replace":"<id from ids.json>","source_doc":"fixture","return_bytes":true}' --file q.hew
 * and write the reply's `bytes_base64` to `part-01-v2.hew`.
 */
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import type { Page, Request, Route } from '@playwright/test'

export const SOURCE_HOST = 'https://parts.example.test'
export const CATALOG_URL = 'https://raw.githubusercontent.com/hew3d/catalog/data/catalog.json'
export const CATALOG_DATA_URL = 'https://raw.githubusercontent.com/hew3d/catalog/data/'

/** A 1 x 1 transparent PNG, the thumbnail every fixture item serves. */
export const THUMB_PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==',
  'base64',
)

const FIXTURE_DIR = join(dirname(fileURLToPath(import.meta.url)), '..', 'fixtures', 'sources')

export const sha256Hex = (bytes: Buffer | Uint8Array): string => createHash('sha256').update(bytes).digest('hex')

/** One real library item file from the fixtures, with the id the kernel minted. */
export interface FixtureItem {
  /** `part-01` … `part-12`. */
  key: string
  /** The display name the item carries (its Library name). */
  name: string
  /** The `hew.library` id inside the file. */
  id: string
  bytes: Buffer
  sha256: string
}

const IDS = JSON.parse(readFileSync(join(FIXTURE_DIR, 'ids.json'), 'utf8')) as Record<string, string>

/** Loads fixture item `n` (1-based, up to 16: 1-14 and the extras are components, 15 is a material, 16 a model). Bytes come from the committed file. */
export function fixtureItem(n: number): FixtureItem {
  const key = `part-${String(n).padStart(2, '0')}`
  const id = IDS[key]
  if (id === undefined) throw new Error(`no fixture id for ${key}`)
  const bytes = readFileSync(join(FIXTURE_DIR, `${key}.hew`))
  return { key, name: `Part ${String(n).padStart(2, '0')}`, id, bytes, sha256: sha256Hex(bytes) }
}

/** The newer version of fixture item `n` (same id, changed body); only part 1 has one. */
export function fixtureNewer(n: number): FixtureItem {
  const base = fixtureItem(n)
  const bytes = readFileSync(join(FIXTURE_DIR, `${base.key}-v2.hew`))
  return { ...base, bytes, sha256: sha256Hex(bytes) }
}

/** The index entry for a fixture item served at `path` under the source's folder. */
export function indexEntry(item: FixtureItem, opts: { path?: string; collection?: string; thumbnail?: boolean; id?: string; name?: string; category?: string } = {}) {
  const path = opts.path ?? `Components/${item.key}.hew`
  return {
    id: opts.id ?? item.id,
    path,
    name: opts.name ?? item.name,
    category: opts.category ?? 'Components',
    keywords: ['part'],
    collection: opts.collection ?? null,
    sha256: item.sha256,
    bytes: item.bytes.length,
    thumbnail: opts.thumbnail === true ? `thumbnails/${item.sha256}.png` : null,
    license: null,
  }
}

export interface SourceIndexJson {
  hew: number
  name: string
  description?: string
  author?: string
  homepage?: string
  license?: string | null
  licenseUrl?: string | null
  items: ReturnType<typeof indexEntry>[]
  plugins?: unknown[]
}

/** A reply the fake host sends for one URL. */
export interface Reply {
  status?: number
  body: Buffer | string
  type?: string
  /** False: no `Access-Control-Allow-Origin` header, so a browser refuses to read it. */
  cors?: boolean
  /** Holds the answer this long, so a spec can act while a run is in flight. */
  delayMs?: number
}

/** A request that left the app's origin, as the page made it. */
export interface OutsideRequest {
  url: string
  method: string
  resourceType: string
}

/**
 * The fake network. `install(page)` routes the source host, the catalog and
 * its data directory; `serve(url, reply)` sets what one URL answers; anything
 * unset answers 404. `outside` logs every request to another origin, whether
 * the fake answered it or not.
 */
export class FakeWeb {
  private readonly replies = new Map<string, Reply | 'abort'>()
  readonly outside: OutsideRequest[] = []
  readonly served: string[] = []

  serve(url: string, reply: Reply | 'abort'): void {
    this.replies.set(url, reply)
  }

  unserve(url: string): void {
    this.replies.delete(url)
  }

  /** Publish an index and its item files under `folder` (a URL ending in `/`). */
  publishSource(folder: string, index: SourceIndexJson, files: Array<{ path: string; bytes: Buffer }>, thumbs: Array<{ path: string; bytes: Buffer }> = []): void {
    this.serve(`${folder}hew.json`, { body: JSON.stringify(index), type: 'application/json' })
    for (const f of files) this.serve(`${folder}${f.path}`, { body: f.bytes, type: 'application/octet-stream' })
    for (const t of thumbs) this.serve(`${folder}${t.path}`, { body: t.bytes, type: 'image/png' })
  }

  /** Publish the catalog and one snapshot per listed source (the catalog's own
   *  `snapshot` paths resolve against the catalog's data folder). */
  publishCatalog(entries: CatalogEntry[], removed: unknown[] = []): void {
    const sources = entries.map((e) => {
      const slug = createHash('sha256').update(e.ref).digest('hex').slice(0, 16)
      const snapshot = `sources/${slug}.json`
      this.serve(`${CATALOG_DATA_URL}${snapshot}`, { body: JSON.stringify(e.index), type: 'application/json' })
      return {
        source: e.ref,
        kind: 'web',
        name: e.index.name,
        description: e.index.description ?? '',
        author: e.index.author ?? null,
        homepage: e.index.homepage ?? null,
        license: e.index.license ?? null,
        licenseUrl: null,
        items: e.index.items.length,
        bytes: e.index.items.reduce((n, i) => n + i.bytes, 0),
        categories: {},
        stale: false,
        snapshot,
      }
    })
    const catalog = { hew: 1, built: '2026-11-23T00:00:00Z', sources, removed }
    this.serve(CATALOG_URL, { body: JSON.stringify(catalog), type: 'application/json' })
  }

  /** Route the fake hosts for this page. Call before the first `goto`. */
  async install(page: Page): Promise<void> {
    page.on('request', (req: Request) => {
      const url = req.url()
      if (url.startsWith('http://127.0.0.1') || url.startsWith('blob:') || url.startsWith('data:')) return
      this.outside.push({ url, method: req.method(), resourceType: req.resourceType() })
    })
    await page.route(/^https:\/\/(parts\.example\.test|raw\.githubusercontent\.com|api\.github\.com)\//, (route) => this.handle(route))
  }

  private async handle(route: Route): Promise<void> {
    const url = route.request().url().split('?')[0]
    const hit = this.replies.get(url)
    if (hit === 'abort') {
      await route.abort('failed')
      return
    }
    this.served.push(url)
    const reply: Reply = hit ?? { status: 404, body: 'not found', type: 'text/plain' }
    if (reply.delayMs !== undefined) await new Promise((r) => setTimeout(r, reply.delayMs))
    const headers: Record<string, string> = {}
    if (reply.cors !== false) headers['Access-Control-Allow-Origin'] = '*'
    await route.fulfill({
      status: reply.status ?? 200,
      headers,
      contentType: reply.type ?? 'application/octet-stream',
      body: reply.body,
    })
  }
}

export interface CatalogEntry {
  /** Canonical reference, e.g. `web:https://parts.example.test/a/hew.json`. */
  ref: string
  index: SourceIndexJson
}

/** The canonical `web:` reference for an index URL. */
export const webRef = (indexUrl: string): string => `web:${indexUrl}`

/** A small, honest source: `count` fixture items, the first four with thumbnails. */
export function sourceOf(name: string, folder: string, count: number, opts: { license?: string | null; collection?: string } = {}) {
  const items: ReturnType<typeof indexEntry>[] = []
  const files: Array<{ path: string; bytes: Buffer }> = []
  const thumbs: Array<{ path: string; bytes: Buffer }> = []
  for (let n = 1; n <= count; n++) {
    const item = fixtureItem(n)
    const withThumb = n <= 4
    const path = `Components/${item.key}.hew`
    items.push(indexEntry(item, { path, thumbnail: withThumb, collection: opts.collection }))
    files.push({ path, bytes: item.bytes })
    if (withThumb) thumbs.push({ path: `thumbnails/${item.sha256}.png`, bytes: THUMB_PNG })
  }
  const index: SourceIndexJson = {
    hew: 1,
    name,
    description: 'Fixture parts for the browser suite.',
    author: 'Example Maker',
    homepage: 'https://example.test/parts',
    license: opts.license === undefined ? 'CC-BY-4.0' : opts.license,
    licenseUrl: null,
    items,
    plugins: [],
  }
  return { index, files, thumbs, folder }
}

/**
 * A source holding one MATERIAL (fixture 15, "Part 15") and one MODEL
 * (fixture 16, "Part 16"), for the acceptance line that adds a material and a
 * model. Their files are real kernel output like the components (a palette
 * material saved alone; a whole document saved as a model).
 */
export function sourceOfMaterialAndModel(name: string, folder: string) {
  const material = fixtureItem(15)
  const model = fixtureItem(16)
  const materialPath = `Materials/${material.key}.hew`
  const modelPath = `Models/${model.key}.hew`
  const index: SourceIndexJson = {
    hew: 1,
    name,
    description: 'A material and a model.',
    author: 'Example Maker',
    homepage: 'https://example.test/parts',
    license: 'CC-BY-4.0',
    licenseUrl: null,
    items: [
      indexEntry(material, { path: materialPath, category: 'Materials' }),
      indexEntry(model, { path: modelPath, category: 'Models' }),
    ],
    plugins: [],
  }
  const files = [
    { path: materialPath, bytes: material.bytes },
    { path: modelPath, bytes: model.bytes },
  ]
  return { index, files, thumbs: [] as Array<{ path: string; bytes: Buffer }>, folder }
}

/** Publish a `sourceOf` result on `host` at its folder, and return its index URL. */
export function publish(host: FakeWeb, s: ReturnType<typeof sourceOf>): string {
  host.publishSource(s.folder, s.index, s.files, s.thumbs)
  return `${s.folder}hew.json`
}
