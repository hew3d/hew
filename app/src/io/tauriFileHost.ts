/**
 * TauriFileHost — native desktop implementation of FileHost.
 *
 * Uses:
 *   - @tauri-apps/plugin-dialog  for open/save native file dialogs
 *   - @tauri-apps/api/core invoke for custom read_file / write_file commands
 *
 * All Tauri imports are DYNAMIC so this module is never bundled into the web
 * build.  Vite code-splits dynamic imports into separate chunks.
 *
 * FileRef.handle is the absolute file path string.
 */

import type { FileHost, FileRef } from './fileHost'

/** Extract the basename from a path that may use / or \ separators. */
function basename(path: string): string {
  return path.replace(/[/\\]+/g, '/').split('/').filter(Boolean).pop() ?? path
}

export class TauriFileHost implements FileHost {
  async open(): Promise<{ ref: FileRef; bytes: Uint8Array } | null> {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const result = await open({
      filters: [{ name: 'Hew model', extensions: ['hew'] }],
      multiple: false,
    })
    // open() returns null on cancel, or a string path (multiple:false → string)
    if (result === null) return null
    const path = result as string

    const { invoke } = await import('@tauri-apps/api/core')
    // read_file returns Vec<u8> which wasm-bindgen marshals as a JS Array<number>.
    const raw: number[] = await invoke('read_file', { path })
    const bytes = new Uint8Array(raw)

    return {
      ref: { name: basename(path), handle: path },
      bytes,
    }
  }

  async save(bytes: Uint8Array, ref: FileRef | null): Promise<FileRef | null> {
    if (ref === null) {
      return this.saveAs(bytes, 'Untitled.hew')
    }
    const path = ref.handle as string
    const { invoke } = await import('@tauri-apps/api/core')
    // write_file expects Vec<u8>; pass Array.from so Tauri IPC sees a JSON array.
    await invoke('write_file', { path, contents: Array.from(bytes) })
    return ref
  }

  async saveAs(bytes: Uint8Array, suggestedName: string): Promise<FileRef | null> {
    const { save } = await import('@tauri-apps/plugin-dialog')
    const result = await save({
      defaultPath: suggestedName.endsWith('.hew') ? suggestedName : suggestedName + '.hew',
      filters: [{ name: 'Hew model', extensions: ['hew'] }],
    })
    if (result === null) return null
    const path = result as string

    const { invoke } = await import('@tauri-apps/api/core')
    await invoke('write_file', { path, contents: Array.from(bytes) })

    return { name: basename(path), handle: path }
  }
}
