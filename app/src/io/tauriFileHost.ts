/**
 * TauriFileHost — native desktop implementation of FileHost.
 *
 * Uses custom shell commands throughout:
 *   - pick_open_path / pick_save_path show the native dialogs FROM RUST and
 *     record the user's pick in the shell's approved-paths registry before
 *     the path ever reaches this webview — read_file / write_file / list_dir
 *     refuse paths with no such approval, so a compromised webview cannot
 *     use them as arbitrary file I/O.
 *   - read_file / write_file / list_dir do the gated I/O.
 *
 * All Tauri imports are DYNAMIC so this module is never bundled into the web
 * build.  Vite code-splits dynamic imports into separate chunks.
 *
 * FileRef.handle is the absolute file path string.
 */

import type { ExportFileType, FileHost, FileRef, ImageEntry, ImportPick } from './fileHost'

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
    const { invoke } = await import('@tauri-apps/api/core')
    // write: true — Save writes back to the opened path without a new dialog.
    const path = await invoke<string | null>('pick_open_path', {
      filters: [{ name: 'Hew model', extensions: ['hew'] }],
      write: true,
      approveDir: false,
    })
    if (path === null) return null

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
    const { invoke } = await import('@tauri-apps/api/core')
    const path = await invoke<string | null>('pick_save_path', {
      defaultName: suggestedName.endsWith('.hew') ? suggestedName : suggestedName + '.hew',
      filters: [{ name: 'Hew model', extensions: ['hew'] }],
    })
    if (path === null) return null

    await invoke('write_file', { path, contents: Array.from(bytes) })

    return { name: basename(path), handle: path }
  }

  async exportBinary(
    bytes: Uint8Array,
    suggestedName: string,
    fileType: ExportFileType,
  ): Promise<boolean> {
    const dotExt = '.' + fileType.ext
    const { invoke } = await import('@tauri-apps/api/core')
    const path = await invoke<string | null>('pick_save_path', {
      defaultName: suggestedName.endsWith(dotExt) ? suggestedName : suggestedName + dotExt,
      filters: [{ name: fileType.description, extensions: [fileType.ext] }],
    })
    if (path === null) return false

    await invoke('write_file', { path, contents: Array.from(bytes) })
    return true
  }

  async openForImport(): Promise<ImportPick | null> {
    const { invoke } = await import('@tauri-apps/api/core')
    // approveDir: true — the DAE texture scan below reads the picked file's
    // siblings and textures/-style subfolders; the pick approves that too.
    const filePath = await invoke<string | null>('pick_open_path', {
      filters: [
        { name: 'Model files (COLLADA, SketchUp, glTF)', extensions: ['dae', 'skp', 'glb', 'gltf'] },
        { name: 'COLLADA model', extensions: ['dae'] },
        { name: 'SketchUp 2017 Model', extensions: ['skp'] },
        { name: 'glTF model', extensions: ['glb', 'gltf'] },
      ],
      write: false,
      approveDir: true,
    })
    if (filePath === null) return null

    const rawFile: number[] = await invoke('read_file', { path: filePath })
    const fileBytes = new Uint8Array(rawFile)

    // glTF embeds its own buffers/images — return the bytes, skip texture scan.
    if (/\.(glb|gltf)$/i.test(filePath)) {
      return { kind: 'gltf', name: basename(filePath), bytes: fileBytes }
    }

    // SketchUp files embed their textures — return the bytes, skip texture scan.
    if (/\.skp$/i.test(filePath)) {
      return { kind: 'skp', name: basename(filePath), bytes: fileBytes }
    }

    // Scan the sibling directory (and a <stem>_textures / textures subfolder
    // if present) for PNG/JPEG images.  Best-effort — failures are silently
    // ignored; the kernel ImportReport will list missing textures.
    const images: Record<string, ImageEntry> = {}
    const dir = dirname(filePath)

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
      const stem = basename(filePath).replace(/\.dae$/i, '')
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

    return { kind: 'dae', name: basename(filePath), bytes: fileBytes, images }
  }
}
