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

import type { FileHost, FileRef, ImageEntry } from './fileHost'

/** Extract the basename from a path that may use / or \ separators. */
function basename(path: string): string {
  return path.replace(/[/\\]+/g, '/').split('/').filter(Boolean).pop() ?? path
}

/** Extract the directory (parent) from an absolute path. */
function dirname(path: string): string {
  const normalized = path.replace(/[/\\]+/g, '/')
  const idx = normalized.lastIndexOf('/')
  return idx < 0 ? '.' : normalized.slice(0, idx)
}

/** Return true if the filename extension is a supported image format. */
function imageFormat(name: string): 'png' | 'jpeg' | null {
  const lower = name.toLowerCase()
  if (lower.endsWith('.png')) return 'png'
  if (lower.endsWith('.jpg') || lower.endsWith('.jpeg')) return 'jpeg'
  return null
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

  async openForImport(): Promise<{
    daeBytes: Uint8Array
    images: Record<string, ImageEntry>
    name: string
  } | null> {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const result = await open({
      filters: [{ name: 'COLLADA model', extensions: ['dae'] }],
      multiple: false,
    })
    if (result === null) return null
    const daePath = result as string

    const { invoke } = await import('@tauri-apps/api/core')
    const rawDae: number[] = await invoke('read_file', { path: daePath })
    const daeBytes = new Uint8Array(rawDae)

    // Scan the sibling directory (and a <stem>_textures / textures subfolder
    // if present) for PNG/JPEG images.  Best-effort — failures are silently
    // ignored; the kernel ImportReport will list missing textures.
    const images: Record<string, ImageEntry> = {}
    const dir = dirname(daePath)

    try {
      const rawList: string[] = await invoke('list_dir', { path: dir })
      for (const entry of rawList) {
        const name = basename(entry)
        const fmt = imageFormat(name)
        if (fmt !== null) {
          try {
            const rawImg: number[] = await invoke('read_file', { path: entry })
            const imgBytes = new Uint8Array(rawImg)
            images[name] = { bytes: imgBytes, format: fmt }
          } catch {
            // ignore unreadable files
          }
        }
      }

      // Also scan known texture sub-directories.
      // Includes the SketchUp-style "<stem>/" sibling folder (named after the
      // .dae file without extension), "<stem>_textures", "textures", "Textures".
      const stem = basename(daePath).replace(/\.dae$/i, '')
      const textureDirs = [`${dir}/${stem}`, `${dir}/${stem}_textures`, `${dir}/textures`, `${dir}/Textures`]
      for (const texDir of textureDirs) {
        let subList: string[]
        try {
          subList = await invoke('list_dir', { path: texDir })
        } catch {
          continue // directory doesn't exist
        }
        for (const entry of subList) {
          const name = basename(entry)
          const fmt = imageFormat(name)
          if (fmt !== null) {
            try {
              const rawImg: number[] = await invoke('read_file', { path: entry })
              const imgBytes = new Uint8Array(rawImg)
              // Key by both bare filename and subdir-relative path so COLLADA
              // references like "textures/wood.png" and "wood.png" both resolve.
              const relKey = `${basename(texDir)}/${name}`
              images[name] = { bytes: imgBytes, format: fmt }
              images[relKey] = { bytes: imgBytes, format: fmt }
            } catch {
              // ignore
            }
          }
        }
      }
    } catch {
      // list_dir not available or permission error — proceed without images.
    }

    return { daeBytes, images, name: basename(daePath) }
  }
}
