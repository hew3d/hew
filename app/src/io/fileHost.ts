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

/** Result of a successful import (mirrors the kernel ImportReport). */
export interface ImportReport {
  objects_created: number
  watertight: number
  leaky: number
  skipped: Array<{ name: string; reason: string }>
  textures_missing: string[]
  /** Parser recovery notes — only the SketchUp (.skp) importer populates this today. */
  warnings: string[]
}

/**
 * A successfully-picked import file. `kind` selects the kernel importer:
 * COLLADA carries host-resolved `images`; glTF and SketchUp embed their own
 * resources, so they carry only bytes. STL carries neither resources nor
 * units — the caller (App.tsx) prompts for a unit scale before dispatching
 * to `scene.import_stl`.
 */
export type ImportPick =
  | { kind: 'dae'; name: string; bytes: Uint8Array; images: Record<string, ImageEntry> }
  | { kind: 'gltf'; name: string; bytes: Uint8Array }
  | { kind: 'skp'; name: string; bytes: Uint8Array }
  | { kind: 'stl'; name: string; bytes: Uint8Array }

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
   * Prompt the user to choose a model file to import — COLLADA (`.dae`),
   * SketchUp 2017 (`.skp`), glTF (`.glb` / `.gltf`), or STL (`.stl`) — the
   * format is chosen by the file the user picks (the dialog offers a filter
   * for each). Returns null if the user cancels.
   *
   * The returned `kind` tells the caller which kernel importer to run. For
   * COLLADA, `images` maps each referenced image URI to its encoded bytes +
   * format (missing images are reported by the kernel ImportReport, not an
   * error here); glTF, SketchUp, and STL all embed their own resources
   * (SketchUp files embed their textures; STL has none), so they carry only
   * the bytes. STL additionally carries no units — the caller prompts for a
   * unit scale before calling `scene.import_stl`.
   *
   * `name` is the display name (basename) of the chosen file, used by the
   * importing overlay to show "Importing "<name>"…" while the parse runs.
   */
  openForImport(): Promise<ImportPick | null>

  /**
   * Write arbitrary bytes out to a user-chosen location (e.g. a `.glb`).
   *
   * Unlike save/saveAs this is a one-shot "write a copy out" — it never tracks
   * a handle for in-place re-save and carries its own file-type filter.
   * Returns true on success, false if the user cancels.
   */
  exportBinary(
    bytes: Uint8Array,
    suggestedName: string,
    fileType: ExportFileType,
  ): Promise<boolean>
}

/** Describes the file type offered in an export dialog. */
export interface ExportFileType {
  /** Human label for the dialog filter, e.g. "glTF Binary". */
  description: string
  /** File extension without the dot, e.g. "glb". */
  ext: string
  /** MIME type, e.g. "model/gltf-binary". */
  mime: string
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
      exportBinary: (bytes, name, fileType) =>
        hostPromise.then((h) => h.exportBinary(bytes, name, fileType)),
    }
  }
  return new WebFileHost()
}
