/**
 * Directory-handle helpers shared by the browser stores that keep files in
 * the origin-private file system (or a picked folder, which speaks the same
 * handle API).
 */

export async function subdir(
  root: FileSystemDirectoryHandle,
  name: string,
  create: boolean,
): Promise<FileSystemDirectoryHandle | null> {
  try {
    return await root.getDirectoryHandle(name, { create })
  } catch {
    return null
  }
}

export async function readFileIn(dir: FileSystemDirectoryHandle, base: string): Promise<Uint8Array> {
  const handle = await dir.getFileHandle(base)
  const file = await handle.getFile()
  return new Uint8Array(await file.arrayBuffer())
}

export async function writeFileIn(
  dir: FileSystemDirectoryHandle,
  base: string,
  bytes: Uint8Array,
): Promise<void> {
  const handle = await dir.getFileHandle(base, { create: true })
  const writable = await handle.createWritable()
  // Copy into a fresh ArrayBuffer-backed view: `bytes` may be a view into a
  // larger (or SharedArrayBuffer-backed, on some wasm paths) buffer, which
  // FileSystemWritableFileStream.write would either over-write or reject.
  await writable.write(new Uint8Array(bytes).buffer as ArrayBuffer)
  await writable.close()
}
