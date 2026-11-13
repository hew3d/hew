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

import { t } from '@lingui/core/macro'
import type { DocFormat, ExportFileType, FileHost, FileRef, ImageEntry, ImportPick, OpenPick } from './fileHost'
import { FileReadError } from './fileHost'
import { extensionFor, formatDescription, nameForFormat } from './docFormat'

/**
 * Write `bytes` to `path` via the raw-IPC-body `write_file_bytes` command
 * (shells/tauri/src-tauri/src/main.rs) instead of `write_file`'s `Vec<u8>`
 * JSON argument — a JSON number array costs roughly 4 bytes of IPC JSON per
 * file byte, which for a large export (a bug-report bundle carrying an
 * imported model easily reaches hundreds of MB) is slow enough to look like
 * a hang. The path can't ride the invoke's normal argument position — a
 * raw-body invoke's argument IS the byte buffer, the same reason
 * `relayClient.ts`'s `relayPut` passes bytes directly rather than under a
 * named key — so it goes in a `path` request header instead, percent-encoded
 * since header values are ASCII/Latin-1-only in the browser `Headers` model
 * `invoke`'s `headers` option goes through (main.rs's `write_file_bytes`
 * undoes the encoding). Subject to the exact same path-approval check
 * `write_file` uses.
 */
async function writeFileBytes(path: string, bytes: Uint8Array): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core')
  await invoke('write_file_bytes', bytes as never, { headers: { path: encodeURIComponent(path) } })
}

/**
 * Write a document's bytes to `path`. `.hew` keeps the original
 * `write_file` JSON-array route; a SketchUp file goes through the raw-body
 * route, because a real model is tens to hundreds of megabytes and the JSON
 * route costs about four bytes of IPC per file byte.
 */
async function writeDocument(path: string, bytes: Uint8Array, format: DocFormat): Promise<void> {
  if (format === 'hew') {
    const { invoke } = await import('@tauri-apps/api/core')
    // write_file expects Vec<u8>; pass Array.from so Tauri IPC sees a JSON array.
    await invoke('write_file', { path, contents: Array.from(bytes) })
    return
  }
  await writeFileBytes(path, bytes)
}

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
    // `handle` is the path: a `.skp` Open binds the file as the document's
    // own, and (in every Tauri pick) the path is already write-approved.
    return { kind: 'skp', name: basename(filePath), bytes: fileBytes, path: filePath, handle: filePath }
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
    // oxlint-disable-next-line lingui/no-unlocalized-strings -- directory names probed on disk
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
      filters: [{ name: t`Hew model`, extensions: ['hew'] }],
      write: true,
    })
    if (path === null) return null

    // read_file returns a raw IPC response, which resolves to an ArrayBuffer.
    // new Uint8Array(buf) is a zero-copy view over it.
    const buf = await invoke<ArrayBuffer>('read_file', { path })
    const bytes = new Uint8Array(buf)

    return {
      ref: { name: basename(path), format: 'hew', handle: path },
      bytes,
    }
  }

  async save(bytes: Uint8Array, ref: FileRef | null): Promise<FileRef | null> {
    if (ref === null) {
      // oxlint-disable-next-line lingui/no-unlocalized-strings -- default file name, not copy
      return this.saveAs(bytes, 'Untitled.hew')
    }
    const path = ref.handle as string
    await writeDocument(path, bytes, ref.format)
    return ref
  }

  async saveAs(bytes: Uint8Array, suggestedName: string, format: DocFormat = 'hew'): Promise<FileRef | null> {
    const { invoke } = await import('@tauri-apps/api/core')
    const path = await invoke<string | null>('pick_save_path', {
      defaultName: nameForFormat(suggestedName, format),
      filters: [{ name: formatDescription(format), extensions: [extensionFor(format).slice(1)] }],
    })
    if (path === null) return null

    await writeDocument(path, bytes, format)

    return { name: basename(path), format, handle: path }
  }

  async writeTo(path: string, bytes: Uint8Array, format?: DocFormat): Promise<FileRef> {
    if (format === undefined) await writeFileBytes(path, bytes)
    else await writeDocument(path, bytes, format)
    return { name: basename(path), format: format ?? 'hew', handle: path }
  }

  async exportBinary(
    bytes: Uint8Array,
    suggestedName: string,
    fileType: ExportFileType,
  ): Promise<string | null> {
    const dotExt = '.' + fileType.ext
    const { invoke } = await import('@tauri-apps/api/core')
    const path = await invoke<string | null>('pick_save_path', {
      defaultName: suggestedName.endsWith(dotExt) ? suggestedName : suggestedName + dotExt,
      filters: [{ name: fileType.description, extensions: [fileType.ext] }],
    })
    if (path === null) return null

    await writeFileBytes(path, bytes)
    return path
  }

  async openImage(): Promise<{ name: string; bytes: Uint8Array } | null> {
    const { invoke } = await import('@tauri-apps/api/core')
    const filePath = await invoke<string | null>('pick_open_path', {
      filters: [{ name: 'Pictures (PNG, JPEG)', extensions: ['png', 'jpg', 'jpeg'] }],
      write: false,
    })
    if (filePath === null) return null
    // A pick whose bytes can't be read is not a cancel: reject with
    // FileReadError, as the FileHost contract says (and as the web host
    // does), so the caller says so.
    let fileBuf: ArrayBuffer
    try {
      fileBuf = await invoke<ArrayBuffer>('read_file', { path: filePath })
    } catch (err) {
      throw new FileReadError(basename(filePath), err)
    }
    return { name: basename(filePath), bytes: new Uint8Array(fileBuf) }
  }

  async openForImport(): Promise<ImportPick | null> {
    const { invoke } = await import('@tauri-apps/api/core')
    // The native picker grants sibling reads only when the picked file is
    // COLLADA, whose texture scan needs textures/-style subfolders.
    const filePath = await invoke<string | null>('pick_open_path', {
      filters: [
        {
          name: t`Model files (COLLADA, SketchUp, glTF, STL)`,
          extensions: ['dae', 'skp', 'glb', 'gltf', 'stl'],
        },
        { name: t`COLLADA model`, extensions: ['dae'] },
        { name: t`SketchUp Model`, extensions: ['skp'] },
        { name: t`glTF model`, extensions: ['glb', 'gltf'] },
        { name: t`STL model`, extensions: ['stl'] },
      ],
      write: false,
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
   * a further dialog). Rust decides additional directory read approval from
   * the resolved extension: only COLLADA needs sibling texture access.
   */
  async openAny(): Promise<OpenPick | null> {
    const { invoke } = await import('@tauri-apps/api/core')
    const filePath = await invoke<string | null>('pick_open_path', {
      filters: [
        {
          name: t`All supported files`,
          extensions: ['hew', 'dae', 'skp', 'glb', 'gltf', 'stl'],
        },
        { name: t`Hew model`, extensions: ['hew'] },
        { name: t`COLLADA model`, extensions: ['dae'] },
        { name: t`SketchUp Model`, extensions: ['skp'] },
        { name: t`glTF model`, extensions: ['glb', 'gltf'] },
        { name: t`STL model`, extensions: ['stl'] },
      ],
      write: true,
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
