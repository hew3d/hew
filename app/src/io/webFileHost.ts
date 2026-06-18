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
 */

import type { FileHost, FileRef } from './fileHost'

const HEW_FILE_TYPES: FilePickerAcceptType[] = [
  {
    description: 'Hew model',
    accept: { 'application/octet-stream': ['.hew'] },
  },
]

/** True when the File System Access API save picker is available. */
function hasFSAA(): boolean {
  return typeof window !== 'undefined' && 'showSaveFilePicker' in window
}

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

/** Trigger an anchor-download of bytes as a .hew file. */
function anchorDownload(bytes: Uint8Array, name: string): void {
  const blob = new Blob([new Uint8Array(bytes)], { type: 'application/octet-stream' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = name.endsWith('.hew') ? name : name + '.hew'
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
      el.accept = '.hew'
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
        ref: { name: file.name, handle },
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
            ref: { name: file.name, handle: null },
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
      return this.saveAs(bytes, 'Untitled.hew')
    }

    // FSAA in-place overwrite: ref.handle is a FileSystemFileHandle
    if (hasFSAA() && ref.handle instanceof Object && 'createWritable' in (ref.handle as object)) {
      const handle = ref.handle as FileSystemFileHandle
      // Verify readwrite permission before writing.  Chrome auto-grants;
      // Brave keeps the handle at 'prompt' and needs an explicit request.
      // If the user denies, fall through to saveAs so the document isn't lost.
      if (!(await ensureWritePermission(handle))) {
        return this.saveAs(bytes, ref.name)
      }
      try {
        const writable = await handle.createWritable()
        await writable.write(new Uint8Array(bytes))
        await writable.close()
        return { name: handle.name, handle }
      } catch (err) {
        if (err instanceof DOMException && err.name === 'AbortError') return null
        throw err
      }
    }

    // Fallback: anchor-download (no in-place overwrite possible)
    anchorDownload(bytes, ref.name)
    return ref
  }

  async saveAs(bytes: Uint8Array, suggestedName: string): Promise<FileRef | null> {
    if (hasFSAA()) {
      let handle: FileSystemFileHandle
      try {
        handle = await showSaveFilePicker({
          suggestedName: suggestedName.endsWith('.hew') ? suggestedName : suggestedName + '.hew',
          types: HEW_FILE_TYPES,
          excludeAcceptAllOption: false,
        })
      } catch (err) {
        if (err instanceof DOMException && err.name === 'AbortError') return null
        throw err
      }
      const writable = await handle.createWritable()
      await writable.write(new Uint8Array(bytes))
      await writable.close()
      return { name: handle.name, handle }
    }

    // Fallback: anchor-download
    anchorDownload(bytes, suggestedName)
    // Return a FileRef with no handle — subsequent "Save" will download again.
    const name = suggestedName.endsWith('.hew') ? suggestedName : suggestedName + '.hew'
    return { name, handle: null }
  }
}
