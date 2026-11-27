/* oxlint-disable lingui/no-unlocalized-strings -- test fixtures */
import { beforeEach, describe, expect, it, vi } from 'vitest'

const m = vi.hoisted(() => ({
  parseIndex: vi.fn(),
  parseSourceRef: vi.fn(),
  sourceUrls: vi.fn(),
  commitFromResponse: vi.fn(),
  readItemSummary: vi.fn(),
  updateItemMeta: vi.fn(),
  sources: [] as Array<() => void>,
  items: [] as Array<() => void>,
}))

vi.mock('./sourcesWasm', () => {
  class SourceWasmError extends Error {
    code: string
    reason: string
    constructor(code: string, reason: string) {
      super(code)
      this.code = code
      this.reason = reason
    }
  }
  return {
    SourceWasmError,
    parseIndex: m.parseIndex,
    parseSourceRef: m.parseSourceRef,
    sourceUrls: m.sourceUrls,
    commitFromResponse: m.commitFromResponse,
  }
})
vi.mock('./itemFiles', () => ({
  readItemSummary: m.readItemSummary,
  updateItemMeta: m.updateItemMeta,
  sha256Hex: async () => 'abc',
}))
vi.mock('../wasm/loader', () => ({ loadKernel: async () => ({}) }))
vi.mock('../io/libraryStore', () => ({
  libraryStore: () => ({
    subscribeSources: (l: () => void) => {
      m.sources.push(l)
      return () => m.sources.splice(m.sources.indexOf(l), 1)
    },
    subscribe: (l: () => void) => {
      m.items.push(l)
      return () => m.items.splice(m.items.indexOf(l), 1)
    },
  }),
}))
vi.mock('./remoteFetcher', async (orig) => ({
  ...(await orig<typeof import('./remoteFetcher')>()),
  remoteFetcher: () => ({ available: () => true, fetchBytes: async () => new Uint8Array() }),
}))

import { SourceWasmError } from './sourcesWasm'
import { adaptWasm, catalogUrlFor, createDefaultBinding, DEFAULT_CATALOG_URL, itemTools, realSourcesDeps } from './sourcesDeps'

const enc = new TextEncoder()

beforeEach(() => {
  vi.clearAllMocks()
  m.sources.length = 0
  m.items.length = 0
})

describe('catalogUrlFor', () => {
  it('defaults to the Hew catalog, is replaceable, and empty hides it', () => {
    expect(catalogUrlFor({})).toBe(DEFAULT_CATALOG_URL)
    expect(DEFAULT_CATALOG_URL).toBe('https://raw.githubusercontent.com/hew3d/catalog/data/catalog.json')
    expect(catalogUrlFor({ VITE_HEW_CATALOG_URL: 'https://catalog.example.org/c.json' })).toBe('https://catalog.example.org/c.json')
    expect(catalogUrlFor({ VITE_HEW_CATALOG_URL: '' })).toBe('')
    expect(catalogUrlFor({ VITE_HEW_CATALOG_URL: '  ' })).toBe('')
  })
})

describe('adaptWasm', () => {
  it('parseIndex decodes the bytes and returns a wasm refusal as a value', () => {
    m.parseIndex.mockReturnValueOnce({ hew: 1, name: 'X', items: [] })
    expect(adaptWasm().parseIndex(enc.encode('{"hew":1}'))).toEqual({ hew: 1, name: 'X', items: [] })
    expect(m.parseIndex).toHaveBeenCalledWith('{"hew":1}')
    m.parseIndex.mockImplementationOnce(() => {
      throw new SourceWasmError('index_too_new', 'newer')
    })
    expect(adaptWasm().parseIndex(enc.encode('{}'))).toEqual({ error: { code: 'index_too_new', reason: 'newer' } })
  })

  it('parseSourceRef maps git_ref to gitRef; sourceUrls maps it back', () => {
    m.parseSourceRef.mockReturnValueOnce([{ kind: 'github', host: 'github.com', repo: 'a/b', dir: '', git_ref: 'v1', canonical: 'github:a/b@v1' }])
    const refs = adaptWasm().parseSourceRef('github:a/b@v1')
    expect(refs).toEqual([{ kind: 'github', host: 'github.com', repo: 'a/b', dir: '', gitRef: 'v1', canonical: 'github:a/b@v1' }])
    m.sourceUrls.mockReturnValueOnce({ resolve: null, index: 'u', files: [] })
    adaptWasm().sourceUrls((refs as never[])[0], ['p.hew'], 'c'.repeat(40))
    expect(m.sourceUrls.mock.calls[0][0]).toMatchObject({ git_ref: 'v1', canonical: 'github:a/b@v1' })
    expect(m.sourceUrls.mock.calls[0][2]).toBe('c'.repeat(40))
  })

  it('a non-wasm failure is still a value, never a throw', () => {
    m.parseSourceRef.mockImplementationOnce(() => {
      throw new Error('boom')
    })
    expect(adaptWasm().parseSourceRef('x')).toHaveProperty('error')
    m.commitFromResponse.mockImplementationOnce(() => {
      throw new Error('boom')
    })
    expect(adaptWasm().commitFromResponse('github', enc.encode('{}'))).toBeNull()
  })
})

describe('itemTools', () => {
  const summary = (attrs: Record<string, unknown> | undefined) => ({
    format_version: 1, objects: 1, materials: 0, components: 1, instances: 1, groups: 0, world_sketches: 0, annotations: 0, guides: 0,
    first_component_name: 'Chair', first_component_sid: '1', first_root_name: null,
    doc_attrs: attrs === undefined ? {} : { 'hew.library': attrs },
    material_entries: [], profile_chain_only: false,
  })

  it('readMeta reads id, name, category, collection, keywords and the note', async () => {
    const remote = { source: 'github:a/b', name: 'B', path: 'x.hew', sha256: 'f'.repeat(64), fetchedAt: '2026-11-23T00:00:00.000Z', edited: false }
    m.readItemSummary.mockResolvedValueOnce(summary({ id: 'i-1', name: 'Hinge', category: 'component', collection: 'Hinges', keywords: ['brass'], remote }))
    const meta = await itemTools().readMeta(new Uint8Array(3))
    expect(meta).toMatchObject({ id: 'i-1', name: 'Hinge', category: 'component', collection: 'Hinges', keywords: ['brass'] })
    expect(meta.remote?.source).toBe('github:a/b')
  })

  it('readMeta rejects a file with no hew.library id', async () => {
    m.readItemSummary.mockResolvedValueOnce(summary(undefined))
    await expect(itemTools().readMeta(new Uint8Array(3))).rejects.toThrow('not a Library item')
  })

  it('stamp writes only the keys the patch names; remote null removes the note', async () => {
    m.updateItemMeta.mockResolvedValue(new Uint8Array(1))
    await itemTools().stamp(new Uint8Array(1), { remote: null })
    expect(m.updateItemMeta.mock.calls[0][1]).toEqual({ remote: null })
    await itemTools().stamp(new Uint8Array(1), { collection: 'Shop/Hinges' })
    expect(m.updateItemMeta.mock.calls[1][1]).toEqual({ collection: 'Shop/Hinges' })
  })

  it('itemFileName mints the same path as the Library does', () => {
    expect(itemTools().itemFileName({ name: 'Butt hinge', id: '0b6f1c1e-5a2f', category: 'component' })).toBe('Components/butt-hinge-0b6f1c.hew')
  })
})

describe('realSourcesDeps', () => {
  it('wires the settings: profiles from the preview wall, auto-check from the setting, catalog from the env', async () => {
    const deps = realSourcesDeps()
    expect(deps.catalogUrl).toBe(DEFAULT_CATALOG_URL)
    expect(typeof deps.profilesEnabled()).toBe('boolean')
    localStorage.setItem('hew.settings.checkSources', 'false')
    expect(deps.autoCheckEnabled()).toBe(false)
    localStorage.removeItem('hew.settings.checkSources')
    expect(deps.autoCheckEnabled()).toBe(true)
    // Not a desktop page: the estimate reads the browser's own.
    expect(await deps.storageEstimate()).toBeNull() // jsdom/node has no navigator.storage
  })

  it('a test can replace any dependency', () => {
    const deps = realSourcesDeps({ catalogUrl: '' })
    expect(deps.catalogUrl).toBe('')
  })
})

describe('createDefaultBinding', () => {
  it('forwards Library and sources-file changes to one listener, and unsubscribes both', () => {
    const b = createDefaultBinding()
    const l = vi.fn()
    const off = b.subscribe(l)
    expect(m.sources).toHaveLength(1)
    expect(m.items).toHaveLength(1)
    m.sources[0]()
    m.items[0]()
    expect(l).toHaveBeenCalledTimes(2)
    off()
    expect(m.sources).toHaveLength(0)
    expect(m.items).toHaveLength(0)
  })

  it('reports availability from the fetcher and exposes the module', () => {
    const b = createDefaultBinding()
    expect(b.available()).toBe(true)
    expect(b.sources.catalogEnabled()).toBe(true)
  })
})
