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
}

// Lazy import to avoid pulling WebFileHost into Tauri builds once that branch
// exists.  For now, makeFileHost() always returns the web implementation.
import { WebFileHost } from './webFileHost'

/**
 * Return the appropriate FileHost for the current platform.
 *
 * TODO(tauri-slice): add `if ('__TAURI__' in window) return new TauriFileHost()`
 * once the Tauri file-dialog bridge is implemented.
 */
export function makeFileHost(): FileHost {
  // isTauri check stub — the Tauri shell sets window.__TAURI_INTERNALS__
  if ('__TAURI_INTERNALS__' in window) {
    // TODO(tauri-slice): return new TauriFileHost()
    // Fall through to WebFileHost for now.
  }
  return new WebFileHost()
}
