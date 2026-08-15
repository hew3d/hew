/**
 * recents — IndexedDB-backed store of recently opened documents for the web
 * build (`webRecoveryStore.ts`'s "one IndexedDB, guarded everywhere, never
 * throws" pattern, applied to a small LRU list instead of a single autosave
 * slot).
 *
 * Every successful `.hew` open in the web build records `{bytes, name,
 * timestamp, source}` here — both shells hook their own open path at the
 * narrowest seam (App.tsx's File ▸ Open, ShopApp's Open…) right after the
 * load succeeds, fire-and-forget. Nothing else does: not saves, not
 * imports — the surface stays exactly "documents that were opened", so a
 * later re-open finds the same bytes without a network round trip. The web
 * WelcomeScreen and Shop Mode's empty state both list the result — together
 * with the PWA's precached app shell (registered in main.tsx), this is what
 * makes airplane-mode workshop use work after the first open.
 *
 * db: "hew-recents", object store "recents" (keyPath "id"). Capped at
 * `MAX_ENTRIES` entries and `MAX_TOTAL_BYTES` total bytes across every
 * entry — `selectEvictions` is the pure eviction rule, factored out so it's
 * unit-testable without touching IndexedDB at all.
 *
 * Guarded throughout: an unavailable `indexedDB` (private browsing, an
 * insecure-context quirk) or any request failure makes every operation a
 * silent no-op / empty result. This module must never throw, and a recents
 * failure must never interrupt the open flow that triggered it — callers
 * fire `recordRecent` without awaiting it for exactly that reason.
 */

const DB_NAME = 'hew-recents'
const STORE_NAME = 'recents'
// Bumped from 1 (playtest fix 3: content-identity dedupe) — the object
// store's SHAPE is unchanged (still keyPath 'id', no new index: at
// `MAX_ENTRIES`-scale a full `getAll()` scan is plenty cheap for the
// dedupe-by-`contentHash` lookup below), so `onupgradeneeded`'s own
// create-if-missing logic needs no new branch for this version — the bump
// exists purely so a stale-schema client re-runs it once. Old records
// simply lack `contentHash` (an additive field, `partCount`'s own
// precedent) and are treated as non-deduping, never as an error.
const DB_VERSION = 2

/** LRU cap: entry count. */
export const MAX_ENTRIES = 20
/** LRU cap: total stored bytes across every entry. */
export const MAX_TOTAL_BYTES = 50 * 1024 * 1024

/** Where a recorded recent came from. Today only the picker-driven "Open"
 *  flow in either shell records anything — a future QR/LAN handoff open
 *  (docs' §4, not part of this effort) would add a source here without
 *  changing this module's shape. */
export type RecentSource = 'open'

/** One recorded recent, as stored and as listed. */
export interface RecentEntry {
  id: string
  bytes: Uint8Array
  name: string
  timestamp: number
  source: RecentSource
  /** Total part count at the moment this was recorded (design_handoff_shop_
   *  mode/README.md §8: the empty state's Recents rows read "2 hours ago ·
   *  9 parts"). Additive/optional — entries written before this field
   *  existed simply lack the key, and every reader treats `undefined` as
   *  "unknown, don't show a count" rather than 0. Not recomputed on
   *  read/re-open; it's a snapshot of what the document held when it was
   *  last opened, same freshness contract as `timestamp`. */
  partCount?: number
  /** SHA-256 hex digest of `bytes` (playtest fix 3: content-identity
   *  dedupe) — computed once in `recordRecent` via `crypto.subtle.digest`
   *  (Shop Mode runs in a secure context, per module doc). Additive/
   *  optional exactly like `partCount`: entries written before this field
   *  existed, or written in an environment without `crypto.subtle`, simply
   *  lack it — `recordRecent` treats a missing/unavailable hash as
   *  non-deduping (it always inserts a fresh row) rather than an error. */
  contentHash?: string
  /** A small (~92×92) JPEG data URL rendered from the model right after it
   *  loaded (playtest fix 4) — replaces the striped placeholder swatch in
   *  the empty-state Recents row. Additive/optional: a capture failure (no
   *  `ViewportApi`, an empty scene, a canvas error) falls back silently to
   *  the placeholder, same freshness contract as `partCount`/`contentHash`
   *  — not recomputed on read, a snapshot of the model as it looked when
   *  this entry was last written. */
  thumbnail?: string
}

/** The slice of a `RecentEntry` the eviction rule needs — id, freshness,
 *  and size — kept separate from the full entry so `selectEvictions` stays
 *  pure and never needs to look at (or copy) any `Uint8Array` payload. */
export interface RecentMeta {
  id: string
  timestamp: number
  byteLength: number
}

/**
 * Pure LRU + size-cap eviction rule: given every entry that would exist
 * after an insert (including the new one), return the ids that no longer
 * fit. Sorts newest-first and walks it accumulating byte size; once either
 * cap is exceeded at some entry, that entry and every OLDER one (the
 * running total only grows from there) is evicted.
 *
 * A freshly inserted entry carries the newest timestamp, so it only ever
 * appears in the result when it alone exceeds `maxTotalBytes` — callers
 * should refuse to write such an entry in the first place rather than
 * write-then-immediately-evict it (`recordRecent` does exactly that).
 */
export function selectEvictions(
  entries: readonly RecentMeta[],
  maxEntries: number = MAX_ENTRIES,
  maxTotalBytes: number = MAX_TOTAL_BYTES,
): string[] {
  const sorted = [...entries].sort((a, b) => b.timestamp - a.timestamp)
  const evict: string[] = []
  let total = 0
  for (let i = 0; i < sorted.length; i++) {
    total += sorted[i].byteLength
    if (i >= maxEntries || total > maxTotalBytes) {
      evict.push(sorted[i].id)
    }
  }
  return evict
}

/** `indexedDB` if present and reachable, else `undefined` — the shared
 *  guard every entry point defaults to. A function (not a constant) and
 *  re-checked on every call, mirroring `platform.isCoarsePointer()`: some
 *  environments (older Safari private-mode, certain insecure-context edge
 *  cases) expose the global but throw on first touch rather than simply
 *  omitting it, so the read itself is guarded too. Takes no state — callers
 *  needing dependency injection (tests) pass their own `IDBFactory` as the
 *  `idb` parameter on `recordRecent`/`listRecents` instead, the same
 *  storage-injection shape `shellMode.ts`'s `readShellModeOverride` uses
 *  for `localStorage`. */
function safeIndexedDB(): IDBFactory | undefined {
  if (typeof indexedDB === 'undefined') return undefined
  try {
    return indexedDB
  } catch {
    return undefined
  }
}

/** Open (creating if needed) the recents database. */
function openDb(idb: IDBFactory): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = idb.open(DB_NAME, DB_VERSION)
    req.onupgradeneeded = () => {
      const db = req.result
      if (!db.objectStoreNames.contains(STORE_NAME)) {
        db.createObjectStore(STORE_NAME, { keyPath: 'id' })
      }
    }
    req.onsuccess = () => resolve(req.result)
    req.onerror = () => reject(req.error ?? new Error('indexedDB.open failed'))
  })
}

/** Wrap an IDBRequest in a Promise. */
function requestToPromise<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result)
    req.onerror = () => reject(req.error ?? new Error('IDBRequest failed'))
  })
}

/** A fresh id for a new entry — collision-proof enough for a ≤20-entry
 *  list; falls back off `crypto.randomUUID` for any environment that lacks
 *  it (older WebKitGTK, some test runners). */
function newId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID()
  }
  return `${Date.now()}-${Math.random().toString(36).slice(2)}`
}

/** SHA-256 hex digest of `bytes` (playtest fix 3) — `RecentEntry.contentHash`'s
 *  own doc comment covers the additive-field/non-deduping-fallback contract.
 *  `undefined` (never throws) when `crypto.subtle` is unavailable — an
 *  insecure context or a very old test runner — matching every other
 *  best-effort guard in this module. */
async function hashBytes(bytes: Uint8Array): Promise<string | undefined> {
  if (typeof crypto === 'undefined' || crypto.subtle === undefined) return undefined
  try {
    // `Uint8Array<ArrayBufferLike>` (this project's TS/DOM lib version) isn't
    // structurally a `BufferSource` (which wants a plain `ArrayBuffer`, not
    // the `ArrayBuffer | SharedArrayBuffer` union) — same cast, same
    // reasoning, as `library/itemFiles.ts`'s own `sha256Hex` (a different
    // module, since this one needs to stay wasm-free — module doc). The
    // bytes are never mutated here, so the assertion is safe.
    const digest = await crypto.subtle.digest('SHA-256', bytes as BufferSource)
    return Array.from(new Uint8Array(digest)).map((b) => b.toString(16).padStart(2, '0')).join('')
  } catch {
    return undefined
  }
}

/**
 * Record a successful `.hew` open. Fire-and-forget from the caller's side —
 * resolves once written (or once it gives up), but a caller that doesn't
 * await it loses nothing: this never throws, and a lost write just means
 * one fewer recents row next launch.
 *
 * Content-identity dedupe (playtest fix 3): if an already-recorded entry's
 * `contentHash` matches this call's `bytes`, that row is UPDATED in place
 * (same `id`, fresh `timestamp`/`name`/`partCount`/`thumbnail`) instead of
 * inserting a second row for the same document — re-opening (or re-saving)
 * an unchanged model bumps it to the top of the list rather than stacking
 * duplicates. An entry with no `contentHash` at all (written before this
 * fix, or hashed in an environment without `crypto.subtle`) never
 * dedupe-matches anything — `RecentEntry.contentHash`'s own doc comment.
 * `partCount`/`thumbnail` fall back to the matched row's own prior value
 * when THIS call doesn't supply one, so a follow-up call that only adds a
 * thumbnail (`applyOpenedBytes`'s own two-call sequence — the first records
 * immediately, a second one merges in the thumbnail once it's captured)
 * never regresses a value an earlier call already recorded.
 *
 * Refuses (silently) to write an entry that alone exceeds `MAX_TOTAL_BYTES`
 * — such a document isn't a case the recents shelf is meant to serve, and
 * writing-then-immediately-evicting it would just be wasted IndexedDB I/O.
 *
 * `idb` defaults to the real `indexedDB`; tests inject a fake. `partCount`/
 * `thumbnail` are appended last (rather than slotted before `idb`) so every
 * existing positional call — including every current call site — keeps
 * working unchanged; omit either to store no value for it (`RecentEntry`'s
 * own doc comments on how readers treat that).
 */
export async function recordRecent(
  bytes: Uint8Array,
  name: string,
  source: RecentSource = 'open',
  idb: IDBFactory | undefined = safeIndexedDB(),
  partCount?: number,
  thumbnail?: string,
): Promise<void> {
  if (idb === undefined) return
  if (bytes.byteLength > MAX_TOTAL_BYTES) return
  try {
    const contentHash = await hashBytes(bytes)
    const db = await openDb(idb)
    try {
      const tx = db.transaction(STORE_NAME, 'readwrite')
      const store = tx.objectStore(STORE_NAME)
      const existing = await requestToPromise<RecentEntry[]>(store.getAll())
      const dup = contentHash === undefined ? undefined : existing.find((e) => e.contentHash === contentHash)
      const incoming: RecentEntry = { id: dup?.id ?? newId(), bytes, name, timestamp: Date.now(), source }
      if (contentHash !== undefined) incoming.contentHash = contentHash
      const resolvedPartCount = partCount ?? dup?.partCount
      if (resolvedPartCount !== undefined) incoming.partCount = resolvedPartCount
      const resolvedThumbnail = thumbnail ?? dup?.thumbnail
      if (resolvedThumbnail !== undefined) incoming.thumbnail = resolvedThumbnail
      // Exclude the matched row from `existing` before building metas — it's
      // superseded by `incoming` (same `id`), and leaving it in would double-
      // count its bytes in the size-cap math below and risk `selectEvictions`
      // returning its id to delete right after `store.put(incoming)` just
      // wrote that same id.
      const rest = dup === undefined ? existing : existing.filter((e) => e.id !== dup.id)
      const metas: RecentMeta[] = [...rest, incoming].map((e) => ({
        id: e.id,
        timestamp: e.timestamp,
        byteLength: e.bytes.byteLength,
      }))
      const evict = new Set(selectEvictions(metas))
      // The incoming entry can only land in `evict` by exceeding
      // maxTotalBytes alone — already refused above — so this always writes.
      store.put(incoming)
      for (const id of evict) store.delete(id)
      await new Promise<void>((resolve, reject) => {
        tx.oncomplete = () => resolve()
        tx.onerror = () => reject(tx.error ?? new Error('transaction failed'))
      })
    } finally {
      db.close()
    }
  } catch {
    // Best-effort — never throw, never break the open flow that triggered it.
  }
}

/**
 * List every recorded recent, newest first. Empty on any failure
 * (unavailable indexedDB, a request error, …) — the caller (WelcomeScreen /
 * ShopApp) just renders no shelf in that case.
 *
 * `idb` defaults to the real `indexedDB`; tests inject a fake.
 */
export async function listRecents(
  idb: IDBFactory | undefined = safeIndexedDB(),
): Promise<RecentEntry[]> {
  if (idb === undefined) return []
  try {
    const db = await openDb(idb)
    try {
      const tx = db.transaction(STORE_NAME, 'readonly')
      const all = await requestToPromise<RecentEntry[]>(tx.objectStore(STORE_NAME).getAll())
      return all.sort((a, b) => b.timestamp - a.timestamp)
    } finally {
      db.close()
    }
  } catch {
    return []
  }
}
