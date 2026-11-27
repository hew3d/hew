/**
 * In-memory stand-ins for the browser storage the web stores sit on: a
 * directory-handle tree (OPFS or a picked folder) and BroadcastChannels.
 */

// ---------------------------------------------------------------------------
// Fake FileSystemDirectoryHandle tree
// ---------------------------------------------------------------------------

interface FakeFile {
  bytes: Uint8Array
  mtime: number
}

export class FakeDir {
  readonly kind = 'directory' as const
  files = new Map<string, FakeFile>()
  dirs = new Map<string, FakeDir>()
  /** Permission the handle reports; requestPermission upgrades to this
   * only when `grantOnRequest` is true. */
  permission: 'granted' | 'denied' | 'prompt' = 'granted'
  grantOnRequest = true

  constructor(readonly name: string) {}

  async getDirectoryHandle(name: string, options?: { create?: boolean }): Promise<FakeDir> {
    let d = this.dirs.get(name)
    if (!d) {
      if (!options?.create) throw new Error(`NotFoundError: ${name}`)
      d = new FakeDir(name)
      this.dirs.set(name, d)
    }
    return d
  }

  async getFileHandle(name: string, options?: { create?: boolean }) {
    if (!this.files.has(name)) {
      if (!options?.create) throw new Error(`NotFoundError: ${name}`)
      this.files.set(name, { bytes: new Uint8Array(), mtime: 0 })
    }
    const dir = this
    return {
      kind: 'file' as const,
      name,
      async getFile() {
        const f = dir.files.get(name)
        if (!f) throw new Error(`NotFoundError: ${name}`)
        return {
          size: f.bytes.length,
          lastModified: f.mtime,
          arrayBuffer: async () => new Uint8Array(f.bytes).buffer,
        }
      },
      async createWritable() {
        let pending: Uint8Array | null = null
        return {
          async write(data: ArrayBuffer) {
            pending = new Uint8Array(data)
          },
          async close() {
            dir.files.set(name, { bytes: pending ?? new Uint8Array(), mtime: 1_000 })
          },
        }
      },
    }
  }

  async removeEntry(name: string, options?: { recursive?: boolean }): Promise<void> {
    const dir = this.dirs.get(name)
    if (dir !== undefined) {
      // Real directories refuse removal while they hold entries, unless asked.
      if (!options?.recursive && (dir.files.size > 0 || dir.dirs.size > 0)) {
        throw new Error(`InvalidModificationError: ${name} is not empty`)
      }
      this.dirs.delete(name)
      return
    }
    if (!this.files.delete(name)) throw new Error(`NotFoundError: ${name}`)
  }

  async *entries(): AsyncIterableIterator<[string, { kind: 'file' | 'directory'; name: string; getFile?: () => Promise<unknown> }]> {
    for (const [name] of this.files) {
      yield [name, await this.getFileHandle(name)]
    }
    for (const [name, d] of this.dirs) {
      yield [name, d]
    }
  }

  async queryPermission() {
    return this.permission
  }

  async requestPermission() {
    if (this.permission !== 'granted' && this.grantOnRequest) this.permission = 'granted'
    return this.permission
  }
}

export function fakeChannel(): BroadcastChannel & { emit: () => void } {
  const handlers = new Set<() => void>()
  return {
    addEventListener: (_type: string, fn: () => void) => handlers.add(fn),
    postMessage: () => {},
    emit: () => handlers.forEach((h) => h()),
  } as unknown as BroadcastChannel & { emit: () => void }
}

/** Two channels wired to each other, the way two tabs' BroadcastChannels
 * are: posting on one fires the OTHER's listeners, never its own. */
export function linkedChannels(): [BroadcastChannel, BroadcastChannel] {
  const a = new Set<() => void>()
  const b = new Set<() => void>()
  const make = (own: Set<() => void>, other: Set<() => void>) =>
    ({
      addEventListener: (_t: string, fn: () => void) => own.add(fn),
      postMessage: () => other.forEach((h) => h()),
    }) as unknown as BroadcastChannel
  return [make(a, b), make(b, a)]
}

