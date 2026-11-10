/**
 * WebFileHost — browser-native implementation of FileHost.
 *
 * Two tiers:
 *
 * 1. File System Access API (Chrome/Edge/Opera, Safari 15.2+):
 *    - open()   → showOpenFilePicker()  → returns bytes + FileSystemFileHandle
 *    - save()   → if ref has a handle, createWritable() + overwrite in place.
 *                 If ref is null, delegates to saveAs().
 *    - saveAs() → showSaveFilePicker()  → write + return new FileRef
 *
 * 2. Fallback (Firefox, older Safari, any env without FSAA):
 *    - open()   → hidden <input type=file> programmatically clicked.
 *    - save()   → anchor-download a Blob (same as the old M3 handleSave).
 *    - saveAs() → same anchor-download.
 *
 * The hidden <input> is created once lazily and appended to document.body.
 * Its `accept` filter is dropped on coarse-pointer (touch) devices — see
 * isCoarsePointer().
 */

import { ph, t } from '@lingui/core/macro'
import type { DocFormat, ExportFileType, FileHost, FileRef, ImageEntry, ImportPick, OpenPick } from './fileHost'
import { FileReadError } from './fileHost'
import { extensionFor, formatDescription, nameForFormat } from './docFormat'
import { isCoarsePointer } from '../platform'

const HEW_FILE_TYPES: FilePickerAcceptType[] = [
  {
    // Getter: resolved in the active locale when the picker opens, not at module load.
    get description() {
      return t`Hew model`
    },
    accept: { 'application/octet-stream': ['.hew'] },
  },
]

/** Save picker filter for a document format: `.hew`, or the SketchUp
 * container's `.skp` (both SketchUp formats write the same extension). */
function fileTypesFor(format: DocFormat): FilePickerAcceptType[] {
  return [
    {
      description: formatDescription(format),
      accept: { 'application/octet-stream': [extensionFor(format)] },
    },
  ]
}

/** The import formats' own accept map (COLLADA/SketchUp/glTF/STL), reused
 * standalone by `openForImport()` and merged with `.hew` by `openAny()`. */
const IMPORT_ACCEPT = {
  'model/vnd.collada+xml': ['.dae'],
  'application/octet-stream': ['.skp'],
  'model/gltf-binary': ['.glb'],
  'model/gltf+json': ['.gltf'],
  'model/stl': ['.stl'],
}

/** Every format `openAny()`'s single dialog accepts: `.hew` plus every
 * import extension. `.hew` shares the `application/octet-stream` key with
 * `.skp` — both are opaque binary formats with no dedicated IANA type, and
 * the File System Access API accepts multiple extensions per key. */
const OPEN_FILE_TYPES: FilePickerAcceptType[] = [
  {
    get description() {
      return t`All supported files`
    },
    accept: { ...IMPORT_ACCEPT, 'application/octet-stream': ['.hew', '.skp'] },
  },
]

/** True when the File System Access API save picker is available. */
function hasFSAA(): boolean {
  return typeof window !== 'undefined' && 'showSaveFilePicker' in window
}

// Why coarse-pointer gates the `accept` filter (see each call site): iOS
// Files greys out any file whose extension maps to no registered UTI, and
// `.hew` (like several import formats) maps to none, so an `accept` list
// there leaves the user staring at their own file, unable to tap it.
// Desktop pickers merely grey non-matching files without blocking the tap,
// so fine-pointer environments keep the narrower list. The check itself is
// platform.ts's shared `isCoarsePointer` — it treats a missing OR throwing
// `matchMedia` (jsdom, older-Safari private mode) as fine-pointer, which is
// the safe default here too: worst case is a filtered picker, never a
// broken one.

/**
 * Ensure readwrite permission on a stored handle.
 *
 * Chrome auto-grants readwrite on picker-acquired handles so queryPermission
 * returns 'granted' immediately (no dialog).  Brave keeps the handle in the
 * 'prompt' state, so we must call requestPermission — which requires a
 * transient user activation.  All callers reach here via a user gesture
 * (⌘S / menu click), satisfying that requirement.
 */
async function ensureWritePermission(handle: FileSystemFileHandle): Promise<boolean> {
  const opts = { mode: 'readwrite' as const }
  if ((await handle.queryPermission(opts)) === 'granted') return true
  if ((await handle.requestPermission(opts)) === 'granted') return true
  return false
}

/** Trigger an anchor-download of bytes as a .hew file. Exported (not just
 *  used internally by `save`/`saveAs` below) for Shop Mode's "Save a copy
 *  (.hew)" overflow-menu entry (docs/design/shop-mode.md §4's durable
 *  "Keep on this phone → Save to Files" path): that flow deliberately wants
 *  a plain download, never the FSAA save-picker `WebFileHost.saveAs` would
 *  reach for on a browser that supports it — Shop Mode's whole posture is
 *  minimal touch chrome, and "Save a copy" is one tap, not a picker. */
export function anchorDownload(bytes: Uint8Array, name: string): void {
  const blob = new Blob([new Uint8Array(bytes)], { type: 'application/octet-stream' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = name.endsWith('.hew') ? name : name + '.hew'
  a.click()
  URL.revokeObjectURL(url)
}

/** Anchor-download of a document's bytes under the name the format implies
 * (`Model.hew` / `Model.skp`) — the no-FSAA fallback of Save and Save As. */
function anchorDownloadDocument(bytes: Uint8Array, name: string, format: DocFormat): string {
  const finalName = nameForFormat(name, format)
  anchorDownloadAs(bytes, finalName, 'application/octet-stream')
  return finalName
}

/** Trigger an anchor-download of bytes with an explicit filename + MIME type. */
export function anchorDownloadAs(bytes: Uint8Array, name: string, mime: string): void {
  const blob = new Blob([new Uint8Array(bytes)], { type: mime })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = name
  a.click()
  URL.revokeObjectURL(url)
}

export class WebFileHost implements FileHost {
  /** Lazily-created hidden <input type=file> for the fallback open path. */
  private _input: HTMLInputElement | null = null

  private getInput(): HTMLInputElement {
    if (this._input === null) {
      const el = document.createElement('input')
      el.type = 'file'
      // Omit `accept` on coarse pointers — see isCoarsePointer().
      if (!isCoarsePointer()) {
        el.accept = '.hew'
      }
      el.style.display = 'none'
      document.body.appendChild(el)
      this._input = el
    }
    return this._input
  }

  async open(): Promise<{ ref: FileRef; bytes: Uint8Array } | null> {
    if (hasFSAA()) {
      let handles: FileSystemFileHandle[]
      try {
        handles = await showOpenFilePicker({
          multiple: false,
          types: HEW_FILE_TYPES,
          excludeAcceptAllOption: false,
        })
      } catch (err) {
        // User cancelled — showOpenFilePicker throws DOMException(AbortError)
        if (err instanceof DOMException && err.name === 'AbortError') return null
        throw err
      }
      const handle = handles[0]
      if (handle === undefined) return null
      const file = await handle.getFile()
      const buf = await file.arrayBuffer()
      return {
        ref: { name: file.name, format: 'hew', handle },
        bytes: new Uint8Array(buf),
      }
    }

    // Fallback: hidden <input type=file>
    return new Promise((resolve) => {
      const input = this.getInput()
      // Reset so the same file can be re-opened.
      input.value = ''

      const onChange = () => {
        input.removeEventListener('change', onChange)
        const file = input.files?.[0]
        if (file == null) {
          resolve(null)
          return
        }
        file.arrayBuffer().then((buf) => {
          resolve({
            ref: { name: file.name, format: 'hew', handle: null },
            bytes: new Uint8Array(buf),
          })
        }).catch(() => resolve(null))
      }

      input.addEventListener('change', onChange)
      input.click()
    })
  }

  async save(bytes: Uint8Array, ref: FileRef | null): Promise<FileRef | null> {
    if (ref === null) {
      // oxlint-disable-next-line lingui/no-unlocalized-strings -- default file name, not copy
      return this.saveAs(bytes, 'Untitled.hew')
    }
    const format = ref.format

    // FSAA in-place overwrite: ref.handle is a FileSystemFileHandle
    if (hasFSAA() && ref.handle instanceof Object && 'createWritable' in (ref.handle as object)) {
      const handle = ref.handle as FileSystemFileHandle
      // Verify readwrite permission before writing.  Chrome auto-grants;
      // Brave keeps the handle at 'prompt' and needs an explicit request.
      // If the user denies, fall through to saveAs so the document isn't lost.
      if (!(await ensureWritePermission(handle))) {
        return this.saveAs(bytes, ref.name, format)
      }
      try {
        const writable = await handle.createWritable()
        await writable.write(new Uint8Array(bytes))
        await writable.close()
        return { name: handle.name, format, handle }
      } catch (err) {
        if (err instanceof DOMException && err.name === 'AbortError') return null
        throw err
      }
    }

    // A SketchUp file with no handle (opened from the recents shelf or a
    // drop without a File System Access handle) has nowhere to write in
    // place: ask where, rather than silently downloading a copy on every
    // Save. (A `.hew` ref without a handle keeps the long-standing
    // anchor-download.)
    if (hasFSAA() && format !== 'hew') {
      return this.saveAs(bytes, ref.name, format)
    }

    // Fallback: anchor-download (no in-place overwrite possible)
    anchorDownloadDocument(bytes, ref.name, format)
    return ref
  }

  async saveAs(bytes: Uint8Array, suggestedName: string, format: DocFormat = 'hew'): Promise<FileRef | null> {
    if (hasFSAA()) {
      let handle: FileSystemFileHandle
      try {
        handle = await showSaveFilePicker({
          suggestedName: nameForFormat(suggestedName, format),
          types: fileTypesFor(format),
          excludeAcceptAllOption: false,
        })
      } catch (err) {
        if (err instanceof DOMException && err.name === 'AbortError') return null
        throw err
      }
      // The chosen format's extension is forced, as the desktop panel does:
      // one format's bytes never land under the other's name. A picker that
      // let the name through without it is asked to rename the file, and
      // the save is refused (nothing written) when it cannot.
      const wanted = extensionFor(format)
      if (!handle.name.toLowerCase().endsWith(wanted)) {
        const renamed = nameForFormat(handle.name, format)
        const movable = handle as FileSystemFileHandle & { move?: (name: string) => Promise<void> }
        if (typeof movable.move !== 'function') {
          throw new Error(t`The file name must end in ${ph({ ext: wanted })}.`)
        }
        await movable.move(renamed)
      }
      const writable = await handle.createWritable()
      await writable.write(new Uint8Array(bytes))
      await writable.close()
      return { name: handle.name, format, handle }
    }

    // Fallback: anchor-download
    // Return a FileRef with no handle — subsequent "Save" will download again.
    const name = anchorDownloadDocument(bytes, suggestedName, format)
    return { name, format, handle: null }
  }

  async exportBinary(
    bytes: Uint8Array,
    suggestedName: string,
    fileType: ExportFileType,
  ): Promise<string | null> {
    const dotExt = '.' + fileType.ext
    const name = suggestedName.endsWith(dotExt) ? suggestedName : suggestedName + dotExt
    if (hasFSAA()) {
      let handle: FileSystemFileHandle
      try {
        handle = await showSaveFilePicker({
          suggestedName: name,
          types: [{ description: fileType.description, accept: { [fileType.mime]: [dotExt] } }],
          excludeAcceptAllOption: false,
        })
      } catch (err) {
        if (err instanceof DOMException && err.name === 'AbortError') return null
        throw err
      }
      const writable = await handle.createWritable()
      await writable.write(new Uint8Array(bytes))
      await writable.close()
      return handle.name
    }

    // Fallback: anchor-download. There is no picker to cancel here, so this
    // branch always "succeeds" — same as before this method reported where
    // the file went, just now returning the name instead of `true`.
    anchorDownloadAs(bytes, name, fileType.mime)
    return name
  }

  async openImage(): Promise<{ name: string; bytes: Uint8Array } | null> {
    if (hasFSAA()) {
      let handles: FileSystemFileHandle[]
      try {
        handles = await showOpenFilePicker({
          multiple: false,
          types: [{ description: t`Pictures (PNG, JPEG)`, accept: { 'image/png': ['.png'], 'image/jpeg': ['.jpg', '.jpeg'] } }],
          excludeAcceptAllOption: false,
        })
      } catch (err) {
        if (err instanceof DOMException && err.name === 'AbortError') return null
        throw err
      }
      const handle = handles[0]
      if (handle === undefined) return null
      try {
        const file = await handle.getFile()
        return { name: file.name, bytes: new Uint8Array(await file.arrayBuffer()) }
      } catch (err) {
        throw new FileReadError(handle.name, err)
      }
    }
    // Fallback: hidden <input type=file> (as openForImport). A read that
    // fails rejects, so it is reported rather than read as a cancel.
    return new Promise((resolve, reject) => {
      const input = document.createElement('input')
      input.type = 'file'
      if (!isCoarsePointer()) input.accept = '.png,.jpg,.jpeg,image/png,image/jpeg'
      input.style.display = 'none'
      document.body.appendChild(input)
      const onChange = () => {
        input.removeEventListener('change', onChange)
        document.body.removeChild(input)
        const file = input.files?.[0]
        if (file == null) {
          resolve(null)
          return
        }
        file.arrayBuffer().then(
          (buf) => resolve({ name: file.name, bytes: new Uint8Array(buf) }),
          (err: unknown) => reject(new FileReadError(file.name, err)),
        )
      }
      input.addEventListener('change', onChange)
      input.click()
    })
  }

  async openForImport(): Promise<ImportPick | null> {
    const FILE_TYPES: FilePickerAcceptType[] = [
      {
        description: t`Model files (COLLADA, SketchUp, glTF, STL)`,
        accept: IMPORT_ACCEPT,
      },
    ]

    let bytes: Uint8Array
    let fileName: string

    if (hasFSAA()) {
      let handles: FileSystemFileHandle[]
      try {
        handles = await showOpenFilePicker({
          multiple: false,
          types: FILE_TYPES,
          excludeAcceptAllOption: false,
        })
      } catch (err) {
        if (err instanceof DOMException && err.name === 'AbortError') return null
        throw err
      }
      const handle = handles[0]
      if (handle === undefined) return null
      const file = await handle.getFile()
      bytes = new Uint8Array(await file.arrayBuffer())
      fileName = file.name
    } else {
      // Fallback: hidden <input type=file>
      const result = await new Promise<{ bytes: Uint8Array; name: string } | null>((resolve) => {
        const input = document.createElement('input')
        input.type = 'file'
        // Omit `accept` on coarse pointers — see isCoarsePointer().
        if (!isCoarsePointer()) {
          input.accept = '.dae,.skp,.glb,.gltf,.stl'
        }
        input.style.display = 'none'
        document.body.appendChild(input)

        const onChange = () => {
          input.removeEventListener('change', onChange)
          document.body.removeChild(input)
          const file = input.files?.[0]
          if (file == null) {
            resolve(null)
            return
          }
          file.arrayBuffer().then((buf) => resolve({ bytes: new Uint8Array(buf), name: file.name })).catch(() => resolve(null))
        }

        input.addEventListener('change', onChange)
        input.click()
      })
      if (result === null) return null
      bytes = result.bytes
      fileName = result.name
    }

    return this.resolveImportPick(bytes, fileName)
  }

  /**
   * Given a picked import-format file's bytes and display name, dispatch to
   * the matching `ImportPick` kind. glTF/SketchUp/STL embed (or lack) their
   * own resources, so they carry the bytes as-is; COLLADA additionally
   * offers a best-effort directory-picker texture scan (skipped if the API
   * is unavailable or the user cancels it — the kernel ImportReport will
   * list any still-missing textures).
   */
  private async resolveImportPick(bytes: Uint8Array, fileName: string): Promise<ImportPick> {
    // glTF embeds its own buffers/images — no external resolution needed.
    if (/\.(glb|gltf)$/i.test(fileName)) {
      return { kind: 'gltf', name: fileName, bytes }
    }

    // SketchUp files embed their textures — no external resolution needed.
    if (/\.skp$/i.test(fileName)) {
      return { kind: 'skp', name: fileName, bytes }
    }

    // STL has no external resources (and no units — the caller prompts).
    if (/\.stl$/i.test(fileName)) {
      return { kind: 'stl', name: fileName, bytes }
    }

    // COLLADA: best-effort texture resolution via a directory picker. If the
    // API is unavailable or the user cancels, proceed with no images — the
    // kernel ImportReport will list them as missing textures, which is fine.
    const images: Record<string, ImageEntry> = {}
    if (hasFSAA() && 'showDirectoryPicker' in window) {
      try {
        const dirHandle = await showDirectoryPicker({ mode: 'read' })
        await collectImagesFromDirectory(dirHandle, '', images)
      } catch {
        // User cancelled or permission denied — proceed without images.
      }
    }
    return { kind: 'dae', name: fileName, bytes, images }
  }

  /**
   * Prompt with ONE dialog covering `.hew` plus every import format, and
   * dispatch on the extension the user picked. Mirrors `open()`'s FSAA vs.
   * hidden-`<input>` fallback shape, and `openForImport()`'s post-pick
   * dispatch (shared via `resolveImportPick`).
   */
  async openAny(): Promise<OpenPick | null> {
    let bytes: Uint8Array
    let fileName: string
    let handle: FileSystemFileHandle | null = null

    if (hasFSAA()) {
      let handles: FileSystemFileHandle[]
      try {
        handles = await showOpenFilePicker({
          multiple: false,
          types: OPEN_FILE_TYPES,
          excludeAcceptAllOption: false,
        })
      } catch (err) {
        if (err instanceof DOMException && err.name === 'AbortError') return null
        throw err
      }
      const picked = handles[0]
      if (picked === undefined) return null
      const file = await picked.getFile()
      bytes = new Uint8Array(await file.arrayBuffer())
      fileName = file.name
      handle = picked
    } else {
      // Fallback: hidden <input type=file>
      const result = await new Promise<{ bytes: Uint8Array; name: string } | null>((resolve) => {
        const input = document.createElement('input')
        input.type = 'file'
        // Omit `accept` on coarse pointers — see isCoarsePointer().
        if (!isCoarsePointer()) {
          input.accept = '.hew,.dae,.skp,.glb,.gltf,.stl'
        }
        input.style.display = 'none'
        document.body.appendChild(input)

        const onChange = () => {
          input.removeEventListener('change', onChange)
          document.body.removeChild(input)
          const file = input.files?.[0]
          if (file == null) {
            resolve(null)
            return
          }
          file.arrayBuffer().then((buf) => resolve({ bytes: new Uint8Array(buf), name: file.name })).catch(() => resolve(null))
        }

        input.addEventListener('change', onChange)
        input.click()
      })
      if (result === null) return null
      bytes = result.bytes
      fileName = result.name
    }

    if (/\.hew$/i.test(fileName)) {
      return { kind: 'hew', name: fileName, bytes, handle }
    }
    const pick = await this.resolveImportPick(bytes, fileName)
    // A `.skp` Open makes the file the document's own — keep the handle so
    // Save writes back in place. (Other import kinds never keep one.)
    if (pick.kind === 'skp' && handle !== null) return { ...pick, handle }
    return pick
  }
}

/** Recursively collect image files from a directory handle into `out`. */
async function collectImagesFromDirectory(
  dir: FileSystemDirectoryHandle,
  prefix: string,
  out: Record<string, ImageEntry>,
): Promise<void> {
  for await (const [name, handle] of dir.entries()) {
    const key = prefix !== '' ? `${prefix}/${name}` : name
    if (handle.kind === 'file') {
      const lower = name.toLowerCase()
      let format: 'png' | 'jpeg' | null = null
      if (lower.endsWith('.png')) format = 'png'
      else if (lower.endsWith('.jpg') || lower.endsWith('.jpeg')) format = 'jpeg'
      if (format !== null) {
        const fileHandle = handle as FileSystemFileHandle
        const file = await fileHandle.getFile()
        const bytes = new Uint8Array(await file.arrayBuffer())
        // Register under both the bare filename and the prefixed path so
        // COLLADA references like "textures/wood.png" and "wood.png" both hit.
        out[key] = { bytes, format }
        out[name] = { bytes, format }
      }
    } else if (handle.kind === 'directory') {
      await collectImagesFromDirectory(handle as FileSystemDirectoryHandle, key, out)
    }
  }
}
