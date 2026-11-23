/* oxlint-disable lingui/no-unlocalized-strings -- identifiers, codes and cache keys; no user-visible text */
// Sources logic (docs/design/shared-library-sources.md 7.4): everything a
// Library source does that is not UI: the types and signatures, then the
// implementation (package S10).
//
// Contract notes the implementer must honor (the tests in sources*.test.ts
// are the executable form):
//
// - The module takes NO logger and never puts a URL in a returned value, a
//   thrown error or a console call (decision 28). Expected failures are
//   RETURNED as `{ ok: false, code, ... }`, never thrown. A fetcher that
//   rejects with anything other than a RemoteFetchError is reported as code
//   'network'.
// - Every network request goes through `deps.fetcher.fetchBytes(url, maxBytes)`
//   with the maxBytes of design 6.4: 4 MiB index / catalog / snapshot, 1 MiB
//   thumbnail, 64 KiB resolve response, the entry's `bytes` for an item file.
//   URLs come only from `deps.wasm.sourceUrls` (or `deps.catalogUrl`).
// - `.sources.json` is written only through `store.writeSources(source, entry)`
//   (one source's whole entry, or null to remove it); never as a whole file.
// - An item reaches the store with exactly ONE `store.write` (plus, on an
//   update whose minted path changed, one `store.remove` of the old file after
//   the new one is written). Stamping happens in memory before the write, so
//   any failure before the write leaves the Library untouched.
// - When `deps.fetcher.available()` is false (Off), every method that would
//   make a request resolves `{ off: true }` without making one.
// - Jobs on a source (check, addAll, follow, updateAll) run inside
//   `deps.withSourceLock(canonicalRef, ...)`; `{ ran: false }` yields
//   `{ ok: false, code: 'busy' }`. Single-item add/update take no lock (they
//   run inside addAll/updateAll's lock).
// - Cache keys given to `store.readSourceCache` / `writeSourceCache` obey the
//   `valid_source_cache_key` charset (1-96 of [a-z0-9_-], first char a letter
//   or digit). The index cache must survive a restart: a new `createSources`
//   over the same store reads the last good index, the catalog and the
//   catalog's last-fetch time back from the store.

import type { LibraryStore } from '../io/libraryStore'
import type { SourceLockResult } from '../io/sourceLock'
import { RemoteFetchError, type RemoteFetcher, type RemoteFetchCode } from './remoteFetcher'
import type { LibraryCategory, LibraryRemoteNote } from './types'

// ---------------------------------------------------------------- wasm shapes

export type SourceKind = 'github' | 'gitlab' | 'forgejo' | 'web'

/** Design 6.4 `SourceRef`, camelCased. `gitRef` is the `@<ref>` the user or
 * the list gave (null when none). */
export interface SourceRef {
  kind: SourceKind
  host: string
  repo: string
  dir: string
  gitRef: string | null
  canonical: string
}

/** The categories exactly as an index spells them (6.1). */
export type IndexCategory = 'Components' | 'Materials' | 'Models' | 'Profiles'

/** One entry of an index (6.1). */
export interface IndexItem {
  id: string
  path: string
  name: string
  category: IndexCategory
  keywords?: string[]
  collection?: string | null
  sha256: string
  bytes: number
  thumbnail?: string | null
  license?: string | null
}

/** A parsed index (6.1, the shape `source_parse_index` returns). */
export interface SourceIndex {
  hew: number
  name: string
  description?: string | null
  author?: string | null
  homepage?: string | null
  license?: string | null
  licenseUrl?: string | null
  items: IndexItem[]
}

/** The wasm layer's failure shape. `code` for parseIndex is one of
 * `index_too_new | index_too_large | index_invalid`. */
export interface WasmError {
  error: { code: string; reason: string }
}

/** `source_urls` result: the ref-to-commit request (null for `web`), the index
 * URL, and one URL per requested path, in order. */
export interface SourceUrls {
  resolve: string | null
  index: string
  files: string[]
}

// ------------------------------------------------------------------ item deps

/** What the app reads out of an item file's own `hew.library` (design 2,
 * decision 15). `category` is the app's `LibraryCategory` (lower-case), NOT the
 * index spelling; the module maps `Components`->`component`, `Materials`->
 * `material`, `Models`->`model`, `Profiles`->`profile`. */
export interface ItemFileMeta {
  id: string
  name: string
  category: LibraryCategory
  /** '' when the item has none. */
  collection: string
  keywords: string[]
  remote?: LibraryRemoteNote
}

/** What `items.stamp` writes into the file's `hew.library`. `remote: null`
 * removes the note. `collection` replaces the collection. Absent key = leave
 * alone. */
export interface StampPatch {
  remote?: LibraryRemoteNote | null
  collection?: string
}

// ----------------------------------------------------------------------- deps

export type LibraryStoreSubset = Pick<
  LibraryStore,
  | 'list'
  | 'read'
  | 'write'
  | 'remove'
  | 'readSources'
  | 'writeSources'
  | 'readSourceCache'
  | 'writeSourceCache'
  | 'removeSourceCache'
  | 'editingNames'
>

export interface SourcesDeps {
  /** The one door to the network. */
  fetcher: RemoteFetcher
  store: LibraryStoreSubset
  wasm: {
    /** `source_parse_index`: parse and validate index bytes (6.1). */
    parseIndex(bytes: Uint8Array): SourceIndex | WasmError
    /** `source_parse_ref`: the candidates in probe order (one unless the host
     * is unknown). An error means the text is not a reference. */
    parseSourceRef(text: string): SourceRef[] | WasmError
    /** `source_urls`: with no `commit` the URLs read at the ref. */
    sourceUrls(ref: SourceRef, paths: string[], commit?: string): SourceUrls | WasmError
    /** `commit_from_response` (6.4 crate fn; the module needs it, though it is
     * not among the three wasm exports of 6.4 - see report question 1). */
    commitFromResponse(kind: SourceKind, body: Uint8Array): string | null
  }
  items: {
    /** Reads an item file's own `hew.library`. Rejects for a file that is not
     * a Library item. */
    readMeta(bytes: Uint8Array): Promise<ItemFileMeta>
    /** The bytes with the patch applied to `hew.library` (real tool:
     * `updateItemMeta`). Pure: the input is not modified. */
    stamp(bytes: Uint8Array, patch: StampPatch): Promise<Uint8Array>
    sha256Hex(bytes: Uint8Array): Promise<string>
    /** Real tool: `itemFileName(name, id, category)`. */
    itemFileName(meta: { name: string; id: string; category: LibraryCategory }): string
  }
  withSourceLock<T>(sourceRef: string, fn: () => Promise<T>): Promise<SourceLockResult<T>>
  now(): Date
  /** The catalog.json URL; '' hides the catalog (6.3). */
  catalogUrl: string
  profilesEnabled(): boolean
  /** The "Check the Library for newer parts" setting. */
  autoCheckEnabled(): boolean
  /** `navigator.storage.estimate()` on the web; null where there is nothing to
   * check (desktop, or an unsupported browser). */
  storageEstimate(): Promise<{ usage: number; quota: number } | null>
}

// -------------------------------------------------------------------- results

export interface Off {
  off: true
}

export type SourceErrorCode =
  | RemoteFetchCode
  | 'ref_invalid' // the text is not a source reference
  | 'index_invalid'
  | 'index_too_new'
  | 'index_too_large'
  | 'no_source' // every candidate failed ("This address does not have a Hew source")
  | 'resolve_invalid' // the ref-to-commit request answered, but with no usable commit
  | 'unknown_source' // the source is not in .sources.json
  | 'item_not_found' // the id is not in the source's index
  | 'not_checked' // no cached index to work from
  | 'busy' // another tab holds the source's lock
  | 'source_removed'
  | 'hash_mismatch'
  | 'listing_incorrect'
  | 'file_invalid' // the fetched file is not a Library item
  | 'name_clash'
  | 'unavailable_other_source'
  | 'needs_confirm'
  | 'already_in_library'
  | 'not_updatable'
  | 'not_linked'
  | 'not_found' // no such item in the Library
  | 'open_for_editing'
  | 'not_enough_space'
  | 'write_failed'
  | 'catalog_disabled'
  | 'catalog_invalid'

export interface Failure {
  ok: false
  code: SourceErrorCode
  /** HTTP status, for code `http_status`. */
  status?: number
  /** `needs_confirm`: which question the UI asks. */
  confirm?: 'replace_item' | 'replace_changes'
  /** `source_removed`: the catalog's reason and optional link. */
  reason?: string
  link?: string
  /** `not_enough_space`: bytes the run needs and bytes the browser has free. */
  needed?: number
  free?: number
  /** `unavailable_other_source`: the name in the note. `not_updatable`: why. */
  otherSource?: string
  state?: string
  /** `no_source`: each candidate's code, in probe order. */
  causes?: SourceErrorCode[]
}

/** Design 5.1, the per-item state names, as the source view shows them. */
export type ItemState =
  | 'not_in_library' // Not in your Library
  | 'in_library' // In your Library (no note)
  | 'in_library_from_other' // In your Library from <that source>
  | 'in_step' // R-in-step, shown as "In your Library"
  | 'newer' // R-newer
  | 'gone' // R-gone
  | 'orphan' // R-orphan

/** `.sources.json` entry for one source (6.5) plus the two fields this module
 * adds (report question 2). */
export interface SourceEntry {
  source: string
  kind: SourceKind
  name: string
  followed: boolean
  addedAt: string
  lastCheck: { at: string; ok: boolean; code: string | null; status?: number } | null
  skipped: string[]
  /** id -> the sha256 the user declined. */
  kept: Record<string, string>
  /** ISO time of the last successful check (the cached listing's age). */
  lastGoodAt?: string | null
  /** The commit the cached index was read at (git kinds). */
  commit?: string | null
}

export interface SourceStatus {
  entry: SourceEntry
  followed: boolean
  /** S-unreachable: the last check failed. */
  unreachable: boolean
  /** S-removed, with the catalog's reason. */
  removed: { reason: string; severity: 'removed' | 'critical'; link?: string } | null
}

export interface IndexSummary {
  name: string
  description: string | null
  author: string | null
  homepage: string | null
  license: string | null
  licenseUrl: string | null
  itemCount: number
  totalBytes: number
  categories: Record<string, number>
}

export interface AddSourceOk {
  ok: true
  /** True when the reference was already in the list: nothing changed. */
  existing: boolean
  entry: SourceEntry
  summary: IndexSummary
  /** The host of the reference, for the Add dialog. */
  host: string
  /** 'unknown' when no catalog could be read. */
  listing: 'listed' | 'unlisted' | 'unknown'
  /** An added source with no cached listing while the check setting is off:
   * nothing was fetched (decision 27), `summary` holds only the stored name
   * and zero counts, and the UI offers Refresh. Absent otherwise. */
  needsCheck?: true
}

export interface PeekOk {
  ok: true
  /** True when the reference is already in the list. */
  existing: boolean
  /** As `AddSourceOk.needsCheck`. */
  needsCheck?: true
  /** The canonical source string `addSource` would store. */
  source: string
  summary: IndexSummary
  host: string
  listing: 'listed' | 'unlisted' | 'unknown'
}

export interface ItemRow {
  id: string
  name: string
  category: IndexCategory
  state: ItemState
  /** R-edited: "changed here". Only on rows that have a local note naming this source. */
  edited: boolean
  /** Kept: the user declined this very sha256. */
  kept: boolean
  /** On this source's skip list. */
  skipped: boolean
  /** The index entry; null for `gone`. */
  entry: IndexItem | null
  /** The local file, when the item is in the Library. */
  file: string | null
  /** `in_library_from_other`: the name in the other note. */
  from?: string
}

export interface ItemStatesOk {
  ok: true
  rows: ItemRow[]
  /** ISO time of the last good check. */
  lastGoodAt: string | null
  commit?: string
}

export interface NewCounts {
  new: number
  changed: number
}

export interface CheckOk {
  ok: true
  entry: SourceEntry
  commit?: string
  rows: ItemRow[]
  counts: NewCounts
}

export interface CheckFailed extends Failure {
  /** The last good listing, when there is one cached. */
  cached?: { rows: ItemRow[]; lastGoodAt: string | null }
}

export interface LocalState {
  id: string
  file: string
  name: string
  note: LibraryRemoteNote
  state: 'in_step' | 'newer' | 'gone' | 'orphan' | 'unchecked'
  edited: boolean
  kept: boolean
}

export interface CatalogSourceEntry {
  source: string
  kind: SourceKind
  commit?: string
  name: string
  description?: string
  author?: string
  homepage?: string
  license?: string
  licenseUrl?: string | null
  items: number
  bytes: number
  categories: Record<string, number>
  stale: boolean
  snapshot: string
}

export interface RemovedEntry {
  source: string
  severity: 'removed' | 'critical'
  reason: string
  link?: string
}

export interface CatalogData {
  hew: number
  built: string
  sources: CatalogSourceEntry[]
  removed: RemovedEntry[]
}

export interface CatalogOk {
  ok: true
  catalog: CatalogData
  /** The ADDED sources the removed list matched (canonical-with-ref-set-aside). */
  removedSources: Array<{ source: string; severity: 'removed' | 'critical'; reason: string; link?: string }>
}

export interface CatalogFailed extends Failure {
  /** The last good catalog, when cached. */
  cached: CatalogData | null
}

export interface AddItemOk {
  ok: true
  file: string
  id: string
  /** True when an existing item was replaced at the same path. */
  replaced: boolean
  sha256: string
  /** Update with a changed minted path: the old file could not be removed. */
  removeFailed?: boolean
  /** updateItem with decision 'keep': nothing was fetched or written. */
  kept?: boolean
}

export interface CatalogPreviewOk {
  ok: true
  /** The snapshot's items with the 5.1 state computed against the Library. */
  rows: ItemRow[]
  summary: IndexSummary
  /** True when the live fetch failed and this is the cached copy. */
  stale: boolean
}

export type AddItemResult = Off | AddItemOk | Failure

export interface RunProgress {
  /** Items finished so far (added or failed). */
  done: number
  total: number
  added: number
  failed: number
  /** The id just finished. */
  id?: string
}

export interface RunOptions {
  onProgress?: (p: RunProgress) => void
  /** Abort = Cancel: keeps what has landed, starts nothing more. */
  signal?: AbortSignal
  /** Retry: only these ids. */
  ids?: string[]
}

export interface RunSummary {
  added: number
  failed: number
  /** Ids of the failed items (the Retry list). */
  retry: string[]
  failures: Array<{ id: string; code: SourceErrorCode }>
  /** Ids never attempted because the run was cancelled or stopped. */
  notTried: string[]
  cancelled: boolean
  stopped: null | 'network' | 'rate_limited'
  /** Profiles left out because the profile preview is off. */
  profilesSkipped: number
}

export type RunResult = Off | { ok: true; summary: RunSummary } | Failure

export interface UpdateRow {
  id: string
  result: AddItemResult
}

export type UpdateAllResult =
  | Off
  | {
      ok: true
      rows: UpdateRow[]
      /** R-newer rows left for Review: edited here. */
      needsReview: string[]
    }
  | Failure

export type ThumbnailResult = Off | { ok: true; bytes: Uint8Array | null } | Failure

export interface UpdateOptions {
  /** Answer to a `needs_confirm` 'replace_changes'. */
  decision?: 'replace' | 'keep'
}

export interface AddItemOptions {
  /** Answer to a `needs_confirm` 'replace_item': Yes. */
  replace?: boolean
}

export interface Sources {
  /** True when a catalog URL is configured. */
  catalogEnabled(): boolean

  // --- network
  addSource(text: string): Promise<Off | AddSourceOk | Failure>
  /** What `addSource` would find, read-only: no `.sources.json` write, no
   * cache write (not the index, not the catalog). For the Add dialog's "Look up". */
  peekSource(text: string): Promise<Off | PeekOk | Failure>
  check(source: string): Promise<Off | CheckOk | CheckFailed>
  catalog(): Promise<Off | CatalogOk | CatalogFailed>
  /** The catalog entry's `snapshot` (a path relative to the catalog URL's
   * folder; never derived from a slug), fetched with the 4 MiB cap, cached via
   * writeSourceCache, parsed with wasm.parseIndex. A failed fetch falls back
   * to the cached copy (`stale: true`); with none, the failure. Off -> off:true
   * even when a copy is cached. `catalog_disabled` when the catalog URL is ''. */
  catalogPreview(entry: CatalogSourceEntry): Promise<Off | CatalogPreviewOk | Failure>
  addItem(source: string, id: string, opts?: AddItemOptions): Promise<AddItemResult>
  updateItem(source: string, id: string, opts?: UpdateOptions): Promise<AddItemResult>
  addAll(source: string, opts?: RunOptions): Promise<RunResult>
  /** Sets `followed`, then runs addAll. */
  follow(source: string, opts?: RunOptions): Promise<RunResult>
  updateAll(source: string): Promise<UpdateAllResult>
  thumbnail(source: string, id: string): Promise<ThumbnailResult>
  /** Runs every due check and, under the same rule, the catalog fetch. */
  autoCheck(): Promise<Off | { checked: string[]; catalogFetched: boolean }>

  // --- no network
  listSources(): Promise<SourceStatus[]>
  removeSource(source: string): Promise<{ ok: true } | Failure>
  unfollow(source: string): Promise<{ ok: true } | Failure>
  itemStates(source: string): Promise<ItemStatesOk | Failure>
  /** Every local item that carries a remote note, against its source's cached index. */
  localStates(): Promise<LocalState[]>
  newCounts(source: string): Promise<NewCounts>
  keepMine(source: string, id: string): Promise<{ ok: true } | Failure>
  unlink(source: string, id: string): Promise<{ ok: true } | Failure>
  /** The UI deleted an item: `remote` is the note it carried, read before the delete. */
  noteDeleted(id: string, remote: { source: string } | undefined): Promise<void>
  shouldAutoCheck(source: string): Promise<boolean>
  shouldAutoCheckCatalog(): Promise<boolean>
}

// ------------------------------------------------------------ implementation

const enc = new TextEncoder()
const dec = new TextDecoder()

const MiB = 1024 * 1024
const MAX_RESOLVE = 64 * 1024
const MAX_INDEX = 4 * MiB
const MAX_THUMB = 1 * MiB
const CANDIDATE_MS = 10_000
const DUE_MS = 6 * 3600_000
const NETWORK_RUN_LIMIT = 3
const CATALOG_KEY = 'catalog'
/** The cache entry listing cache keys whose delete failed (see `sweep`). */
const PENDING_KEY = 'pending-deletes'
/** At most this many pending keys are kept; the oldest go first. */
const PENDING_MAX = 512
const CACHE_KEY_RE = /^[a-z0-9][a-z0-9_-]{0,95}$/

const LIB_CATEGORY: Record<IndexCategory, LibraryCategory> = {
  Components: 'component',
  Materials: 'material',
  Models: 'model',
  Profiles: 'profile',
}
const INDEX_CATEGORY: Record<LibraryCategory, IndexCategory> = {
  component: 'Components',
  material: 'Materials',
  model: 'Models',
  profile: 'Profiles',
}

const fail = (code: SourceErrorCode, extra: Partial<Failure> = {}): Failure => ({ ok: false, code, ...extra })

const isFailure = (x: unknown): x is Failure => typeof x === 'object' && x !== null && (x as { ok?: unknown }).ok === false

/** The canonical reference with any `@<ref>` set aside (6.2 removed-list match). */
function withoutRef(wasm: SourcesDeps['wasm'], canonical: string): string {
  const refs = wasm.parseSourceRef(canonical)
  if ('error' in refs || refs.length === 0) return canonical
  const gitRef = refs[0].gitRef
  return gitRef !== null && canonical.endsWith(`@${gitRef}`) ? canonical.slice(0, -(gitRef.length + 1)) : canonical
}

interface LibRec {
  file: string
  meta: ItemFileMeta
}

/** A file name as a case-insensitive disk sees it. */
const fileKey = (file: string): string => file.toLowerCase()

/** The Library's items as a run sees them: by id and by file, kept current as
 * the run writes. `byFile` is keyed case-insensitively (`fileKey`), so a
 * differently-cased name finds the file a case-insensitive disk would
 * overwrite. `unreadable` holds the keys of listed files whose `hew.library`
 * could not be read (damaged, newer format, no id): they are in the Library
 * but in no record, and nothing may be written over them. */
class LibIndex {
  byId = new Map<string, LibRec>()
  byFile = new Map<string, LibRec>()
  constructor(
    public recs: LibRec[],
    public unreadable: Set<string> = new Set(),
  ) {
    for (const r of recs) this.put(r)
  }
  /** `fresh` marks a record the run just wrote: it becomes the id's record. */
  put(r: LibRec, fresh = false): void {
    if (fresh || !this.byId.has(r.meta.id)) this.byId.set(r.meta.id, r)
    this.byFile.set(fileKey(r.file), r)
  }
  drop(file: string): void {
    const r = this.byFile.get(fileKey(file))
    this.byFile.delete(fileKey(file))
    if (r !== undefined && this.byId.get(r.meta.id) === r) this.byId.delete(r.meta.id)
  }
}

interface Ctx {
  entry: SourceEntry
  ref: SourceRef
  index: SourceIndex
}

interface Fetched {
  ok: true
  bytes: Uint8Array
  index: SourceIndex
  commit?: string
}

interface CheckCore {
  ok: true
  entry: SourceEntry
  index: SourceIndex
  commit?: string
}

interface CachedCatalog {
  at: string
  data: CatalogData
}

/** An entry written before `status` existed has none; a stray non-number is dropped. */
function normalizeLastCheck(c: NonNullable<SourceEntry['lastCheck']>): NonNullable<SourceEntry['lastCheck']> {
  const { status, ...rest } = c
  return typeof status === 'number' ? { ...rest, status } : rest
}

function normalizeEntry(raw: unknown): SourceEntry | null {
  if (typeof raw !== 'object' || raw === null) return null
  const o = raw as Record<string, unknown>
  if (typeof o.source !== 'string' || typeof o.kind !== 'string') return null
  const lastCheck = o.lastCheck as SourceEntry['lastCheck'] | undefined
  return {
    ...(o as unknown as SourceEntry),
    name: typeof o.name === 'string' ? o.name : o.source,
    followed: o.followed === true,
    addedAt: typeof o.addedAt === 'string' ? o.addedAt : '',
    lastCheck: lastCheck && typeof lastCheck === 'object' ? normalizeLastCheck(lastCheck) : null,
    skipped: Array.isArray(o.skipped) ? (o.skipped as string[]) : [],
    kept: o.kept && typeof o.kept === 'object' ? (o.kept as Record<string, string>) : {},
  }
}

/** The 6.1 relative-path rule, as Rust `check_rel_path_with_suffix`
 * (crates/sources/src/paths.rs): non-empty, at most 1,024 bytes, no control
 * character, backslash, `:`, `?` or `#` (so no scheme), no empty or
 * dot-leading segment (so no leading `/` and no `..`), no percent escape of
 * `.`, `/` or `\`, and the given suffix. */
function isSafeRelPath(p: string, suffix: string): boolean {
  if (p === '' || enc.encode(p).length > 1024 || !p.endsWith(suffix)) return false
  for (const ch of p) {
    const c = ch.codePointAt(0) ?? 0
    if (c <= 0x1f || (c >= 0x7f && c <= 0x9f) || ch === '\\' || ch === ':' || ch === '?' || ch === '#') return false
  }
  if (p.split('/').some((seg) => seg === '' || seg.startsWith('.'))) return false
  return !/%(2e|2f|5c)/i.test(p)
}

function isHttpsUrl(s: string): boolean {
  try {
    const u = new URL(s)
    return u.protocol === 'https:' && u.hostname !== ''
  } catch {
    return false
  }
}

const COMMIT_HEX = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/
const KINDS: ReadonlySet<string> = new Set<SourceKind>(['github', 'gitlab', 'forgejo', 'web'])

/** One `sources` entry checked as Rust `parse_catalog_source` checks the
 * fields the app relies on: `source` canonical, a known `kind`, `name` a
 * string, `commit` 40/64 lowercase hex and absent on `web`, `snapshot` a safe
 * relative `.json` path, `homepage`/`licenseUrl` https when present. */
function validCatalogSource(wasm: SourcesDeps['wasm'], s: unknown): s is CatalogSourceEntry {
  if (typeof s !== 'object' || s === null) return false
  const x = s as Record<string, unknown>
  if (typeof x.source !== 'string' || typeof x.kind !== 'string' || !KINDS.has(x.kind) || typeof x.name !== 'string') return false
  const refs = wasm.parseSourceRef(x.source)
  if ('error' in refs || refs.length !== 1 || refs[0].canonical !== x.source) return false
  if (x.commit !== undefined && x.commit !== null && (typeof x.commit !== 'string' || !COMMIT_HEX.test(x.commit) || x.kind === 'web')) return false
  if (typeof x.snapshot !== 'string' || !isSafeRelPath(x.snapshot, '.json')) return false
  for (const k of ['homepage', 'licenseUrl'] as const) {
    const v = x[k]
    if (v !== undefined && v !== null && (typeof v !== 'string' || !isHttpsUrl(v))) return false
  }
  return true
}

/** A catalog as the app keeps it. Rust `parse_catalog` refuses a whole
 * catalog over one bad entry; here a bad `sources` entry is DROPPED instead
 * (the rest of the catalog stays usable, and a dropped entry is only a
 * source the view does not list), nothing is logged. A `removed` entry is
 * kept whenever it names a source and a reason, since dropping one would
 * quietly un-remove a source; only a `link` that is not https is dropped
 * from it. */
function normalizeCatalog(raw: unknown, wasm: SourcesDeps['wasm']): CatalogData | null {
  if (typeof raw !== 'object' || raw === null) return null
  const o = raw as Record<string, unknown>
  if (typeof o.hew !== 'number' || !Array.isArray(o.sources)) return null
  const sources = (o.sources as unknown[]).filter((s): s is CatalogSourceEntry => validCatalogSource(wasm, s))
  const removed: RemovedEntry[] = []
  if (Array.isArray(o.removed)) {
    for (const r of o.removed as unknown[]) {
      if (typeof r !== 'object' || r === null) continue
      const x = r as Record<string, unknown>
      if (typeof x.source !== 'string' || typeof x.reason !== 'string') continue
      removed.push({
        source: x.source,
        severity: x.severity === 'critical' ? 'critical' : 'removed',
        reason: x.reason,
        ...(typeof x.link === 'string' && isHttpsUrl(x.link) ? { link: x.link } : {}),
      })
    }
  }
  return { hew: o.hew, built: typeof o.built === 'string' ? o.built : '', sources, removed }
}

function summarize(index: SourceIndex): IndexSummary {
  const categories: Record<string, number> = {}
  let totalBytes = 0
  for (const it of index.items) {
    categories[it.category] = (categories[it.category] ?? 0) + 1
    totalBytes += it.bytes
  }
  return {
    name: index.name,
    description: index.description ?? null,
    author: index.author ?? null,
    homepage: index.homepage ?? null,
    license: index.license ?? null,
    licenseUrl: index.licenseUrl ?? null,
    itemCount: index.items.length,
    totalBytes,
    categories,
  }
}

/** The summary of an added source whose listing is not cached: its stored
 * name and nothing else (`needsCheck`). */
function emptySummary(entry: SourceEntry): IndexSummary {
  return { name: entry.name, description: null, author: null, homepage: null, license: null, licenseUrl: null, itemCount: 0, totalBytes: 0, categories: {} }
}

/** Bookkeeping a state row needs from a source entry. */
interface RowSource {
  source: string
  skipped: string[]
  kept: Record<string, string>
}

export function createSources(deps: SourcesDeps): Sources {
  const { fetcher, store, wasm, items } = deps
  const nowIso = (): string => deps.now().toISOString()
  const off = (): Off => ({ off: true })

  // ------------------------------------------------------------- network

  async function fetchRaw(url: string, maxBytes: number): Promise<{ ok: true; bytes: Uint8Array } | Failure> {
    try {
      return { ok: true, bytes: await fetcher.fetchBytes(url, maxBytes) }
    } catch (e) {
      if (e instanceof RemoteFetchError) {
        return e.code === 'http_status' && e.status !== undefined ? fail(e.code, { status: e.status }) : fail(e.code)
      }
      return fail('network')
    }
  }

  function withTimeout<T>(p: Promise<T>, ms: number): Promise<T | Failure> {
    return new Promise((resolve) => {
      const timer = setTimeout(() => resolve(fail('timeout')), ms)
      p.then(
        (v) => {
          clearTimeout(timer)
          resolve(v)
        },
        () => {
          clearTimeout(timer)
          resolve(fail('network'))
        },
      )
    })
  }

  function parseIdx(bytes: Uint8Array): { ok: true; index: SourceIndex } | Failure {
    const r = wasm.parseIndex(bytes)
    if ('error' in r) {
      const code = r.error.code
      return fail(code === 'index_too_new' || code === 'index_too_large' ? code : 'index_invalid')
    }
    return { ok: true, index: r }
  }

  /** Resolves the ref to a commit (git kinds). A rate-limited answer falls
   * back to reading at the ref (no commit), per 6.2: the only unpinned
   * read. Any other failure, a 404 included, is an error on every kind,
   * Forgejo too, and so is an answer that names no usable commit
   * (`resolve_invalid`). */
  async function resolveCommit(ref: SourceRef): Promise<{ ok: true; commit?: string } | Failure> {
    const urls = wasm.sourceUrls(ref, [])
    if ('error' in urls) return fail('ref_invalid')
    if (urls.resolve === null) return { ok: true }
    const r = await fetchRaw(urls.resolve, MAX_RESOLVE)
    if (!r.ok) {
      return r.code === 'rate_limited' ? { ok: true } : r
    }
    const commit = wasm.commitFromResponse(ref.kind, r.bytes)
    return commit === null ? fail('resolve_invalid') : { ok: true, commit }
  }

  async function fetchIndex(ref: SourceRef): Promise<Fetched | Failure> {
    const resolved = await resolveCommit(ref)
    if (!resolved.ok) return resolved
    const urls = wasm.sourceUrls(ref, [], resolved.commit)
    if ('error' in urls) return fail('ref_invalid')
    const got = await fetchRaw(urls.index, MAX_INDEX)
    if (!got.ok) return got
    const parsed = parseIdx(got.bytes)
    if (!parsed.ok) return parsed
    return { ok: true, bytes: got.bytes, index: parsed.index, ...(resolved.commit !== undefined ? { commit: resolved.commit } : {}) }
  }

  // --------------------------------------------------------------- store

  const hashes = new Map<string, string>()
  async function keyFor(prefix: string, text: string): Promise<string> {
    let h = hashes.get(text)
    if (h === undefined) {
      h = await items.sha256Hex(enc.encode(text))
      hashes.set(text, h)
    }
    return `${prefix}-${h.slice(0, 24)}`
  }

  async function readCache(key: string): Promise<Uint8Array | null> {
    try {
      return await store.readSourceCache(key)
    } catch {
      return null
    }
  }

  async function writeCache(key: string, bytes: Uint8Array): Promise<void> {
    try {
      await store.writeSourceCache(key, bytes)
    } catch {
      // The cache is only a convenience; a failed write loses offline listings.
    }
  }

  /** The cache keys of every thumbnail `index` references for `source`. */
  async function thumbKeysOf(source: string, index: SourceIndex): Promise<Set<string>> {
    const keys = new Set<string>()
    for (const item of index.items) {
      if (item.thumbnail) keys.add(await keyFor('thumb', `${source}\n${item.thumbnail}`))
    }
    return keys
  }

  // A delete that fails is remembered and retried. No store method lists the
  // cache, so a key that has dropped out of every index would otherwise never
  // be a sweep candidate again. The keys whose delete failed are kept as a JSON
  // array in one cache entry of their own, and every sweep retries them. A
  // retried key that is live again (its source re-added, its thumbnail listed
  // again) is kept and leaves the list.
  async function readPending(): Promise<string[]> {
    const bytes = await readCache(PENDING_KEY)
    if (bytes === null) return []
    try {
      const j: unknown = JSON.parse(dec.decode(bytes))
      return Array.isArray(j) ? j.filter((k): k is string => typeof k === 'string' && CACHE_KEY_RE.test(k) && k !== PENDING_KEY) : []
    } catch {
      return []
    }
  }

  /** Delete the cache entries in `candidates`, and every key left pending by
   * an earlier failed delete, that no listed source uses any more: the
   * thumbnails its cached index references, and its own `idx-*` and `snap-*`.
   * The catalog is never a candidate. A delete that fails goes on the pending
   * list for the next sweep. Runs inside `exclusive`. */
  async function sweep(candidates: Set<string>): Promise<void> {
    const pending = await readPending()
    if (candidates.size === 0 && pending.length === 0) return
    const live = new Set<string>()
    for (const e of await readEntries()) {
      live.add(await keyFor('idx', e.source))
      live.add(await keyFor('snap', e.source))
      const cached = await loadIndex(e.source)
      if (cached === null) continue
      for (const k of await thumbKeysOf(e.source, cached.index)) live.add(k)
    }
    const failed: string[] = []
    for (const k of new Set([...candidates, ...pending])) {
      if (live.has(k)) continue
      try {
        await store.removeSourceCache(k)
      } catch {
        failed.push(k)
      }
    }
    if (failed.length === 0 && pending.length === 0) return
    if (failed.length === pending.length && failed.every((k, i) => k === pending[i])) return
    if (failed.length > 0) {
      await writeCache(PENDING_KEY, enc.encode(JSON.stringify(failed.slice(-PENDING_MAX))))
      return
    }
    try {
      await store.removeSourceCache(PENDING_KEY)
    } catch {
      // A stale list is harmless: its keys are retried, and live ones are kept.
    }
  }

  async function readEntries(): Promise<SourceEntry[]> {
    let bytes: Uint8Array | null
    try {
      bytes = await store.readSources()
    } catch {
      return []
    }
    if (bytes === null) return []
    try {
      const j = JSON.parse(dec.decode(bytes)) as { sources?: unknown }
      if (!Array.isArray(j.sources)) return []
      return j.sources.map(normalizeEntry).filter((e): e is SourceEntry => e !== null)
    } catch {
      return []
    }
  }

  async function getEntry(source: string): Promise<SourceEntry | undefined> {
    return (await readEntries()).find((e) => e.source === source)
  }

  async function putEntry(entry: SourceEntry): Promise<void> {
    await store.writeSources(entry.source, entry as unknown as Record<string, unknown>)
  }

  // One write path for a source's state. Every read-then-write of an entry,
  // a removal, and every cache write that must not outlive its source or its
  // item (a check's index copy, a thumbnail) run here one at a time, so a
  // removal can never land between such a read and its write and be undone
  // by it. Each job re-reads the entry inside and writes nothing when it is
  // gone. Only short store work runs here, never a request, and nothing in
  // here calls `exclusive` again. It orders the jobs of this window only;
  // another tab's writes are not ordered against them.
  let tail: Promise<unknown> = Promise.resolve()
  function exclusive<T>(fn: () => Promise<T>): Promise<T> {
    const run = tail.then(fn)
    tail = run.then(
      () => undefined,
      () => undefined,
    )
    return run
  }

  /** `updateEntry` for a caller already inside `exclusive`. */
  async function updateEntryNow(source: string, change: (e: SourceEntry) => SourceEntry | null): Promise<SourceEntry | undefined> {
    const cur = await getEntry(source)
    if (cur === undefined) return undefined
    const next = change(cur)
    if (next === null) return cur
    await putEntry(next)
    return next
  }

  /** Re-reads the entry, applies `change` (return null for no change) and
   * writes only when something changed; nothing when the source is gone. */
  function updateEntry(source: string, change: (e: SourceEntry) => SourceEntry | null): Promise<SourceEntry | undefined> {
    return exclusive(() => updateEntryNow(source, change))
  }

  async function loadIndex(source: string): Promise<{ bytes: Uint8Array; index: SourceIndex } | null> {
    const bytes = await readCache(await keyFor('idx', source))
    if (bytes === null) return null
    const parsed = parseIdx(bytes)
    return parsed.ok ? { bytes, index: parsed.index } : null
  }

  async function loadCatalog(): Promise<CachedCatalog | null> {
    if (deps.catalogUrl === '') return null
    const bytes = await readCache(CATALOG_KEY)
    if (bytes === null) return null
    try {
      const j = JSON.parse(dec.decode(bytes)) as { at?: unknown; catalog?: unknown }
      const data = normalizeCatalog(j.catalog, wasm)
      return data === null || typeof j.at !== 'string' ? null : { at: j.at, data }
    } catch {
      return null
    }
  }

  async function removedFor(source: string): Promise<RemovedEntry | null> {
    const cat = await loadCatalog()
    if (cat === null || cat.data.removed.length === 0) return null
    const bare = withoutRef(wasm, source)
    return cat.data.removed.find((r) => withoutRef(wasm, r.source) === bare) ?? null
  }

  const removedFailure = (r: RemovedEntry): Failure => fail('source_removed', { reason: r.reason, ...(r.link !== undefined ? { link: r.link } : {}) })

  async function scanLibrary(): Promise<LibIndex> {
    const files = await store.list()
    const recs: LibRec[] = []
    const unreadable = new Set<string>()
    const BATCH = 8
    for (let i = 0; i < files.length; i += BATCH) {
      const got = await Promise.all(
        files.slice(i, i + BATCH).map(async (f): Promise<LibRec | string> => {
          try {
            const meta = await items.readMeta(await store.read(f.fileName))
            return { file: f.fileName, meta }
          } catch {
            return f.fileName
          }
        }),
      )
      for (const g of got) {
        if (typeof g === 'string') unreadable.add(fileKey(g))
        else recs.push(g)
      }
    }
    return new LibIndex(recs, unreadable)
  }

  // ---------------------------------------------------------- state rows

  function computeRows(src: RowSource, index: SourceIndex, lib: LibIndex, withGone: boolean): ItemRow[] {
    const profiles = deps.profilesEnabled()
    const skipped = new Set(src.skipped)
    const rows: ItemRow[] = []
    const inIndex = new Set<string>()
    for (const it of index.items) {
      inIndex.add(it.id)
      if (it.category === 'Profiles' && !profiles) continue
      const local = lib.byId.get(it.id)
      const note = local?.meta.remote
      let state: ItemState
      let from: string | undefined
      if (local === undefined) state = 'not_in_library'
      else if (note === undefined) state = 'in_library'
      else if (note.source !== src.source) {
        state = 'in_library_from_other'
        from = note.name
      } else state = note.sha256 === it.sha256 ? 'in_step' : 'newer'
      const mine = note !== undefined && note.source === src.source
      rows.push({
        id: it.id,
        name: it.name,
        category: it.category,
        state,
        edited: mine ? note.edited === true : false,
        kept: state === 'newer' && src.kept[it.id] === it.sha256,
        skipped: skipped.has(it.id),
        entry: it,
        file: local?.file ?? null,
        ...(from !== undefined ? { from } : {}),
      })
    }
    if (withGone) {
      for (const rec of lib.recs) {
        const note = rec.meta.remote
        if (note === undefined || note.source !== src.source || inIndex.has(rec.meta.id)) continue
        if (rec.meta.category === 'profile' && !profiles) continue
        rows.push({
          id: rec.meta.id,
          name: rec.meta.name,
          category: INDEX_CATEGORY[rec.meta.category],
          state: 'gone',
          edited: note.edited === true,
          kept: false,
          skipped: false,
          entry: null,
          file: rec.file,
        })
      }
    }
    return rows
  }

  function countsOf(entry: SourceEntry, rows: ItemRow[]): NewCounts {
    let n = 0
    let changed = 0
    for (const r of rows) {
      if (r.state === 'newer' && !r.kept) changed++
      if (entry.followed && r.state === 'not_in_library' && !r.skipped) n++
    }
    return { new: n, changed }
  }

  // --------------------------------------------------------------- checks

  async function runCheckCore(source: string): Promise<CheckCore | Failure> {
    const entry = await getEntry(source)
    if (entry === undefined) return fail('unknown_source')
    const refs = wasm.parseSourceRef(entry.source)
    if ('error' in refs || refs.length === 0) return fail('ref_invalid')
    const got = await fetchIndex(refs[0])
    const at = nowIso()
    // The fetch took time: the user may have deleted an item (skip list),
    // kept one, unfollowed, unlinked or removed the source meanwhile, none of
    // which takes the source lock. Re-read the entry now and change only the
    // fields a check owns; a source removed meanwhile stays removed.
    if (!got.ok) {
      await updateEntry(entry.source, (cur) => ({ ...cur, lastCheck: { at, ok: false, code: got.code, ...(got.code === 'http_status' && got.status !== undefined ? { status: got.status } : {}) } }))
      return got
    }
    // The index copy, the entry and the sweep are one job on the write path:
    // a removal lands wholly before it (nothing is written) or wholly after
    // it (and deletes what it wrote).
    const next = await exclusive(async (): Promise<SourceEntry | undefined> => {
      if ((await getEntry(entry.source)) === undefined) return undefined
      const before = await loadIndex(entry.source)
      await writeCache(await keyFor('idx', entry.source), got.bytes)
      const updated = await updateEntryNow(entry.source, (cur) => ({
        ...cur,
        name: got.index.name,
        lastCheck: { at, ok: true, code: null },
        lastGoodAt: at,
        commit: got.commit ?? null,
      }))
      // Also retries any delete an earlier sweep could not make.
      await sweep(before !== null ? await thumbKeysOf(entry.source, before.index) : new Set())
      return updated
    })
    if (next === undefined) return fail('unknown_source')
    return { ok: true, entry: next, index: got.index, ...(got.commit !== undefined ? { commit: got.commit } : {}) }
  }

  async function openCtx(source: string): Promise<{ ctx: Ctx } | Failure> {
    const entry = await getEntry(source)
    if (entry === undefined) return fail('unknown_source')
    const refs = wasm.parseSourceRef(entry.source)
    if ('error' in refs || refs.length === 0) return fail('ref_invalid')
    const cached = await loadIndex(entry.source)
    if (cached === null) return fail('not_checked')
    return { ctx: { entry, ref: refs[0], index: cached.index } }
  }

  async function check(source: string): Promise<Off | CheckOk | CheckFailed> {
    if (!fetcher.available()) return off()
    const entry = await getEntry(source)
    if (entry === undefined) return fail('unknown_source')
    const locked = await deps.withSourceLock(entry.source, async (): Promise<CheckOk | CheckFailed> => {
      const r = await runCheckCore(entry.source)
      const lib = await scanLibrary()
      if (!isFailure(r)) {
        const rows = computeRows(r.entry, r.index, lib, true)
        return { ok: true, entry: r.entry, ...(r.commit !== undefined ? { commit: r.commit } : {}), rows, counts: countsOf(r.entry, rows) }
      }
      // The rows show the entry as it is now: a skip, keep or unfollow made
      // during the fetch takes no lock.
      const now = await getEntry(entry.source)
      const cached = now === undefined ? null : await loadIndex(now.source)
      const failed: CheckFailed = { ...r }
      if (now !== undefined && cached !== null) failed.cached = { rows: computeRows(now, cached.index, lib, true), lastGoodAt: now.lastGoodAt ?? null }
      return failed
    })
    return locked.ran ? locked.value : fail('busy')
  }

  // -------------------------------------------------------------- catalog

  async function fetchCatalog(persist = true): Promise<{ ok: true; data: CatalogData } | CatalogFailed> {
    const cached = (await loadCatalog())?.data ?? null
    const got = await fetchRaw(deps.catalogUrl, MAX_INDEX)
    if (!got.ok) return { ...got, cached }
    let data: CatalogData | null = null
    try {
      data = normalizeCatalog(JSON.parse(dec.decode(got.bytes)), wasm)
    } catch {
      data = null
    }
    if (data === null) return { ...fail('catalog_invalid'), cached }
    if (persist) await writeCache(CATALOG_KEY, enc.encode(JSON.stringify({ at: nowIso(), catalog: data })))
    return { ok: true, data }
  }

  async function catalog(): Promise<Off | CatalogOk | CatalogFailed> {
    if (!fetcher.available()) return off()
    if (deps.catalogUrl === '') return { ...fail('catalog_disabled'), cached: null }
    const r = await fetchCatalog()
    if (!r.ok) return r
    const removedSources: CatalogOk['removedSources'] = []
    for (const e of await readEntries()) {
      const bare = withoutRef(wasm, e.source)
      const hit = r.data.removed.find((x) => withoutRef(wasm, x.source) === bare)
      if (hit !== undefined) removedSources.push({ source: e.source, severity: hit.severity, reason: hit.reason, ...(hit.link !== undefined ? { link: hit.link } : {}) })
    }
    return { ok: true, catalog: r.data, removedSources }
  }

  async function catalogPreview(entry: CatalogSourceEntry): Promise<Off | CatalogPreviewOk | Failure> {
    if (!fetcher.available()) return off()
    if (deps.catalogUrl === '') return fail('catalog_disabled')
    // The snapshot is a path under the catalog's own folder (6.3): never
    // another host, and never another path on the same host (the default
    // catalog's host serves every GitHub repository).
    if (typeof entry.snapshot !== 'string' || !isSafeRelPath(entry.snapshot, '.json')) return fail('catalog_invalid')
    let url: URL
    let folder: URL
    try {
      folder = new URL('.', deps.catalogUrl)
      url = new URL(entry.snapshot, folder)
    } catch {
      return fail('catalog_invalid')
    }
    if (url.protocol !== 'https:' || url.origin !== folder.origin || !url.pathname.startsWith(folder.pathname) || url.search !== '' || url.hash !== '') {
      return fail('catalog_invalid')
    }
    const key = await keyFor('snap', entry.source)
    const got = await fetchRaw(url.href, MAX_INDEX)
    let failure: Failure | null = null
    let index: SourceIndex | null = null
    let stale = false
    if (got.ok) {
      const parsed = parseIdx(got.bytes)
      if (parsed.ok) {
        index = parsed.index
        await writeCache(key, got.bytes)
      } else failure = parsed
    } else failure = got
    if (index === null) {
      const cachedBytes = await readCache(key)
      const parsed = cachedBytes === null ? null : parseIdx(cachedBytes)
      if (parsed === null || !parsed.ok) return failure ?? fail('index_invalid')
      index = parsed.index
      stale = true
    }
    const lib = await scanLibrary()
    const rows = computeRows({ source: entry.source, skipped: [], kept: {} }, index, lib, false)
    return { ok: true, rows, summary: summarize(index), stale }
  }

  // ----------------------------------------------------------- add source

  /** Which catalog listing a canonical source has. `persist` false reads the
   * catalog without writing the cache (a peek leaves no trace). */
  async function listingFor(canonical: string, fetchIfMissing: boolean, persist: boolean): Promise<AddSourceOk['listing']> {
    if (deps.catalogUrl === '') return 'unknown'
    let data = (await loadCatalog())?.data ?? null
    if (data === null && fetchIfMissing) {
      const r = await fetchCatalog(persist)
      data = r.ok ? r.data : null
    }
    if (data === null) return 'unknown'
    const bare = withoutRef(wasm, canonical)
    return data.sources.some((s) => withoutRef(wasm, s.source) === bare) ? 'listed' : 'unlisted'
  }

  /** Tries each candidate reading of an address; the first index that loads
   * wins. Writes nothing. */
  async function fetchFirstCandidate(refs: SourceRef[]): Promise<{ ok: true; ref: SourceRef; fetched: Fetched } | Failure> {
    const multi = refs.length > 1
    const causes: SourceErrorCode[] = []
    let tooNew = false
    let single: Failure | null = null
    for (const ref of refs) {
      const r = multi ? await withTimeout(fetchIndex(ref), CANDIDATE_MS) : await fetchIndex(ref)
      if (isFailure(r)) {
        causes.push(r.code)
        if (r.code === 'index_too_new') tooNew = true
        single = r
        continue
      }
      return { ok: true, ref, fetched: r }
    }
    if (!multi && single !== null) return single
    return tooNew ? fail('index_too_new') : fail('no_source', { causes })
  }

  async function peekSource(text: string): Promise<Off | PeekOk | Failure> {
    if (!fetcher.available()) return off()
    const refs = wasm.parseSourceRef(text)
    if ('error' in refs || refs.length === 0) return fail('ref_invalid')
    const known = (await readEntries()).find((e) => refs.some((r) => r.canonical === e.source))
    if (known !== undefined) {
      const ref = refs.find((r) => r.canonical === known.source) ?? refs[0]
      let index = (await loadIndex(known.source))?.index ?? null
      if (index === null && !deps.autoCheckEnabled()) {
        return { ok: true, existing: true, needsCheck: true, source: known.source, summary: emptySummary(known), host: ref.host, listing: await listingFor(known.source, false, false) }
      }
      if (index === null) {
        const r = await fetchIndex(ref)
        if (isFailure(r)) return r
        index = r.index
      }
      return { ok: true, existing: true, source: known.source, summary: summarize(index), host: ref.host, listing: await listingFor(known.source, false, false) }
    }
    const got = await fetchFirstCandidate(refs)
    if (isFailure(got)) return got
    return {
      ok: true,
      existing: false,
      source: got.ref.canonical,
      summary: summarize(got.fetched.index),
      host: got.ref.host,
      listing: await listingFor(got.ref.canonical, true, false),
    }
  }

  async function addSource(text: string): Promise<Off | AddSourceOk | Failure> {
    if (!fetcher.available()) return off()
    const refs = wasm.parseSourceRef(text)
    if ('error' in refs || refs.length === 0) return fail('ref_invalid')
    const entries = await readEntries()
    const known = entries.find((e) => refs.some((r) => r.canonical === e.source))
    if (known !== undefined) {
      const ref = refs.find((r) => r.canonical === known.source) ?? refs[0]
      let cached = await loadIndex(known.source)
      let entry = known
      if (cached === null && !deps.autoCheckEnabled()) {
        return { ok: true, existing: true, needsCheck: true, entry, summary: emptySummary(entry), host: ref.host, listing: await listingFor(known.source, false, true) }
      }
      if (cached === null) {
        const r = await runCheckCore(known.source)
        if (isFailure(r)) return r
        entry = r.entry
        cached = { bytes: new Uint8Array(0), index: r.index }
      }
      return { ok: true, existing: true, entry, summary: summarize(cached.index), host: ref.host, listing: await listingFor(known.source, false, true) }
    }
    const got = await fetchFirstCandidate(refs)
    if (isFailure(got)) return got
    const { ref, fetched: r } = got
    const at = nowIso()
    const entry: SourceEntry = {
      source: ref.canonical,
      kind: ref.kind,
      name: r.index.name,
      followed: false,
      addedAt: at,
      lastCheck: { at, ok: true, code: null },
      skipped: [],
      kept: {},
      lastGoodAt: at,
      commit: r.commit ?? null,
    }
    try {
      await exclusive(async () => {
        await writeCache(await keyFor('idx', ref.canonical), r.bytes)
        await putEntry(entry)
      })
    } catch {
      return fail('write_failed')
    }
    return { ok: true, existing: false, entry, summary: summarize(r.index), host: ref.host, listing: await listingFor(ref.canonical, true, true) }
  }

  // ---------------------------------------------------------- taking items

  async function takeItem(ctx: Ctx, id: string, lib: LibIndex): Promise<AddItemOk | Failure> {
    let item = ctx.index.items.find((i) => i.id === id)
    if (item === undefined) return fail('item_not_found')
    const local = lib.byId.get(id)
    let bytes: Uint8Array | undefined
    for (let attempt = 0; bytes === undefined; attempt++) {
      const urls = wasm.sourceUrls(ctx.ref, [item.path], ctx.entry.commit ?? undefined)
      if ('error' in urls) return fail('ref_invalid')
      const got = await fetchRaw(urls.files[0], item.bytes)
      // A body longer than the entry's `bytes` is a hash mismatch (6.4).
      if (!got.ok && got.code !== 'too_large') return got
      if (got.ok && (await items.sha256Hex(got.bytes)) === item.sha256) {
        bytes = got.bytes
        break
      }
      if (attempt >= 1) return fail('hash_mismatch')
      const fresh = await runCheckCore(ctx.entry.source)
      if (isFailure(fresh)) return fresh
      ctx.entry = fresh.entry
      ctx.index = fresh.index
      const again = ctx.index.items.find((i) => i.id === id)
      if (again === undefined) return fail('item_not_found')
      item = again
    }
    let meta: ItemFileMeta
    try {
      meta = await items.readMeta(bytes)
    } catch {
      return fail('file_invalid')
    }
    if (meta.id !== item.id || LIB_CATEGORY[item.category] !== meta.category) return fail('listing_incorrect')
    const minted = items.itemFileName({ name: meta.name, id: meta.id, category: meta.category })
    // Nothing is overwritten (5.2): not another id's file, and not a file the
    // scan could not read (it may be anything). Matched case-insensitively,
    // as a case-insensitive disk would match it.
    if (lib.unreadable.has(fileKey(minted))) return fail('name_clash')
    const atPath = lib.byFile.get(fileKey(minted))
    if (atPath !== undefined && atPath.meta.id !== meta.id) return fail('name_clash')
    // The same item already there under another case is written in place, so
    // the remove of an "old" path below can never delete the file just written.
    const file = atPath !== undefined ? atPath.file : minted

    const license = item.license ?? ctx.index.license ?? undefined
    const note: LibraryRemoteNote = {
      source: ctx.entry.source,
      name: ctx.entry.name,
      path: item.path,
      sha256: item.sha256,
      ...(ctx.entry.commit ? { commit: ctx.entry.commit } : {}),
      ...(license ? { license } : {}),
      fetchedAt: nowIso(),
      edited: false,
    }
    // The user's own filing is kept for an existing item; a first add files
    // under the source's name.
    const collection = local !== undefined ? local.meta.collection : meta.collection ? `${ctx.entry.name}/${meta.collection}` : ctx.entry.name
    let stamped: Uint8Array
    try {
      stamped = await items.stamp(bytes, { remote: note, collection })
    } catch {
      return fail('file_invalid')
    }
    try {
      await store.write(file, stamped)
    } catch {
      return fail('write_failed')
    }
    lib.put({ file, meta: { ...meta, collection, remote: note } }, true)
    const out: AddItemOk = { ok: true, file, id, replaced: local !== undefined, sha256: item.sha256 }
    if (local !== undefined && fileKey(local.file) !== fileKey(file)) {
      let removed = false
      try {
        await store.remove(local.file)
        removed = true
      } catch {
        // Reported on the result (7.4): both files now carry this id.
        out.removeFailed = true
      }
      if (removed) lib.drop(local.file)
    }
    if (ctx.entry.kept[id] !== undefined) {
      try {
        const next = await updateEntry(ctx.entry.source, (e) => {
          if (e.kept[id] === undefined) return null
          const { [id]: _gone, ...rest } = e.kept
          return { ...e, kept: rest }
        })
        if (next !== undefined) ctx.entry = next
      } catch {
        // The item has landed; a Kept record left behind only names a sha256
        // the note now matches, so it no longer applies. The result, with any
        // removeFailed, must still reach the caller.
      }
    }
    return out
  }

  async function addItem(source: string, id: string, opts: AddItemOptions = {}): Promise<AddItemResult> {
    if (!fetcher.available()) return off()
    const opened = await openCtx(source)
    if (isFailure(opened)) return opened
    const { ctx } = opened
    const removed = await removedFor(ctx.entry.source)
    if (removed !== null) return removedFailure(removed)
    if (!ctx.index.items.some((i) => i.id === id)) return fail('item_not_found')
    const lib = await scanLibrary()
    const local = lib.byId.get(id)
    if (local !== undefined) {
      const note = local.meta.remote
      if (note !== undefined) {
        return note.source === ctx.entry.source ? fail('already_in_library') : fail('unavailable_other_source', { otherSource: note.name })
      }
      // As Update (5.2): never write under an open edit window. Asked before
      // the confirm, so the user is not asked a question whose Yes cannot work.
      if ((await store.editingNames()).includes(local.file)) return fail('open_for_editing')
      if (opts.replace !== true) return fail('needs_confirm', { confirm: 'replace_item' })
    }
    return takeItem(ctx, id, lib)
  }

  async function updateItem(source: string, id: string, opts: UpdateOptions = {}): Promise<AddItemResult> {
    if (!fetcher.available()) return off()
    const opened = await openCtx(source)
    if (isFailure(opened)) return opened
    const { ctx } = opened
    const removed = await removedFor(ctx.entry.source)
    if (removed !== null) return removedFailure(removed)
    const lib = await scanLibrary()
    const local = lib.byId.get(id)
    if (local === undefined) return fail('not_found')
    const note = local.meta.remote
    if (note === undefined || note.source !== ctx.entry.source) return fail('not_linked')
    const item = ctx.index.items.find((i) => i.id === id)
    if (item === undefined) return fail('not_updatable', { state: 'gone' })
    if (note.sha256 === item.sha256) return fail('not_updatable', { state: 'in_step' })
    if (opts.decision === 'keep') {
      await updateEntry(ctx.entry.source, (e) => ({ ...e, kept: { ...e.kept, [id]: item.sha256 } }))
      return { ok: true, kept: true, file: local.file, id, replaced: false, sha256: item.sha256 }
    }
    if ((await store.editingNames()).includes(local.file)) return fail('open_for_editing')
    if (note.edited && opts.decision !== 'replace') return fail('needs_confirm', { confirm: 'replace_changes' })
    return takeItem(ctx, id, lib)
  }

  // ----------------------------------------------------------------- runs

  async function runAll(source: string, opts: RunOptions, follow: boolean): Promise<RunResult> {
    if (!fetcher.available()) return off()
    const entry = await getEntry(source)
    if (entry === undefined) return fail('unknown_source')
    const locked = await deps.withSourceLock(entry.source, async (): Promise<RunResult> => {
      const removed = await removedFor(entry.source)
      if (removed !== null) return removedFailure(removed)
      const fresh = await runCheckCore(entry.source)
      if (isFailure(fresh)) return fresh
      const refs = wasm.parseSourceRef(fresh.entry.source)
      if ('error' in refs || refs.length === 0) return fail('ref_invalid')
      const ctx: Ctx = { entry: fresh.entry, ref: refs[0], index: fresh.index }
      const lib = await scanLibrary()
      const skipped = new Set(ctx.entry.skipped)
      const wanted = opts.ids !== undefined ? new Set(opts.ids) : null
      const candidates = ctx.index.items.filter((i) => !lib.byId.has(i.id) && !skipped.has(i.id) && (wanted === null || wanted.has(i.id)))
      const profilesOn = deps.profilesEnabled()
      const todo = candidates.filter((i) => i.category !== 'Profiles' || profilesOn)
      const profilesSkipped = candidates.length - todo.length
      const space = await deps.storageEstimate()
      if (space !== null) {
        const needed = todo.reduce((n, i) => n + i.bytes, 0)
        const free = Math.max(0, space.quota - space.usage)
        if (needed > free) return fail('not_enough_space', { needed, free })
      }
      if (follow) await updateEntry(entry.source, (e) => ({ ...e, followed: true }))

      const summary: RunSummary = { added: 0, failed: 0, retry: [], failures: [], notTried: [], cancelled: false, stopped: null, profilesSkipped }
      let networkRun = 0
      for (let i = 0; i < todo.length; i++) {
        if (summary.stopped !== null || opts.signal?.aborted === true) {
          summary.notTried = todo.slice(i).map((x) => x.id)
          break
        }
        const id = todo[i].id
        const r = await takeItem(ctx, id, lib)
        if (isFailure(r)) {
          summary.failed++
          summary.retry.push(id)
          summary.failures.push({ id, code: r.code })
          if (r.code === 'network' || r.code === 'timeout' || r.code === 'offline') networkRun++
          else networkRun = 0
          if (r.code === 'rate_limited') summary.stopped = 'rate_limited'
          else if (networkRun >= NETWORK_RUN_LIMIT) summary.stopped = 'network'
        } else {
          summary.added++
          networkRun = 0
        }
        opts.onProgress?.({ done: i + 1, total: todo.length, added: summary.added, failed: summary.failed, id })
      }
      summary.cancelled = opts.signal?.aborted === true && summary.notTried.length > 0
      return { ok: true, summary }
    })
    return locked.ran ? locked.value : fail('busy')
  }

  async function updateAll(source: string): Promise<UpdateAllResult> {
    if (!fetcher.available()) return off()
    const entry = await getEntry(source)
    if (entry === undefined) return fail('unknown_source')
    const locked = await deps.withSourceLock(entry.source, async (): Promise<UpdateAllResult> => {
      const removed = await removedFor(entry.source)
      if (removed !== null) return removedFailure(removed)
      const fresh = await runCheckCore(entry.source)
      if (isFailure(fresh)) return fresh
      const refs = wasm.parseSourceRef(fresh.entry.source)
      if ('error' in refs || refs.length === 0) return fail('ref_invalid')
      const ctx: Ctx = { entry: fresh.entry, ref: refs[0], index: fresh.index }
      const lib = await scanLibrary()
      const editing = new Set(await store.editingNames())
      const rows: UpdateRow[] = []
      const needsReview: string[] = []
      for (const row of computeRows(ctx.entry, ctx.index, lib, false)) {
        if (row.state !== 'newer' || row.kept) continue
        if (row.edited) {
          needsReview.push(row.id)
          continue
        }
        const result: AddItemResult = row.file !== null && editing.has(row.file) ? fail('open_for_editing') : await takeItem(ctx, row.id, lib)
        rows.push({ id: row.id, result })
      }
      return { ok: true, rows, needsReview }
    })
    return locked.ran ? locked.value : fail('busy')
  }

  async function thumbnail(source: string, id: string): Promise<ThumbnailResult> {
    if (!fetcher.available()) return off()
    const opened = await openCtx(source)
    if (isFailure(opened)) return opened
    const { ctx } = opened
    const item = ctx.index.items.find((i) => i.id === id)
    if (item === undefined) return fail('item_not_found')
    if (!item.thumbnail) return { ok: true, bytes: null }
    const key = await keyFor('thumb', `${ctx.entry.source}\n${item.thumbnail}`)
    const cached = await readCache(key)
    if (cached !== null) return { ok: true, bytes: cached }
    const urls = wasm.sourceUrls(ctx.ref, [item.thumbnail], ctx.entry.commit ?? undefined)
    if ('error' in urls) return fail('ref_invalid')
    const got = await fetchRaw(urls.files[0], MAX_THUMB)
    if (!got.ok) return got
    // The fetch took time: the source may have been removed, or a check may
    // have dropped the item, and either one has already swept. Cache the
    // thumbnail only while the source is listed and its cached index still
    // names it, or nothing would ever delete it.
    const thumbPath = item.thumbnail
    await exclusive(async () => {
      const now = await getEntry(ctx.entry.source)
      if (now === undefined) return
      const live = await loadIndex(now.source)
      if (live === null || !live.index.items.some((i) => i.thumbnail === thumbPath)) return
      await writeCache(key, got.bytes)
    })
    return { ok: true, bytes: got.bytes }
  }

  // ------------------------------------------------------------ scheduler

  const isDue = (iso: string | undefined): boolean => {
    if (iso === undefined) return true
    const t = Date.parse(iso)
    return Number.isNaN(t) || deps.now().getTime() - t > DUE_MS
  }

  async function shouldAutoCheck(source: string): Promise<boolean> {
    if (!deps.autoCheckEnabled() || !fetcher.available()) return false
    const entry = await getEntry(source)
    return entry !== undefined && isDue(entry.lastCheck?.at)
  }

  async function shouldAutoCheckCatalog(): Promise<boolean> {
    if (!deps.autoCheckEnabled() || !fetcher.available() || deps.catalogUrl === '') return false
    if ((await readEntries()).length === 0) return false
    return isDue((await loadCatalog())?.at)
  }

  async function autoCheck(): Promise<Off | { checked: string[]; catalogFetched: boolean }> {
    if (!fetcher.available()) return off()
    const checked: string[] = []
    for (const e of await readEntries()) {
      if (!(await shouldAutoCheck(e.source))) continue
      const r = await check(e.source)
      if (!isFailure(r) || r.code !== 'busy') checked.push(e.source)
    }
    let catalogFetched = false
    if (await shouldAutoCheckCatalog()) {
      const r = await catalog()
      catalogFetched = 'ok' in r && r.ok
    }
    return { checked, catalogFetched }
  }

  // ------------------------------------------------------------ no network

  async function listSources(): Promise<SourceStatus[]> {
    const out: SourceStatus[] = []
    for (const entry of await readEntries()) {
      const removed = await removedFor(entry.source)
      out.push({
        entry,
        followed: entry.followed,
        unreachable: entry.lastCheck !== null && !entry.lastCheck.ok,
        removed: removed === null ? null : { reason: removed.reason, severity: removed.severity, ...(removed.link !== undefined ? { link: removed.link } : {}) },
      })
    }
    return out
  }

  /** Removes the entry and the source's cache entries: its index copy, its
   * catalog snapshot and its thumbnails (6.5). The catalog stays. */
  function removeSource(source: string): Promise<{ ok: true } | Failure> {
    return exclusive(async (): Promise<{ ok: true } | Failure> => {
      if ((await getEntry(source)) === undefined) return fail('unknown_source')
      const before = await loadIndex(source)
      try {
        await store.writeSources(source, null)
      } catch {
        return fail('write_failed')
      }
      const candidates = before !== null ? await thumbKeysOf(source, before.index) : new Set<string>()
      candidates.add(await keyFor('idx', source))
      candidates.add(await keyFor('snap', source))
      await sweep(candidates)
      return { ok: true }
    })
  }

  async function unfollow(source: string): Promise<{ ok: true } | Failure> {
    try {
      const e = await updateEntry(source, (cur) => (cur.followed ? { ...cur, followed: false } : null))
      return e === undefined ? fail('unknown_source') : { ok: true }
    } catch {
      return fail('write_failed')
    }
  }

  async function itemStates(source: string): Promise<ItemStatesOk | Failure> {
    const opened = await openCtx(source)
    if (isFailure(opened)) return opened
    const { ctx } = opened
    const rows = computeRows(ctx.entry, ctx.index, await scanLibrary(), true)
    return { ok: true, rows, lastGoodAt: ctx.entry.lastGoodAt ?? null, ...(ctx.entry.commit ? { commit: ctx.entry.commit } : {}) }
  }

  async function localStates(): Promise<LocalState[]> {
    const entries = new Map((await readEntries()).map((e) => [e.source, e]))
    const indexes = new Map<string, SourceIndex | null>()
    const out: LocalState[] = []
    for (const rec of (await scanLibrary()).recs) {
      const note = rec.meta.remote
      if (note === undefined) continue
      const entry = entries.get(note.source)
      let state: LocalState['state'] = 'orphan'
      let kept = false
      if (entry !== undefined) {
        if (!indexes.has(entry.source)) indexes.set(entry.source, (await loadIndex(entry.source))?.index ?? null)
        const index = indexes.get(entry.source) ?? null
        if (index === null) state = 'unchecked'
        else {
          const it = index.items.find((i) => i.id === rec.meta.id)
          state = it === undefined ? 'gone' : it.sha256 === note.sha256 ? 'in_step' : 'newer'
          kept = state === 'newer' && entry.kept[rec.meta.id] === it?.sha256
        }
      }
      out.push({ id: rec.meta.id, file: rec.file, name: rec.meta.name, note, state, edited: note.edited === true, kept })
    }
    return out
  }

  async function newCounts(source: string): Promise<NewCounts> {
    const none = { new: 0, changed: 0 }
    const opened = await openCtx(source)
    if (isFailure(opened)) return none
    const { ctx } = opened
    if ((await removedFor(ctx.entry.source)) !== null) return none
    return countsOf(ctx.entry, computeRows(ctx.entry, ctx.index, await scanLibrary(), true))
  }

  /** The local item that must be linked to `source`, or the refusal. */
  async function linkedLocal(source: string, id: string): Promise<{ lib: LibIndex; rec: LibRec; note: LibraryRemoteNote } | Failure> {
    const lib = await scanLibrary()
    const rec = lib.byId.get(id)
    if (rec === undefined) return fail('not_found')
    const note = rec.meta.remote
    if (note === undefined || note.source !== source) return fail('not_linked')
    return { lib, rec, note }
  }

  async function keepMine(source: string, id: string): Promise<{ ok: true } | Failure> {
    const opened = await openCtx(source)
    if (isFailure(opened)) return opened
    const { ctx } = opened
    const linked = await linkedLocal(ctx.entry.source, id)
    if (isFailure(linked)) return linked
    const item = ctx.index.items.find((i) => i.id === id)
    if (item === undefined) return fail('not_updatable', { state: 'gone' })
    if (item.sha256 === linked.note.sha256) return fail('not_updatable', { state: 'in_step' })
    try {
      await updateEntry(ctx.entry.source, (e) => ({ ...e, kept: { ...e.kept, [id]: item.sha256 } }))
    } catch {
      return fail('write_failed')
    }
    return { ok: true }
  }

  async function unlink(source: string, id: string): Promise<{ ok: true } | Failure> {
    const linked = await linkedLocal(source, id)
    if (isFailure(linked)) return linked
    try {
      const stamped = await items.stamp(await store.read(linked.rec.file), { remote: null })
      await store.write(linked.rec.file, stamped)
    } catch {
      return fail('write_failed')
    }
    try {
      await updateEntry(source, (e) => {
        if (e.kept[id] === undefined) return null
        const { [id]: _gone, ...rest } = e.kept
        return { ...e, kept: rest }
      })
    } catch {
      return fail('write_failed')
    }
    return { ok: true }
  }

  async function noteDeleted(id: string, remote: { source: string } | undefined): Promise<void> {
    if (remote === undefined) return
    try {
      await updateEntry(remote.source, (e) => {
        const skip = e.followed && !e.skipped.includes(id)
        const drop = e.kept[id] !== undefined
        if (!skip && !drop) return null
        const { [id]: _gone, ...rest } = e.kept
        return { ...e, skipped: skip ? [...e.skipped, id] : e.skipped, kept: rest }
      })
    } catch {
      // The delete already happened; losing a skip-list entry only offers the item again.
    }
  }

  return {
    catalogEnabled: () => deps.catalogUrl !== '',
    addSource,
    peekSource,
    check,
    catalog,
    catalogPreview,
    addItem,
    updateItem,
    addAll: (source, opts = {}) => runAll(source, opts, false),
    follow: (source, opts = {}) => runAll(source, opts, true),
    updateAll,
    thumbnail,
    autoCheck,
    listSources,
    removeSource,
    unfollow,
    itemStates,
    localStates,
    newCounts,
    keepMine,
    unlink,
    noteDeleted,
    shouldAutoCheck,
    shouldAutoCheckCatalog,
  }
}
