/**
 * TauriLibraryStore — native desktop implementation of LibraryStore.
 *
 * Backed by seven custom Tauri commands (shells/tauri/src-tauri/src/main.rs):
 * library_get_dir / library_choose_dir persist and read the configured
 * folder (`library.json` in the app config dir, defaulting to `$HOME/Hew
 * Library`); library_list / library_read / library_write / library_delete /
 * library_reveal address `.hew` files inside it — either directly, or one
 * `Components/` / `Materials/` / `Models/` subfolder down, the library
 * folder's type organization (`fileName` carries that subfolder as part of
 * its relative name, e.g. `Components/chair-3f2a.hew`, and is otherwise
 * opaque here); library_thumb_read / library_thumb_write address the
 * `.thumbnails/` cache, which stays flat at the root and is keyed by
 * content hash, never by fileName. Every name/key is validated shell-side —
 * at most one subfolder segment (and only one of the three category names),
 * no `..` — which is what lets these commands skip the `ApprovedPaths`
 * registry `read_file` / `write_file` use: they can only ever touch paths
 * inside the one configured folder.
 *
 * All Tauri imports are DYNAMIC so this module is never bundled into the web
 * build — see libraryStore.ts's lazy `libraryStore()` facade, which is the
 * only caller of `makeTauriLibraryStore`.
 */

import type { LibraryFileEntry } from '../library/types'
import type { LibraryStore } from './libraryStore'

/** Shape of one `library_list` entry — field names match the Rust struct
 * (`LibraryEntry` in main.rs) as-is; Tauri does not camelCase struct fields
 * on the way out, only command argument names on the way in. */
interface LibraryListEntry {
  name: string
  size: number
  mtime_ms: number
}

/** The literal error string `library_thumb_read` rejects with when no
 * cached thumbnail exists for the key — see its doc comment in main.rs. */
const THUMB_NOT_FOUND = 'not_found'

/** True for both a bare-string IPC rejection and one wrapped in an Error
 * (the shape actually observed depends on the Tauri version; check both so
 * a future bump can't silently turn "no thumbnail" into a thrown error). */
function isThumbNotFound(err: unknown): boolean {
  return err === THUMB_NOT_FOUND || (err instanceof Error && err.message === THUMB_NOT_FOUND)
}

export function makeTauriLibraryStore(): LibraryStore {
  const listeners = new Set<() => void>()

  // Cross-window sync: the Rust side broadcasts the Tauri global event
  // 'settings-changed' with payload `{ key: 'libraryFolder' }` whenever
  // library_choose_dir persists a new folder (see main.rs). This is the
  // same event channel app/src/settings/theme.ts and debugMode.ts use for
  // their own settings, each filtering on its own payload key so all the
  // listeners coexist. There is no localStorage-backed 'storage' event
  // counterpart here — the library folder's source of truth is
  // library.json on the Rust side, not a browser storage key.
  import('@tauri-apps/api/event')
    .then(({ listen }) => {
      void listen<{ key?: unknown }>('settings-changed', (event) => {
        if (event.payload?.key === 'libraryFolder') {
          listeners.forEach((l) => l())
        }
      })
      // Content mutations broadcast app-wide from the SHELL (library_write /
      // library_delete emit 'library-changed'), so a save in one window
      // refreshes an open Library window and every other window's palette
      // index — the JS facade's own after-write notify only covers the
      // window that performed the write.
      void listen('library-changed', () => {
        listeners.forEach((l) => l())
      })
    })
    .catch(() => {
      /* ignore — event subscription is best-effort */
    })

  return {
    available: () => true,

    async folderInfo() {
      const { invoke } = await import('@tauri-apps/api/core')
      const path = await invoke<string>('library_get_dir')
      return { path }
    },

    async chooseFolder() {
      const { invoke } = await import('@tauri-apps/api/core')
      return invoke<string | null>('library_choose_dir')
    },

    async list(): Promise<LibraryFileEntry[]> {
      const { invoke } = await import('@tauri-apps/api/core')
      const entries = await invoke<LibraryListEntry[]>('library_list')
      return entries.map((e) => ({ fileName: e.name, size: e.size, mtimeMs: e.mtime_ms }))
    },

    async read(fileName: string): Promise<Uint8Array> {
      const { invoke } = await import('@tauri-apps/api/core')
      // library_read returns a raw IPC response, which resolves to an
      // ArrayBuffer — see read_file's doc comment for why (no JSON
      // round-trip for multi-megabyte models).
      const buf = await invoke<ArrayBuffer>('library_read', { name: fileName })
      return new Uint8Array(buf)
    },

    async write(fileName: string, bytes: Uint8Array): Promise<void> {
      const { invoke } = await import('@tauri-apps/api/core')
      // library_write expects Vec<u8>; Array.from so Tauri IPC sees a JSON array.
      await invoke('library_write', { name: fileName, contents: Array.from(bytes) })
    },

    async remove(fileName: string): Promise<void> {
      const { invoke } = await import('@tauri-apps/api/core')
      await invoke('library_delete', { name: fileName })
    },

    async readThumbnail(key: string): Promise<Uint8Array | null> {
      const { invoke } = await import('@tauri-apps/api/core')
      try {
        const buf = await invoke<ArrayBuffer>('library_thumb_read', { key })
        return new Uint8Array(buf)
      } catch (err) {
        if (isThumbNotFound(err)) return null
        throw err
      }
    },

    async writeThumbnail(key: string, png: Uint8Array): Promise<void> {
      const { invoke } = await import('@tauri-apps/api/core')
      await invoke('library_thumb_write', { key, contents: Array.from(png) })
    },

    async reveal(fileName: string): Promise<void> {
      const { invoke } = await import('@tauri-apps/api/core')
      await invoke('library_reveal', { name: fileName })
    },

    async itemPath(fileName: string): Promise<string | null> {
      const { invoke } = await import('@tauri-apps/api/core')
      try {
        return await invoke<string>('library_item_path', { name: fileName })
      } catch {
        return null
      }
    },

    capabilities: () => ({ canReveal: true, canChooseFolder: true, canDownload: false }),

    subscribe(listener: () => void): () => void {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
  }
}
