/**
 * Minimal ambient declarations for the File System Access API.
 *
 * FSAA isn't in the TypeScript DOM lib yet (lib.dom.d.ts has partial coverage
 * but not the exact shapes we rely on).  We declare only the members we use
 * rather than pulling in an npm package.
 *
 * Runtime feature detection: check `'showSaveFilePicker' in window`.
 */

interface FilePickerAcceptType {
  description?: string
  accept: Record<string, string[]>
}

interface OpenFilePickerOptions {
  multiple?: boolean
  types?: FilePickerAcceptType[]
  excludeAcceptAllOption?: boolean
}

interface SaveFilePickerOptions {
  suggestedName?: string
  types?: FilePickerAcceptType[]
  excludeAcceptAllOption?: boolean
}

interface FileSystemWritableFileStream extends WritableStream {
  write(data: BufferSource | Blob | string): Promise<void>
  close(): Promise<void>
}

interface FileSystemHandlePermissionDescriptor {
  mode?: 'read' | 'readwrite'
}

interface FileSystemFileHandle {
  readonly name: string
  getFile(): Promise<File>
  createWritable(): Promise<FileSystemWritableFileStream>
  queryPermission(descriptor?: FileSystemHandlePermissionDescriptor): Promise<'granted' | 'denied' | 'prompt'>
  requestPermission(descriptor?: FileSystemHandlePermissionDescriptor): Promise<'granted' | 'denied' | 'prompt'>
}

declare function showOpenFilePicker(
  options?: OpenFilePickerOptions,
): Promise<FileSystemFileHandle[]>

declare function showSaveFilePicker(
  options?: SaveFilePickerOptions,
): Promise<FileSystemFileHandle>
