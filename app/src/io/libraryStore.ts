// The library storage seam: a folder of `.hew` files plus a thumbnail
// cache, behind one interface with per-platform implementations — the same
// triple-file pattern as `fileHost.ts` / `recoveryStore.ts`. This module is
// platform-neutral: the Tauri implementation loads lazily so the web bundle
// never pulls Tauri code.

import type { LibraryFileEntry } from '../library/types'
import { isTauri } from './fileHost'

/** Storage for the library folder. File names are relative names inside
 * the folder — either bare (a legacy flat item) or one category subfolder
 * down (`Components/`, `Materials/`, `Models/`, e.g.
 * `Components/chair-3f2a.hew`) — always treated as an opaque id by callers;
 * implementations MUST reject names with more than one subfolder segment,
 * any segment other than the three category names for that position, or
 * `..`. */
export interface LibraryStore {
  /** Whether this platform has a working library backend. The browser
   * build returns false until the OPFS store lands — the UI shows an
   * honest "not available in the browser yet" state, never a broken one. */
  available(): boolean
  /** The current library folder: `path` for real filesystems (shown in the
   * modal footer, e.g. `~/Hew Library`), null on backends with no
   * user-visible path. */
  folderInfo(): Promise<{ path: string | null }>
  /** Open a native folder picker and persist the choice. Resolves to the
   * new path, or null if cancelled/unsupported. */
  chooseFolder(): Promise<string | null>
  /** List the folder's `.hew` files, root plus all three category
   * subfolders (thumbnail cache excluded). */
  list(): Promise<LibraryFileEntry[]>
  read(fileName: string): Promise<Uint8Array>
  write(fileName: string, bytes: Uint8Array): Promise<void>
  /** Delete an item file (and its cached thumbnail is the caller's job —
   * thumbnails are keyed by content hash, so orphans are just cache). */
  remove(fileName: string): Promise<void>
  /** Cached thumbnail PNG by content-hash key, or null when absent. */
  readThumbnail(key: string): Promise<Uint8Array | null>
  writeThumbnail(key: string, png: Uint8Array): Promise<void>
  /** Reveal the item in the OS file manager (desktop only; rejects
   * elsewhere — gate on `capabilities().canReveal`). */
  reveal(fileName: string): Promise<void>
  /** The item's absolute filesystem path, or `null` on a backend with no
   * real path (the web store, or any failure resolving it) — the Library ▸
   * Open as Document flow's non-pristine-window case needs a real path to
   * hand `open_in_new_window` (see `App.tsx`'s `onOpenAsDocument`). */
  itemPath(fileName: string): Promise<string | null>
  /** What the current backend can do, for UI gating. */
  capabilities(): { canReveal: boolean; canChooseFolder: boolean }
  /** Subscribe to library-folder location changes (Settings, other
   * windows). Returns an unsubscribe. */
  subscribe(listener: () => void): () => void
}

/** A store that reports unavailable and rejects every operation — the web
 * build's stand-in until an OPFS-backed store lands, and the safe default
 * while the Tauri implementation loads. */
export function unavailableLibraryStore(reason: string): LibraryStore {
  const fail = (): Promise<never> => Promise.reject(new Error(reason))
  return {
    available: () => false,
    folderInfo: () => Promise.resolve({ path: null }),
    chooseFolder: () => Promise.resolve(null),
    list: () => Promise.resolve([]),
    read: fail,
    write: fail,
    remove: fail,
    readThumbnail: () => Promise.resolve(null),
    writeThumbnail: fail,
    reveal: fail,
    itemPath: () => Promise.resolve(null),
    capabilities: () => ({ canReveal: false, canChooseFolder: false }),
    subscribe: () => () => {},
  }
}

let cached: LibraryStore | null = null

/** The platform's library store. Synchronous facade over a lazily-imported
 * backend (the `makeFileHost` pattern): callers get a stable object
 * immediately; operations await the backend underneath. */
export function libraryStore(): LibraryStore {
  if (cached) return cached
  if (!isTauri) {
    cached = unavailableLibraryStore(
      'The library is not available in the browser build yet.',
    )
    return cached
  }
  let backend: LibraryStore | null = null
  const load = async (): Promise<LibraryStore> => {
    if (!backend) {
      const mod = await import('./tauriLibraryStore')
      backend = mod.makeTauriLibraryStore()
    }
    return backend
  }
  const listeners = new Set<() => void>()
  const notify = (): void => listeners.forEach((l) => l())
  void load().then((b) => b.subscribe(notify))
  // Content-change notification rides the SHELL's app-wide
  // 'library-changed' event (emitted by library_write/library_delete and
  // consumed by the backend's subscribe), which reaches this window AND
  // every other one — a facade-level after-write notify here would just
  // double-fire the same listeners (adversarial review S7).
  cached = {
    available: () => true,
    folderInfo: () => load().then((b) => b.folderInfo()),
    chooseFolder: () => load().then((b) => b.chooseFolder()),
    list: () => load().then((b) => b.list()),
    read: (f) => load().then((b) => b.read(f)),
    write: (f, bytes) => load().then((b) => b.write(f, bytes)),
    remove: (f) => load().then((b) => b.remove(f)),
    readThumbnail: (k) => load().then((b) => b.readThumbnail(k)),
    writeThumbnail: (k, png) => load().then((b) => b.writeThumbnail(k, png)),
    reveal: (f) => load().then((b) => b.reveal(f)),
    itemPath: (f) => load().then((b) => b.itemPath(f)),
    capabilities: () => ({ canReveal: true, canChooseFolder: true }),
    subscribe: (l) => {
      listeners.add(l)
      return () => listeners.delete(l)
    },
  }
  return cached
}
