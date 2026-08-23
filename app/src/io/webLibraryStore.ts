/**
 * WebLibraryStore — browser implementation of LibraryStore.
 *
 * One storage engine, two roots. Everything is written against
 * `FileSystemDirectoryHandle`, which both roots speak:
 *
 * - **Browser storage** (default): an origin-private file system directory
 *   (`navigator.storage.getDirectory()` → `library/`). No permission
 *   prompts, works in every evergreen browser, but private to this browser
 *   profile on this device — and cleared with site data, which is why the
 *   first successful write requests `navigator.storage.persist()` and the
 *   detail pane offers Download (capabilities().canDownload).
 *
 * - **A bound folder** (Chromium): a real directory the user picked via
 *   `showDirectoryPicker()`, persisted as a handle in IndexedDB. The
 *   library becomes ordinary `.hew` files on disk — the same folder the
 *   desktop app can use, or a cloud-synced folder (Dropbox/Drive/OneDrive),
 *   which is bring-your-own-cloud with zero cloud code. Permission does not
 *   always survive a browser restart: `list()` re-requests it while user
 *   activation is live, and otherwise reports `needsReconnect` through
 *   `webStorage()` so the Library dialog can show a real Reconnect button
 *   (whose click IS the user gesture `requestPermission` needs).
 *
 * Folder layout mirrors the desktop store exactly (tauriLibraryStore.ts):
 * `.hew` files at the root or one `Components/` / `Materials/` / `Models/`
 * subfolder down, plus a flat `.thumbnails/` cache keyed by content hash.
 * The same names are valid on both platforms, so a bound folder is
 * interchangeable with a desktop library folder.
 *
 * Config (mode + folder handle) lives in IndexedDB db `hew-library`,
 * store `config` — handles are structured-cloneable, which is the only
 * way to persist one. All IndexedDB access is guarded the way
 * `recents.ts` guards it: an environment without working IndexedDB
 * degrades to browser-storage mode with nothing persisted.
 *
 * Cross-tab change notification rides a BroadcastChannel — the browser
 * counterpart of the desktop shell's app-wide 'library-changed' event.
 * BroadcastChannel does NOT deliver to the posting context, so mutations
 * notify local listeners directly as well.
 */

import type { LibraryFileEntry } from '../library/types'
import type { LibraryStore } from './libraryStore'

/** The three category subfolders, identical to the desktop store's. */
const CATEGORY_DIRS = ['Components', 'Materials', 'Models'] as const

const THUMB_DIR = '.thumbnails'

/** OPFS directory holding the browser-storage library. */
const OPFS_LIBRARY_DIR = 'library'

const DB_NAME = 'hew-library'
const CONFIG_STORE = 'config'
const KEY_MODE = 'mode'
const KEY_FOLDER_HANDLE = 'folderHandle'

const BROADCAST_CHANNEL = 'hew-library'

type Mode = 'browser' | 'folder'

/** True when this browser can host a library at all. Sync on purpose — the
 * facade's `available()` is sync. */
export function webLibrarySupported(): boolean {
  return (
    typeof navigator !== 'undefined' &&
    typeof navigator.storage !== 'undefined' &&
    typeof navigator.storage.getDirectory === 'function'
  )
}

/**
 * Validate a relative item name, mirroring the desktop shell's rule
 * (main.rs `library_*` commands): at most one subfolder segment, and only
 * one of the three category names in that position; every segment
 * non-empty, no `..`, no backslashes. Throws on violation — same posture
 * as the shell, which rejects rather than sanitizes.
 */
function validateItemName(name: string): { dir: string | null; base: string } {
  if (name.includes('\\') || name.includes('..')) {
    throw new Error(`invalid library item name: ${name}`)
  }
  const segments = name.split('/')
  if (segments.some((s) => s === '')) {
    throw new Error(`invalid library item name: ${name}`)
  }
  if (segments.length === 1) return { dir: null, base: segments[0] }
  if (segments.length === 2 && (CATEGORY_DIRS as readonly string[]).includes(segments[0])) {
    return { dir: segments[0], base: segments[1] }
  }
  throw new Error(`invalid library item name: ${name}`)
}

/** Thumbnail keys are content hashes — hex only, no path characters. */
function validateThumbKey(key: string): void {
  if (!/^[0-9a-f]{8,128}$/.test(key)) {
    throw new Error(`invalid thumbnail key: ${key}`)
  }
}

// ---------------------------------------------------------------------------
// IndexedDB config (mode + persisted folder handle), guarded throughout.
// ---------------------------------------------------------------------------

function safeIndexedDB(): IDBFactory | undefined {
  if (typeof indexedDB === 'undefined') return undefined
  try {
    return indexedDB
  } catch {
    return undefined
  }
}

function openConfigDb(idb: IDBFactory): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = idb.open(DB_NAME, 1)
    req.onupgradeneeded = () => {
      const db = req.result
      if (!db.objectStoreNames.contains(CONFIG_STORE)) {
        db.createObjectStore(CONFIG_STORE)
      }
    }
    req.onsuccess = () => resolve(req.result)
    req.onerror = () => reject(req.error ?? new Error('indexedDB.open failed'))
  })
}

async function configGet<T>(idb: IDBFactory | undefined, key: string): Promise<T | undefined> {
  if (!idb) return undefined
  try {
    const db = await openConfigDb(idb)
    try {
      return await new Promise<T | undefined>((resolve, reject) => {
        const req = db.transaction(CONFIG_STORE, 'readonly').objectStore(CONFIG_STORE).get(key)
        req.onsuccess = () => resolve(req.result as T | undefined)
        req.onerror = () => reject(req.error ?? new Error('get failed'))
      })
    } finally {
      db.close()
    }
  } catch {
    return undefined
  }
}

async function configSet(idb: IDBFactory | undefined, key: string, value: unknown): Promise<void> {
  if (!idb) return
  try {
    const db = await openConfigDb(idb)
    try {
      await new Promise<void>((resolve, reject) => {
        const tx = db.transaction(CONFIG_STORE, 'readwrite')
        tx.objectStore(CONFIG_STORE).put(value, key)
        tx.oncomplete = () => resolve()
        tx.onerror = () => reject(tx.error ?? new Error('put failed'))
      })
    } finally {
      db.close()
    }
  } catch {
    /* best effort — an unpersisted mode just means defaults next launch */
  }
}

// ---------------------------------------------------------------------------
// Directory-handle helpers (shared by both roots).
// ---------------------------------------------------------------------------

async function subdir(
  root: FileSystemDirectoryHandle,
  name: string,
  create: boolean,
): Promise<FileSystemDirectoryHandle | null> {
  try {
    return await root.getDirectoryHandle(name, { create })
  } catch {
    return null
  }
}

async function readFileIn(dir: FileSystemDirectoryHandle, base: string): Promise<Uint8Array> {
  const handle = await dir.getFileHandle(base)
  const file = await handle.getFile()
  return new Uint8Array(await file.arrayBuffer())
}

async function writeFileIn(
  dir: FileSystemDirectoryHandle,
  base: string,
  bytes: Uint8Array,
): Promise<void> {
  const handle = await dir.getFileHandle(base, { create: true })
  const writable = await handle.createWritable()
  // Copy into a fresh ArrayBuffer-backed view: `bytes` may be a view into a
  // larger (or SharedArrayBuffer-backed, on some wasm paths) buffer, which
  // FileSystemWritableFileStream.write would either over-write or reject.
  await writable.write(new Uint8Array(bytes).buffer as ArrayBuffer)
  await writable.close()
}

/** An entry that is already gone — DOMException NotFoundError, however the
 * environment spells it. */
function isNotFound(err: unknown): boolean {
  return (
    (typeof DOMException !== 'undefined' && err instanceof DOMException && err.name === 'NotFoundError') ||
    (err instanceof Error && err.message.includes('NotFound'))
  )
}

/** List the `.hew` files directly inside `dir`, names prefixed with
 * `prefix` (`''` or `'Components/'` …). */
async function listHewIn(
  dir: FileSystemDirectoryHandle,
  prefix: string,
): Promise<LibraryFileEntry[]> {
  const out: LibraryFileEntry[] = []
  for await (const [name, handle] of dir.entries()) {
    if (handle.kind !== 'file' || !name.toLowerCase().endsWith('.hew')) continue
    try {
      const file = await (handle as unknown as FileSystemFileHandle).getFile()
      out.push({ fileName: prefix + name, size: file.size, mtimeMs: file.lastModified })
    } catch {
      /* a file that vanished mid-listing is simply skipped */
    }
  }
  return out
}

// ---------------------------------------------------------------------------
// The store.
// ---------------------------------------------------------------------------

/** Injectable dependencies — production defaults; tests hand in fakes. */
export interface WebLibraryDeps {
  idb?: IDBFactory
  /** OPFS root provider (production: `navigator.storage.getDirectory()`). */
  opfsRoot?: () => Promise<FileSystemDirectoryHandle>
  /** Folder picker (production: `showDirectoryPicker`), or null when the
   * browser has none — capabilities().canChooseFolder follows this. */
  pickDirectory?: (() => Promise<FileSystemDirectoryHandle>) | null
  /** Best-effort durable-storage request (production:
   * `navigator.storage.persist()`). */
  requestPersist?: () => Promise<boolean>
  /** BroadcastChannel factory, or null where unsupported. */
  channel?: BroadcastChannel | null
}

type ResolvedDeps = Omit<Required<WebLibraryDeps>, 'idb'> & { idb: IDBFactory | undefined }

function defaultDeps(): ResolvedDeps {
  return {
    idb: safeIndexedDB(),
    opfsRoot: () => navigator.storage.getDirectory(),
    pickDirectory:
      typeof showDirectoryPicker === 'function'
        ? () => showDirectoryPicker({ id: 'hew-library', mode: 'readwrite' })
        : null,
    requestPersist: () =>
      typeof navigator.storage.persist === 'function'
        ? navigator.storage.persist()
        : Promise.resolve(false),
    channel:
      typeof BroadcastChannel !== 'undefined' ? new BroadcastChannel(BROADCAST_CHANNEL) : null,
  }
}

export function makeWebLibraryStore(overrides: WebLibraryDeps = {}): LibraryStore {
  const deps = { ...defaultDeps(), ...overrides }

  // --- mode + folder handle (lazily loaded from config, cached) ---------
  let mode: Mode = 'browser'
  let folderHandle: FileSystemDirectoryHandle | null = null
  let needsReconnect = false
  let configLoaded: Promise<void> | null = null

  function loadConfig(): Promise<void> {
    if (!configLoaded) {
      configLoaded = (async () => {
        const [storedMode, storedHandle] = await Promise.all([
          configGet<Mode>(deps.idb, KEY_MODE),
          configGet<FileSystemDirectoryHandle>(deps.idb, KEY_FOLDER_HANDLE),
        ])
        // Derive BOTH ways from what is stored — a reload after another tab
        // switched folder→browser must land on browser, not keep the stale
        // in-memory folder mode.
        if (storedMode === 'folder' && storedHandle) {
          mode = 'folder'
          folderHandle = storedHandle
        } else {
          mode = 'browser'
          folderHandle = storedHandle ?? null
        }
      })()
    }
    return configLoaded
  }

  // --- change notification ----------------------------------------------
  const listeners = new Set<() => void>()
  const notifyLocal = (): void => listeners.forEach((l) => l())
  // A message from another tab may mean a CONTENT change or a ROOT change
  // (folder bound/unbound there) — drop the memoized config so the next
  // operation re-reads mode + handle from IndexedDB, then re-render. One
  // extra IndexedDB get per external change is the price of two windows
  // never silently writing to different roots.
  deps.channel?.addEventListener?.('message', () => {
    configLoaded = null
    notifyLocal()
  })
  const notifyAll = (): void => {
    notifyLocal()
    try {
      deps.channel?.postMessage('changed')
    } catch {
      /* a closed channel must never break a write */
    }
  }

  // --- durable-storage request: once, after the first successful write ---
  let persistRequested = false
  const requestPersistOnce = (): void => {
    if (persistRequested) return
    persistRequested = true
    void deps.requestPersist().catch(() => {})
  }

  /** Permission on the bound folder. Tries a silent re-grant first (covers
   * Chromium's "allow on every visit"), then an explicit request — which
   * succeeds only while user activation is live (e.g. right after the
   * click that opened the Library). Otherwise flags `needsReconnect` for
   * the dialog's Reconnect button. */
  async function ensureFolderPermission(handle: FileSystemDirectoryHandle): Promise<boolean> {
    const h = handle as unknown as {
      queryPermission?: (d: { mode: string }) => Promise<string>
      requestPermission?: (d: { mode: string }) => Promise<string>
    }
    try {
      const state = (await h.queryPermission?.({ mode: 'readwrite' })) ?? 'granted'
      if (state === 'granted') {
        needsReconnect = false
        return true
      }
      const requested = await h.requestPermission?.({ mode: 'readwrite' })
      if (requested === 'granted') {
        needsReconnect = false
        return true
      }
    } catch {
      /* fall through to needsReconnect */
    }
    needsReconnect = true
    return false
  }

  /** The current root, or null when the bound folder needs a re-grant. */
  async function root(): Promise<FileSystemDirectoryHandle | null> {
    await loadConfig()
    if (mode === 'folder' && folderHandle) {
      if (!(await ensureFolderPermission(folderHandle))) return null
      return folderHandle
    }
    const opfs = await deps.opfsRoot()
    return opfs.getDirectoryHandle(OPFS_LIBRARY_DIR, { create: true })
  }

  /** Root that throws (for mutations, where silence would lose data). */
  async function rootOrThrow(): Promise<FileSystemDirectoryHandle> {
    const r = await root()
    if (r === null) {
      throw new Error('The library folder needs permission again — reconnect it in the Library window.')
    }
    return r
  }

  async function dirFor(
    r: FileSystemDirectoryHandle,
    name: string,
    create: boolean,
  ): Promise<{ dir: FileSystemDirectoryHandle; base: string } | null> {
    const { dir, base } = validateItemName(name)
    if (dir === null) return { dir: r, base }
    const d = await subdir(r, dir, create)
    return d === null ? null : { dir: d, base }
  }

  async function listAll(r: FileSystemDirectoryHandle): Promise<LibraryFileEntry[]> {
    const out = await listHewIn(r, '')
    for (const cat of CATEGORY_DIRS) {
      const d = await subdir(r, cat, false)
      if (d !== null) out.push(...(await listHewIn(d, `${cat}/`)))
    }
    return out
  }

  /** Copy every item (and cached thumbnail) absent from `target` — the
   * one-way migration run when the user binds a real folder. The source is
   * the CURRENT root (browser storage, or the previously-bound folder), so
   * re-binding carries the library along instead of stranding it in the
   * old folder. Existing files in the target always win; nothing is
   * overwritten. Per-item failures are logged, never silent-fatal — the
   * source copy stays in place either way. */
  async function migrateInto(target: FileSystemDirectoryHandle): Promise<void> {
    try {
      const source = await root()
      if (source === null) return // old folder lost permission — leave it be
      const existing = new Set((await listAll(target)).map((e) => e.fileName))
      for (const entry of await listAll(source)) {
        if (existing.has(entry.fileName)) continue
        const from = await dirFor(source, entry.fileName, false)
        const to = await dirFor(target, entry.fileName, true)
        if (from === null || to === null) continue
        try {
          await writeFileIn(to.dir, to.base, await readFileIn(from.dir, from.base))
        } catch (err) {
          // Best effort per item — the source copy remains — but never
          // silent: the console names exactly what was left behind.
          console.warn(`hew library: could not copy ${entry.fileName} to the new folder`, err)
        }
      }
      const srcThumbs = await subdir(source, THUMB_DIR, false)
      if (srcThumbs !== null) {
        const dstThumbs = await subdir(target, THUMB_DIR, true)
        if (dstThumbs !== null) {
          for await (const [name, handle] of srcThumbs.entries()) {
            if (handle.kind !== 'file') continue
            try {
              await dstThumbs.getFileHandle(name)
            } catch {
              try {
                await writeFileIn(
                  dstThumbs,
                  name,
                  await readFileIn(srcThumbs, name),
                )
              } catch {
                /* thumbnails are cache — losing one costs a re-render */
              }
            }
          }
        }
      }
    } catch {
      /* no source library yet — nothing to migrate */
    }
  }

  return {
    available: () => true,

    folderInfo: async () => {
      await loadConfig()
      // A display label, not a filesystem path — the browser has no
      // user-visible absolute path in either mode (LibraryStore.folderInfo).
      if (mode === 'folder' && folderHandle) return { path: folderHandle.name }
      return { path: 'Browser storage' }
    },

    chooseFolder: async () => {
      if (deps.pickDirectory === null) return null
      let picked: FileSystemDirectoryHandle
      try {
        picked = await deps.pickDirectory()
      } catch {
        return null // cancelled
      }
      await loadConfig()
      await migrateInto(picked)
      mode = 'folder'
      folderHandle = picked
      needsReconnect = false
      await configSet(deps.idb, KEY_MODE, 'folder')
      await configSet(deps.idb, KEY_FOLDER_HANDLE, picked)
      notifyAll()
      return picked.name
    },

    list: async () => {
      const r = await root()
      if (r === null) return [] // needsReconnect is set; the dialog shows Reconnect
      return listAll(r)
    },

    read: async (fileName) => {
      const r = await rootOrThrow()
      const loc = await dirFor(r, fileName, false)
      if (loc === null) throw new Error(`no such library item: ${fileName}`)
      return readFileIn(loc.dir, loc.base)
    },

    write: async (fileName, bytes) => {
      const r = await rootOrThrow()
      const loc = await dirFor(r, fileName, true)
      if (loc === null) throw new Error(`cannot create folder for: ${fileName}`)
      await writeFileIn(loc.dir, loc.base, bytes)
      requestPersistOnce()
      notifyAll()
    },

    remove: async (fileName) => {
      const r = await rootOrThrow()
      const loc = await dirFor(r, fileName, false)
      // Deleting an already-absent file is not an error — the caller's goal
      // ("this name is gone") is already satisfied. Same posture as the
      // desktop shell's library_delete (main.rs).
      if (loc === null) return
      try {
        await loc.dir.removeEntry(loc.base)
      } catch (err) {
        if (!isNotFound(err)) throw err
        return
      }
      notifyAll()
    },

    readThumbnail: async (key) => {
      validateThumbKey(key)
      const r = await root()
      if (r === null) return null
      const thumbs = await subdir(r, THUMB_DIR, false)
      if (thumbs === null) return null
      try {
        return await readFileIn(thumbs, `${key}.png`)
      } catch {
        return null // absent = cache miss, never an error
      }
    },

    writeThumbnail: async (key, png) => {
      validateThumbKey(key)
      const r = await rootOrThrow()
      const thumbs = await subdir(r, THUMB_DIR, true)
      if (thumbs === null) throw new Error('cannot create thumbnail cache')
      await writeFileIn(thumbs, `${key}.png`, png)
    },

    reveal: () => Promise.reject(new Error('reveal is not available in the browser')),

    itemPath: () => Promise.resolve(null),

    capabilities: () => ({
      canReveal: false,
      canChooseFolder: deps.pickDirectory !== null,
      canDownload: true,
    }),

    webStorage: async () => {
      await loadConfig()
      // list() is what actually probes permission; report the flag it set.
      return { mode, needsReconnect }
    },

    reconnect: async () => {
      await loadConfig()
      if (mode !== 'folder' || folderHandle === null) return true
      const ok = await ensureFolderPermission(folderHandle)
      if (ok) notifyAll()
      return ok
    },

    useBrowserStorage: async () => {
      await loadConfig()
      mode = 'browser'
      needsReconnect = false
      await configSet(deps.idb, KEY_MODE, 'browser')
      notifyAll()
    },

    subscribe: (l) => {
      listeners.add(l)
      return () => listeners.delete(l)
    },
  }
}
