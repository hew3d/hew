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

import type { HealthCounts } from '../health/healthReport'
import type { SkpStreamFormat } from './docFormat'

/**
 * A file the user picked whose bytes could not be read (its permission
 * withdrawn, or the file moved or changed while it was read). Distinct
 * from a cancel, which a picker answers with null, so the caller can say
 * so instead of doing nothing.
 */
export class FileReadError extends Error {
  readonly fileName: string
  readonly reason: unknown
  constructor(fileName: string, reason: unknown) {
    // oxlint-disable-next-line lingui/no-unlocalized-strings -- log text; the toast wording is chosen by the caller
    super(`Couldn't read ${fileName}`)
    this.name = 'FileReadError'
    this.fileName = fileName
    this.reason = reason
  }
}

/**
 * The on-disk format a document is bound to. `hew` is the native format;
 * `skp2026` and the stream releases (`SKP_STREAM_FORMATS`) are the SketchUp
 * formats a `.skp` document is saved back in (the format of the file that
 * was opened, or the one Save As picked). A persisted ref that predates
 * this field is `hew`.
 */
export type DocFormat = 'hew' | 'skp2026' | SkpStreamFormat

export interface FileRef {
  /** Display name (filename without directory). */
  name: string
  /** The format Save writes this file in. */
  format: DocFormat
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
  /** Drawing segments that came in: lines bounding no face in the source. */
  lines_added: number
  watertight: number
  /** Open objects, surfaces included — kept for API compatibility; the
   *  report's wording comes from `health`. */
  leaky: number
  /** Per-verdict counts (Model Health): solids, surfaces (flat / bent
   *  sheets) and the open objects that need attention. */
  health: HealthCounts
  skipped: Array<{ name: string; reason: string }>
  textures_missing: string[]
  /** Parser recovery notes — only the SketchUp (.skp) importer populates this today. */
  warnings: string[]
  /** A `.skp` that looks incomplete or damaged (cut short, most often) but
   *  of which something was read: parts of the model may be missing. Set
   *  only by the `.skp` open and import. */
  incomplete?: boolean
}

/**
 * A successfully-picked import file. `kind` selects the kernel importer:
 * COLLADA carries host-resolved `images`; glTF and SketchUp embed their own
 * resources, so they carry only bytes. STL carries neither resources nor
 * units — the caller (App.tsx) prompts for a unit scale before dispatching
 * to `scene.import_stl`.
 *
 * `path` is the absolute filesystem path the pick came from, when the host
 * can supply one (TauriFileHost always can; WebFileHost never does — the
 * File System Access API has no real fs path). Used by App.tsx's File ▸
 * Open to route a pick onto a non-pristine window into a fresh one
 * (`open_in_new_window`), which needs a path rather than in-memory bytes.
 */
export type ImportPick =
  | { kind: 'dae'; name: string; bytes: Uint8Array; images: Record<string, ImageEntry>; path?: string }
  | { kind: 'gltf'; name: string; bytes: Uint8Array; path?: string }
  | { kind: 'skp'; name: string; bytes: Uint8Array; path?: string; handle?: unknown }
  | { kind: 'stl'; name: string; bytes: Uint8Array; path?: string }

/**
 * A file picked through the unified Open dialog (`openAny`). `kind: 'hew'`
 * is the native format — `handle` is the same opaque per-host handle
 * `FileRef.handle` carries, so a hew pick can be wrapped straight into a
 * `FileRef` for `afterOpen`/subsequent `save()`. Every other kind is an
 * `ImportPick`, routed through the same post-pick steps (STL's units
 * prompt, the blank-then-import replace, the import report) that
 * `openForImport()` already feeds. A `kind: 'skp'` pick from `openAny` also
 * carries the per-host `handle` (when the host has one), because Open makes
 * the `.skp` the document's own file; File ▸ Import ignores it.
 */
export type OpenPick = { kind: 'hew'; name: string; bytes: Uint8Array; handle: unknown } | ImportPick

export interface FileHost {
  /**
   * Prompt the user to choose a .hew file to open.
   * Returns null if the user cancels.
   */
  open(): Promise<{ ref: FileRef; bytes: Uint8Array } | null>

  /**
   * Save bytes to the file identified by `ref` (in-place overwrite if the
   * host supports it), or prompt for a new location when `ref` is null. The
   * bytes are in `ref.format`; a prompt (null ref, denied permission, no
   * in-place support) uses that format's filter and extension.
   * Returns the (possibly updated) FileRef on success, or null on cancel.
   */
  save(bytes: Uint8Array, ref: FileRef | null): Promise<FileRef | null>

  /**
   * Always prompt the user for a save location ("Save As").
   * `bytes` are in `format` (default `hew`); the picker's filter and the
   * forced extension follow it. Returns the new FileRef on success, or null
   * on cancel.
   */
  saveAs(bytes: Uint8Array, suggestedName: string, format?: DocFormat): Promise<FileRef | null>

  /**
   * Prompt the user to choose a model file to import — COLLADA (`.dae`),
   * SketchUp (`.skp`, the 2026 container or a 2017 to 2020 release), glTF (`.glb` /
   * `.gltf`), or STL (`.stl`) — the
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
   * Prompt the user with ONE dialog that accepts every format Hew can open:
   * the native `.hew` format plus every import format `openForImport`
   * accepts (`.dae`, `.skp`, `.glb`/`.gltf`, `.stl`) — the dialog offers a
   * filter for each, plus one covering all of them together. The format is
   * chosen by the file the user picks; the returned `kind` tells the caller
   * whether to treat it as a native open (`'hew'`, applied directly) or an
   * import (everything else, routed through the same post-pick steps
   * `openForImport()`'s result feeds — the STL units prompt, the
   * blank-then-import replace, the import report). Returns null if the
   * user cancels.
   */
  openAny(): Promise<OpenPick | null>

  /**
   * Prompt the user to choose a picture (PNG or JPEG) to place in the
   * current document — File ▸ Import Image…. Returns the file's base name
   * and bytes, or null if the user cancels. Unlike `openForImport`, nothing
   * about the document is replaced: the caller arms Place Image with it.
   * A picked file whose bytes can't be read rejects with `FileReadError`
   * (never null: that would read as a cancel). Optional only so test
   * doubles may leave it out; both shipped hosts implement it.
   */
  openImage?(): Promise<{ name: string; bytes: Uint8Array } | null>

  /**
   * Write `bytes` to a path the native save panel (`io/savePanel.ts`)
   * already picked and approved — no dialog. A `.hew` goes through the
   * same writer `save` uses. Desktop only: the browser has no such panel,
   * so `WebFileHost` leaves it out.
   */
  writeTo?(path: string, bytes: Uint8Array, format?: DocFormat): Promise<FileRef>

  /**
   * Write arbitrary bytes out to a user-chosen location (e.g. a `.glb`).
   *
   * Unlike save/saveAs this is a one-shot "write a copy out" — it never tracks
   * a handle for in-place re-save and carries its own file-type filter.
   * Returns where it went on success — the absolute path on desktop, the
   * file name on web (there is no real filesystem path to report: the File
   * System Access API's save picker and the anchor-download fallback both
   * hand control to the browser/OS once the write starts) — or null if the
   * user cancels the picker.
   */
  exportBinary(
    bytes: Uint8Array,
    suggestedName: string,
    fileType: ExportFileType,
  ): Promise<string | null>
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
      saveAs: (bytes, name, format) => hostPromise.then((h) => h.saveAs(bytes, name, format)),
      openForImport: () => hostPromise.then((h) => h.openForImport()),
      openAny: () => hostPromise.then((h) => h.openAny()),
      openImage: () => hostPromise.then((h) => h.openImage()),
      exportBinary: (bytes, name, fileType) =>
        hostPromise.then((h) => h.exportBinary(bytes, name, fileType)),
      writeTo: (path, bytes, format) => hostPromise.then((h) => h.writeTo(path, bytes, format)),
    }
  }
  return new WebFileHost()
}
