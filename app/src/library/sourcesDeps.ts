/* oxlint-disable lingui/no-unlocalized-strings -- identifiers, URLs and storage keys; no user-visible text */
// The real dependencies of the sources logic (docs/design/shared-library-
// sources.md 7.4, package S11): the platform fetcher, the Library store, the
// wasm wrappers, the item tools, the source lock, the clock, and the three
// settings the module reads. `createSources` itself stays UI- and
// platform-free; this file is the one place that knows about them.
//
// The default binding is a lazy singleton so a window that never opens the
// Library never builds it.

import { libraryStore } from '../io/libraryStore'
import { isTauri } from '../io/fileHost'
import { withSourceLock } from '../io/sourceLock'
import { getCheckSources } from '../settings/checkSources'
import { behaviorEnabled } from '../preview'
import { loadKernel } from '../wasm/loader'
import { readItemSummary, sha256Hex, updateItemMeta } from './itemFiles'
import { itemFileName } from './fileNaming'
import { catalogUrlFor, DEFAULT_CATALOG_URL } from './catalogUrl'
import { buildLibraryItem } from './libraryModel'
import { remoteFetcher, type RemoteFetcher } from './remoteFetcher'
import {
  commitFromResponse as wasmCommit,
  parseIndex as wasmParseIndex,
  parseSourceRef as wasmParseSourceRef,
  sourceUrls as wasmSourceUrls,
  SourceWasmError,
  type SourceRef as WasmSourceRef,
} from './sourcesWasm'
import {
  createSources,
  type ItemFileMeta,
  type SourceIndex,
  type SourceRef,
  type Sources,
  type SourcesDeps,
  type SourceUrls,
  type StampPatch,
  type WasmError,
} from './sources'

const dec = new TextDecoder()

function asWasmError(e: unknown): WasmError {
  if (e instanceof SourceWasmError) return { error: { code: e.code, reason: e.reason } }
  return { error: { code: 'bad_input', reason: 'unreadable' } }
}

function toRef(w: WasmSourceRef): SourceRef {
  return { kind: w.kind, host: w.host, repo: w.repo, dir: w.dir, gitRef: w.git_ref, canonical: w.canonical }
}

function fromRef(r: SourceRef): WasmSourceRef {
  return { kind: r.kind, host: r.host, repo: r.repo, dir: r.dir, git_ref: r.gitRef, canonical: r.canonical }
}

/** The wasm exports in the shapes `createSources` wants (bytes in, errors
 * returned, camelCase refs). */
export function adaptWasm(): SourcesDeps['wasm'] {
  return {
    parseIndex(bytes: Uint8Array): SourceIndex | WasmError {
      try {
        return wasmParseIndex(dec.decode(bytes)) as unknown as SourceIndex
      } catch (e) {
        return asWasmError(e)
      }
    },
    parseSourceRef(text: string): SourceRef[] | WasmError {
      try {
        return wasmParseSourceRef(text).map(toRef)
      } catch (e) {
        return asWasmError(e)
      }
    },
    sourceUrls(ref: SourceRef, paths: string[], commit?: string): SourceUrls | WasmError {
      try {
        return wasmSourceUrls(fromRef(ref), paths, commit)
      } catch (e) {
        return asWasmError(e)
      }
    },
    commitFromResponse(kind, body) {
      try {
        return wasmCommit(kind, dec.decode(body))
      } catch {
        return null
      }
    },
  }
}

/** What the item tools of `createSources` do with the real item readers. */
export function itemTools(): SourcesDeps['items'] {
  return {
    async readMeta(bytes: Uint8Array): Promise<ItemFileMeta> {
      const summary = await readItemSummary(bytes)
      const item = buildLibraryItem({ fileName: '', size: bytes.length, mtimeMs: 0 }, summary, '')
      const id = item.meta.id
      if (id === undefined) throw new Error('not a Library item')
      return {
        id,
        name: item.meta.name ?? item.displayName,
        category: item.category,
        collection: item.meta.collection ?? '',
        keywords: item.meta.keywords ?? [],
        ...(item.meta.remote !== undefined ? { remote: item.meta.remote } : {}),
      }
    },
    stamp(bytes: Uint8Array, patch: StampPatch): Promise<Uint8Array> {
      const meta: Record<string, unknown> = {}
      if (patch.remote !== undefined) meta.remote = patch.remote
      if (patch.collection !== undefined) meta.collection = patch.collection
      return updateItemMeta(bytes, meta)
    },
    sha256Hex,
    itemFileName: (m) => itemFileName(m.name, m.id, m.category),
  }
}

/** `navigator.storage.estimate()` on the web; null on the desktop and where
 * the browser has no estimate (there is nothing to check against). */
async function storageEstimate(): Promise<{ usage: number; quota: number } | null> {
  if (isTauri) return null
  try {
    const est = await navigator.storage?.estimate?.()
    if (est === undefined || est.quota === undefined) return null
    return { usage: est.usage ?? 0, quota: est.quota }
  } catch {
    return null
  }
}

/** The pieces the Library dialog works with. */
export interface SourcesBinding {
  sources: Sources
  /** False only in the Off state (web page whose policy forbids outside
   * requests): the dialog then shows one line and makes no request. */
  available(): boolean
  /** Called when the sources file or the Library changes (any window). */
  subscribe(listener: () => void): () => void
}

/** Real dependencies for `createSources`, each replaceable by a test. */
export function realSourcesDeps(over: Partial<SourcesDeps> = {}, fetcher: RemoteFetcher = remoteFetcher()): SourcesDeps {
  return {
    fetcher,
    store: libraryStore(),
    wasm: adaptWasm(),
    items: itemTools(),
    withSourceLock,
    now: () => new Date(),
    catalogUrl: catalogUrlFor(import.meta.env as { VITE_HEW_CATALOG_URL?: string }),
    profilesEnabled: () => behaviorEnabled('fillet-profiles'),
    autoCheckEnabled: getCheckSources,
    storageEstimate,
    ...over,
  }
}

let ready: Promise<void> | null = null
function kernelReady(): Promise<void> {
  if (ready === null) ready = loadKernel().then(() => undefined)
  return ready
}

/** The sources module over the real dependencies. Every call waits for the
 * wasm module first (the source exports need it initialized). */
export function createDefaultBinding(): SourcesBinding {
  const fetcher = remoteFetcher()
  const real = createSources(realSourcesDeps({}, fetcher))
  const sources = {} as Record<string, unknown>
  for (const key of Object.keys(real) as Array<keyof Sources>) {
    const fn = real[key] as unknown as (...a: unknown[]) => unknown
    sources[key] = key === 'catalogEnabled' ? fn : async (...a: unknown[]) => (await kernelReady(), fn(...a))
  }
  return {
    sources: sources as unknown as Sources,
    available: () => fetcher.available(),
    subscribe(listener) {
      const store = libraryStore()
      const offSources = store.subscribeSources(listener)
      const offItems = store.subscribe(listener)
      return () => {
        offSources()
        offItems()
      }
    },
  }
}

let defaultBinding: SourcesBinding | null = null

/** The one binding the app's Library windows share; built on first use. */
export function defaultSourcesBinding(): SourcesBinding {
  if (defaultBinding === null) defaultBinding = createDefaultBinding()
  return defaultBinding
}

export { catalogUrlFor, DEFAULT_CATALOG_URL }
