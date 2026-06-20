/**
 * FileHost seam — abstracts the platform's open/save file dialogs.
 *
 * WebFileHost  (this slice): FSAA when available, anchor-download fallback.
 * TauriFileHost (future slice): will call Tauri dialog commands.
 *
 * The `handle` field inside FileRef is opaque per-host.  WebFileHost stores
 * the FileSystemFileHandle there so subsequent saves can overwrite in place
 * without re-prompting the user.
 */

export interface FileRef {
  /** Display name (filename without directory). */
  name: string
  /**
   * Opaque per-host handle for in-place Save.
   * WebFileHost: FileSystemFileHandle | null (null = fallback mode).
   * TauriFileHost (future): absolute file path string.
   */
  handle: unknown
}

/** Image payload for one texture referenced by a COLLADA file. */
export interface ImageEntry {
  bytes: Uint8Array
  format: 'png' | 'jpeg'
}

/** Result of a successful COLLADA import (mirrors the kernel ImportReport). */
export interface ImportReport {
  objects_created: number
  watertight: number
  leaky: number
  skipped: Array<{ name: string; reason: string }>
  textures_missing: string[]
}

export interface FileHost {
  /**
   * Prompt the user to choose a .hew file to open.
   * Returns null if the user cancels.
   */
  open(): Promise<{ ref: FileRef; bytes: Uint8Array } | null>

  /**
   * Save bytes to the file identified by `ref` (in-place overwrite if the
   * host supports it), or prompt for a new location when `ref` is null.
   * Returns the (possibly updated) FileRef on success, or null on cancel.
   */
  save(bytes: Uint8Array, ref: FileRef | null): Promise<FileRef | null>

  /**
   * Always prompt the user for a save location ("Save As").
   * Returns the new FileRef on success, or null on cancel.
   */
  saveAs(bytes: Uint8Array, suggestedName: string): Promise<FileRef | null>

  /**
   * Prompt the user to choose a .dae (COLLADA) file to import.
   * Also attempts to resolve texture images referenced inside the file.
   * Returns null if the user cancels.
   *
   * The `images` record maps each image URI (as the COLLADA file would
   * reference it) to its encoded bytes + format.  Missing images are
   * reported by the kernel ImportReport — they are not a failure here.
   *
   * `name` is the display name (basename) of the chosen file, used by the
   * importing overlay to show "Importing "<name>"…" while the parse runs.
   */
  openForImport(): Promise<{
    daeBytes: Uint8Array
    images: Record<string, ImageEntry>
    /** Display name (basename) of the chosen .dae file. */
    name: string
  } | null>
}

// WebFileHost is statically imported — it's always needed for the web build.
import { WebFileHost } from './webFileHost'

/**
 * True when running inside the Tauri desktop shell.
 * The Tauri runtime injects `window.__TAURI_INTERNALS__` before the webview loads.
 */
export const isTauri: boolean =
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/**
 * Return the appropriate FileHost for the current platform.
 *
 * Under Tauri, TauriFileHost is imported dynamically (lazy) so its Tauri-only
 * imports never appear in the web bundle.  The web path always gets WebFileHost.
 */
export function makeFileHost(): FileHost {
  if (isTauri) {
    // Dynamic import so the web bundle never pulls in TauriFileHost or its
    // Tauri dependencies.  makeFileHost() is called once (in a useRef) so
    // we return a synchronous shim that defers to TauriFileHost once loaded.
    const hostPromise = import('./tauriFileHost').then(
      (m) => new m.TauriFileHost(),
    )
    return {
      open: () => hostPromise.then((h) => h.open()),
      save: (bytes, ref) => hostPromise.then((h) => h.save(bytes, ref)),
      saveAs: (bytes, name) => hostPromise.then((h) => h.saveAs(bytes, name)),
      openForImport: () => hostPromise.then((h) => h.openForImport()),
    }
  }
  return new WebFileHost()
}
