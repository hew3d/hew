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

import type { ExportFileType, FileHost, FileRef, ImageEntry, ImportPick, OpenPick } from './fileHost'

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

/**
 * Given a picked import-format path and its already-read bytes, dispatch to
 * the matching `ImportPick` kind, stamped with the path it came from.
 * glTF/SketchUp/STL embed (or lack) their own resources, so they return the
 * bytes as-is; COLLADA additionally scans the sibling directory (and
 * textures/-style subfolders) for PNG/JPEG images, best-effort — failures
 * are silently ignored, since the kernel ImportReport will list any still-
 * missing textures.
 *
 * A free function (not a method) so App.tsx's `openPath` — which opens a
 * path handed to it by the shell (a File ▸ Open pick delivered to a freshly
 * opened window, see `open_in_new_window`) rather than one picked through a
 * dialog here — can reach the same dispatch/texture-scan logic without a
 * `TauriFileHost` instance or a redundant dialog.
 */
export async function resolveImportPickFromPath(filePath: string, fileBytes: Uint8Array): Promise<ImportPick> {
  const { invoke } = await import('@tauri-apps/api/core')

  // glTF embeds its own buffers/images — return the bytes, skip texture scan.
  if (/\.(glb|gltf)$/i.test(filePath)) {
    return { kind: 'gltf', name: basename(filePath), bytes: fileBytes, path: filePath }
  }

  // SketchUp files embed their textures — return the bytes, skip texture scan.
  if (/\.skp$/i.test(filePath)) {
    return { kind: 'skp', name: basename(filePath), bytes: fileBytes, path: filePath }
  }

  // STL has no external resources (and no units — the caller prompts).
  if (/\.stl$/i.test(filePath)) {
    return { kind: 'stl', name: basename(filePath), bytes: fileBytes, path: filePath }
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
          const imgBuf = await invoke<ArrayBuffer>('read_file', { path: entry })
          const imgBytes = new Uint8Array(imgBuf)
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
            const imgBuf = await invoke<ArrayBuffer>('read_file', { path: entry })
            const imgBytes = new Uint8Array(imgBuf)
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

  return { kind: 'dae', name: basename(filePath), bytes: fileBytes, images, path: filePath }
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

    // read_file returns a raw IPC response, which resolves to an ArrayBuffer.
    // new Uint8Array(buf) is a zero-copy view over it.
    const buf = await invoke<ArrayBuffer>('read_file', { path })
    const bytes = new Uint8Array(buf)

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
        {
          name: 'Model files (COLLADA, SketchUp, glTF, STL)',
          extensions: ['dae', 'skp', 'glb', 'gltf', 'stl'],
        },
        { name: 'COLLADA model', extensions: ['dae'] },
        { name: 'SketchUp 2017 Model', extensions: ['skp'] },
        { name: 'glTF model', extensions: ['glb', 'gltf'] },
        { name: 'STL model', extensions: ['stl'] },
      ],
      write: false,
      approveDir: true,
    })
    if (filePath === null) return null

    const fileBuf = await invoke<ArrayBuffer>('read_file', { path: filePath })
    const fileBytes = new Uint8Array(fileBuf)
    return resolveImportPickFromPath(filePath, fileBytes)
  }

  /**
   * Prompt with ONE dialog covering `.hew` plus every import format, and
   * dispatch on the extension the user picked. `write: true` mirrors
   * `open()`'s hew-only pick (a hew pick may be Saved back in place without
   * a further dialog); `approveDir: true` mirrors `openForImport()`'s pick
   * (a DAE pick needs its sibling directory approved for the texture scan).
   * Both are granted regardless of which extension is actually picked, since
   * the extension isn't known until the dialog resolves and approval is
   * granted at pick time — `write` is inert for every non-hew kind (nothing
   * ever calls `write_file` against an import-format path), but `approveDir`
   * is a real, if bounded, widening: a plain `.hew` pick through THIS dialog
   * now also gets its containing directory approved for read/list (see
   * `ApprovedPaths::read_dirs` in shells/tauri/src-tauri/src/main.rs), where
   * the narrower `.hew`-only `open()` above never did. Flagged for the
   * maintainer, not fixed here — gating `approveDir` on the resolved path's
   * own extension would need a Rust-side change to `pick_open_path` itself.
   */
  async openAny(): Promise<OpenPick | null> {
    const { invoke } = await import('@tauri-apps/api/core')
    const filePath = await invoke<string | null>('pick_open_path', {
      filters: [
        {
          name: 'All supported files',
          extensions: ['hew', 'dae', 'skp', 'glb', 'gltf', 'stl'],
        },
        { name: 'Hew model', extensions: ['hew'] },
        { name: 'COLLADA model', extensions: ['dae'] },
        { name: 'SketchUp 2017 Model', extensions: ['skp'] },
        { name: 'glTF model', extensions: ['glb', 'gltf'] },
        { name: 'STL model', extensions: ['stl'] },
      ],
      write: true,
      approveDir: true,
    })
    if (filePath === null) return null

    const fileBuf = await invoke<ArrayBuffer>('read_file', { path: filePath })
    const fileBytes = new Uint8Array(fileBuf)

    if (/\.hew$/i.test(filePath)) {
      return { kind: 'hew', name: basename(filePath), bytes: fileBytes, handle: filePath }
    }
    return resolveImportPickFromPath(filePath, fileBytes)
  }
}
