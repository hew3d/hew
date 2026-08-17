/**
 * sceneThumbnails — IndexedDB-backed cache of derived Scene thumbnails,
 * keyed by `${contentHash}:${sid}` (docs/design/scenes.md §5 "Add": the
 * thumbnail is derived at runtime, never stored in the `.hew` file itself,
 * so it needs a durable side-cache to survive a reload). Mirrors
 * `io/recents.ts`'s IndexedDB idioms (guarded-everywhere, never throws,
 * fire-and-forget from the caller's side) rather than reusing that module
 * directly — recents keys by document identity alone, this keys by
 * document identity PLUS Scene id, and caps by entry COUNT rather than
 * total bytes (each entry is one small JPEG data URL, not a whole document).
 *
 * App.tsx wires this at two seams: `applyLoadedBytes` loads the cached map
 * for the freshly-loaded bytes' content hash and merges it into the
 * `ScenesController` (`mergeThumbnails`); a successful save persists the
 * controller's current thumbnail map under the NEWLY WRITTEN bytes' hash
 * (a save can change the content hash — e.g. Save As, or any edit since the
 * last save — so re-keying on every save is what keeps a later re-open
 * finding its thumbnails). Both call sites are fire-and-forget: a cache
 * failure must never block loading or saving a document.
 */

const DB_NAME = 'hew-scene-thumbnails'
const STORE_NAME = 'thumbnails'
const DB_VERSION = 1

/** LRU cap: total entries across every document's Scenes combined. Each
 *  entry is one small (SCENE_THUMB_WIDTH_PX x SCENE_THUMB_HEIGHT_PX) JPEG
 *  data URL, so a byte cap isn't worth the bookkeeping recents.ts's own
 *  (whole-document-sized) entries need. */
export const MAX_ENTRIES = 500

/** One cached thumbnail row. `key` is the object store's keyPath — computed
 *  by `thumbnailKey`, never hand-assembled at a call site. */
export interface ScenesThumbnailEntry {
  key: string
  contentHash: string
  sid: number
  dataUrl: string
  timestamp: number
}

/** The composite primary key for one document's one Scene's thumbnail. */
export function thumbnailKey(contentHash: string, sid: number): string {
  return `${contentHash}:${sid}`
}

/** The slice `selectThumbnailEvictions` needs — kept separate from the full
 *  entry (mirrors `io/recents.ts`'s own `RecentMeta`) so the pure eviction
 *  rule never has to look at (or copy) any data URL. */
export interface ThumbnailMeta {
  key: string
  timestamp: number
}

/**
 * Pure LRU eviction rule: given every entry that would exist after an
 * insert (including the new ones), return the keys that no longer fit
 * under `maxEntries`. Sorted newest-first; everything past the cap evicts.
 */
export function selectThumbnailEvictions(
  entries: readonly ThumbnailMeta[],
  maxEntries: number = MAX_ENTRIES,
): string[] {
  const sorted = [...entries].sort((a, b) => b.timestamp - a.timestamp)
  return sorted.slice(maxEntries).map((e) => e.key)
}

/** `indexedDB` if present and reachable, else `undefined` — mirrors
 *  `io/recents.ts`'s own `safeIndexedDB` (same guard, same reasoning). */
function safeIndexedDB(): IDBFactory | undefined {
  if (typeof indexedDB === 'undefined') return undefined
  try {
    return indexedDB
  } catch {
    return undefined
  }
}

function openDb(idb: IDBFactory): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = idb.open(DB_NAME, DB_VERSION)
    req.onupgradeneeded = () => {
      const db = req.result
      if (!db.objectStoreNames.contains(STORE_NAME)) {
        db.createObjectStore(STORE_NAME, { keyPath: 'key' })
      }
    }
    req.onsuccess = () => resolve(req.result)
    req.onerror = () => reject(req.error ?? new Error('indexedDB.open failed'))
  })
}

function requestToPromise<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result)
    req.onerror = () => reject(req.error ?? new Error('IDBRequest failed'))
  })
}

/**
 * Persist every `(sid -> dataUrl)` pair in `thumbs` under `contentHash`.
 * Fire-and-forget from the caller's side — never throws, and a failed write
 * just means one fewer cached thumbnail next reload. Evicts the globally
 * oldest entries past `MAX_ENTRIES` after writing.
 */
export async function saveThumbnails(
  contentHash: string,
  thumbs: ReadonlyMap<number, string>,
  idb: IDBFactory | undefined = safeIndexedDB(),
): Promise<void> {
  if (idb === undefined || thumbs.size === 0) return
  try {
    const db = await openDb(idb)
    try {
      const tx = db.transaction(STORE_NAME, 'readwrite')
      const store = tx.objectStore(STORE_NAME)
      const now = Date.now()
      const existing = await requestToPromise<ScenesThumbnailEntry[]>(store.getAll())
      const written = new Set<string>()
      for (const [sid, dataUrl] of thumbs) {
        const key = thumbnailKey(contentHash, sid)
        written.add(key)
        const entry: ScenesThumbnailEntry = { key, contentHash, sid, dataUrl, timestamp: now }
        store.put(entry)
      }
      const rest = existing.filter((e) => !written.has(e.key))
      const metas: ThumbnailMeta[] = [
        ...rest.map((e) => ({ key: e.key, timestamp: e.timestamp })),
        ...[...written].map((key) => ({ key, timestamp: now })),
      ]
      for (const key of selectThumbnailEvictions(metas)) store.delete(key)
      await new Promise<void>((resolve, reject) => {
        tx.oncomplete = () => resolve()
        tx.onerror = () => reject(tx.error ?? new Error('transaction failed'))
      })
    } finally {
      db.close()
    }
  } catch {
    // Best-effort — never throw, never block the save that triggered it.
  }
}

/**
 * Load every cached thumbnail for `contentHash`, keyed by sid. Empty on any
 * failure (unavailable indexedDB, a request error, no cached entries) — the
 * caller (`applyLoadedBytes`) just shows placeholders in that case.
 */
export async function loadThumbnails(
  contentHash: string,
  idb: IDBFactory | undefined = safeIndexedDB(),
): Promise<Map<number, string>> {
  if (idb === undefined) return new Map()
  try {
    const db = await openDb(idb)
    try {
      const tx = db.transaction(STORE_NAME, 'readonly')
      const all = await requestToPromise<ScenesThumbnailEntry[]>(tx.objectStore(STORE_NAME).getAll())
      const out = new Map<number, string>()
      for (const e of all) {
        if (e.contentHash === contentHash) out.set(e.sid, e.dataUrl)
      }
      return out
    } finally {
      db.close()
    }
  } catch {
    return new Map()
  }
}
