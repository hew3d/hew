/**
 * webLibraryStore.test — the browser library store against in-memory fakes
 * (same injection philosophy as recents.test.ts): a fake directory-handle
 * tree standing in for both OPFS and a picked FSA folder, a fake
 * IndexedDB-shaped config store, and a fake BroadcastChannel. Covers the
 * round-trips, the desktop-mirroring name validation, folder binding with
 * one-way migration, the permission/reconnect flow, and config persistence
 * across store instances.
 */

import { describe, expect, it, vi } from 'vitest'
import { makeWebLibraryStore, webLibrarySupported, type WebLibraryDeps } from './webLibraryStore'

// ---------------------------------------------------------------------------
// Fake FileSystemDirectoryHandle tree
// ---------------------------------------------------------------------------

interface FakeFile {
  bytes: Uint8Array
  mtime: number
}

class FakeDir {
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

  async removeEntry(name: string): Promise<void> {
    if (!this.files.delete(name) && !this.dirs.delete(name)) {
      throw new Error(`NotFoundError: ${name}`)
    }
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

// ---------------------------------------------------------------------------
// Fake config store shaped like the two IDB helpers need (open/get/put).
// The store only ever uses get/put on one object store, so the fake keeps
// a shared Map per "factory" and implements just that surface.
// ---------------------------------------------------------------------------

function fakeIdb(): IDBFactory {
  const data = new Map<string, unknown>()
  const makeRequest = (result?: unknown) => {
    const req: {
      result: unknown
      error: null
      onsuccess: null | (() => void)
      onerror: null | (() => void)
      onupgradeneeded: null | (() => void)
    } = { result, error: null, onsuccess: null, onerror: null, onupgradeneeded: null }
    return req
  }
  const db = {
    objectStoreNames: { contains: () => true },
    createObjectStore: () => ({}),
    close: () => {},
    transaction: (_store: string, _mode: string) => {
      const tx: { oncomplete: null | (() => void); onerror: null | (() => void); error: null; objectStore: () => unknown } = {
        oncomplete: null,
        onerror: null,
        error: null,
        objectStore: () => ({
          get: (key: string) => {
            const req = makeRequest(data.get(key))
            queueMicrotask(() => req.onsuccess?.())
            return req
          },
          put: (value: unknown, key: string) => {
            data.set(key, value)
            queueMicrotask(() => tx.oncomplete?.())
            return makeRequest()
          },
        }),
      }
      // A read-only transaction never fires oncomplete in the helpers' use.
      return tx
    },
  }
  return {
    open: () => {
      const req = makeRequest(db)
      queueMicrotask(() => req.onsuccess?.())
      return req as unknown as IDBOpenDBRequest
    },
  } as unknown as IDBFactory
}

function fakeChannel(): BroadcastChannel & { emit: () => void } {
  const handlers = new Set<() => void>()
  return {
    addEventListener: (_type: string, fn: () => void) => handlers.add(fn),
    postMessage: () => {},
    emit: () => handlers.forEach((h) => h()),
  } as unknown as BroadcastChannel & { emit: () => void }
}

/** Two channels wired to each other, the way two tabs' BroadcastChannels
 * are: posting on one fires the OTHER's listeners, never its own. */
function linkedChannels(): [BroadcastChannel, BroadcastChannel] {
  const a = new Set<() => void>()
  const b = new Set<() => void>()
  const make = (own: Set<() => void>, other: Set<() => void>) =>
    ({
      addEventListener: (_t: string, fn: () => void) => own.add(fn),
      postMessage: () => other.forEach((h) => h()),
    }) as unknown as BroadcastChannel
  return [make(a, b), make(b, a)]
}

function makeDeps(overrides: Partial<WebLibraryDeps> & { opfs?: FakeDir } = {}) {
  const opfs = overrides.opfs ?? new FakeDir('opfs-root')
  const deps: WebLibraryDeps = {
    idb: overrides.idb ?? fakeIdb(),
    opfsRoot: async () => opfs as unknown as FileSystemDirectoryHandle,
    pickDirectory: overrides.pickDirectory ?? null,
    requestPersist: overrides.requestPersist ?? (async () => true),
    channel: overrides.channel ?? null,
  }
  return { deps, opfs }
}

const bytes = (...v: number[]) => new Uint8Array(v)

// ---------------------------------------------------------------------------

describe('webLibrarySupported', () => {
  it('is true exactly when navigator.storage.getDirectory exists', () => {
    vi.stubGlobal('navigator', { storage: { getDirectory: () => Promise.resolve() } })
    expect(webLibrarySupported()).toBe(true)
    vi.stubGlobal('navigator', { storage: {} })
    expect(webLibrarySupported()).toBe(false)
    vi.stubGlobal('navigator', {})
    expect(webLibrarySupported()).toBe(false)
    vi.unstubAllGlobals()
  })
})

describe('browser-storage mode round-trips', () => {
  it('writes, lists, reads, and removes items at the root and in category folders', async () => {
    const { deps } = makeDeps()
    const store = makeWebLibraryStore(deps)
    await store.write('legacy.hew', bytes(1))
    await store.write('Components/chair.hew', bytes(2, 2))
    await store.write('Materials/oak.hew', bytes(3, 3, 3))

    const names = (await store.list()).map((e) => e.fileName).sort()
    expect(names).toEqual(['Components/chair.hew', 'Materials/oak.hew', 'legacy.hew'])
    expect(Array.from(await store.read('Components/chair.hew'))).toEqual([2, 2])
    const chair = (await store.list()).find((e) => e.fileName === 'Components/chair.hew')
    expect(chair?.size).toBe(2)

    await store.remove('Components/chair.hew')
    expect((await store.list()).map((e) => e.fileName)).not.toContain('Components/chair.hew')
    await expect(store.read('Components/chair.hew')).rejects.toThrow()
  })

  it('rejects names the desktop shell would reject', async () => {
    const { deps } = makeDeps()
    const store = makeWebLibraryStore(deps)
    for (const bad of [
      'Nested/Too/deep.hew',
      'NotACategory/item.hew',
      '../escape.hew',
      'Components/../up.hew',
      'back\\slash.hew',
      '/leading.hew',
    ]) {
      await expect(store.write(bad, bytes(1)), bad).rejects.toThrow(/invalid library item name/)
    }
  })

  it('thumbnails: miss is null, round-trip works, non-hex keys are rejected', async () => {
    const { deps } = makeDeps()
    const store = makeWebLibraryStore(deps)
    expect(await store.readThumbnail('abcdef1234')).toBeNull()
    await store.writeThumbnail('abcdef1234', bytes(9))
    expect(Array.from((await store.readThumbnail('abcdef1234'))!)).toEqual([9])
    await expect(store.writeThumbnail('../sneaky', bytes(1))).rejects.toThrow(/invalid thumbnail key/)
  })

  it('requests durable storage once, after the first successful write', async () => {
    const requestPersist = vi.fn(async () => true)
    const { deps } = makeDeps({ requestPersist })
    const store = makeWebLibraryStore(deps)
    expect(requestPersist).not.toHaveBeenCalled()
    await store.write('a.hew', bytes(1))
    await store.write('b.hew', bytes(2))
    expect(requestPersist).toHaveBeenCalledOnce()
  })

  it('reports browser mode with no reconnect needed', async () => {
    const { deps } = makeDeps()
    const store = makeWebLibraryStore(deps)
    expect(await store.webStorage!()).toEqual({ mode: 'browser', needsReconnect: false })
    expect(await store.folderInfo()).toEqual({ path: 'Browser storage' })
  })
})

describe('binding a real folder', () => {
  it('chooseFolder migrates missing items and thumbnails, never overwriting', async () => {
    const { deps, opfs } = makeDeps()
    const preStore = makeWebLibraryStore(deps)
    await preStore.write('Components/chair.hew', bytes(1))
    await preStore.write('kept.hew', bytes(2))
    await preStore.writeThumbnail('abcdef1234', bytes(7))

    const folder = new FakeDir('Hew Library')
    // The target already owns a different chair — it must win.
    const comp = await folder.getDirectoryHandle('Components', { create: true })
    comp.files.set('chair.hew', { bytes: bytes(9, 9), mtime: 5 })

    const store = makeWebLibraryStore({
      ...deps,
      opfsRoot: async () => opfs as unknown as FileSystemDirectoryHandle,
      pickDirectory: async () => folder as unknown as FileSystemDirectoryHandle,
    })
    const label = await store.chooseFolder()
    expect(label).toBe('Hew Library')

    expect(await store.webStorage!()).toEqual({ mode: 'folder', needsReconnect: false })
    expect(await store.folderInfo()).toEqual({ path: 'Hew Library' })
    const names = (await store.list()).map((e) => e.fileName).sort()
    expect(names).toEqual(['Components/chair.hew', 'kept.hew'])
    // Existing target file untouched; missing one copied; thumbnail copied.
    expect(Array.from(await store.read('Components/chair.hew'))).toEqual([9, 9])
    expect(Array.from(await store.read('kept.hew'))).toEqual([2])
    expect(Array.from((await store.readThumbnail('abcdef1234'))!)).toEqual([7])
  })

  it('a cancelled picker changes nothing', async () => {
    const { deps } = makeDeps({
      pickDirectory: async () => {
        throw new DOMException('user cancelled', 'AbortError')
      },
    })
    const store = makeWebLibraryStore(deps)
    expect(await store.chooseFolder()).toBeNull()
    expect(await store.webStorage!()).toEqual({ mode: 'browser', needsReconnect: false })
  })

  it('persists the folder choice across store instances (same config db)', async () => {
    const idb = fakeIdb()
    const folder = new FakeDir('Shared')
    const first = makeWebLibraryStore({
      ...makeDeps({ idb }).deps,
      idb,
      pickDirectory: async () => folder as unknown as FileSystemDirectoryHandle,
    })
    await first.chooseFolder()
    await first.write('a.hew', bytes(1))

    const second = makeWebLibraryStore({ ...makeDeps({ idb }).deps, idb })
    expect(await second.webStorage!()).toEqual({ mode: 'folder', needsReconnect: false })
    expect((await second.list()).map((e) => e.fileName)).toEqual(['a.hew'])
  })

  it('re-binding migrates from the CURRENTLY-bound folder, not from browser storage (review HIGH)', async () => {
    const folderA = new FakeDir('Folder A')
    const folderB = new FakeDir('Folder B')
    const picks = [folderA, folderB]
    const { deps } = makeDeps({
      pickDirectory: async () => picks.shift() as unknown as FileSystemDirectoryHandle,
    })
    const store = makeWebLibraryStore(deps)
    await store.chooseFolder() // bind A
    await store.write('saved-in-a.hew', bytes(1))
    await store.chooseFolder() // re-bind to B
    // The item followed the library to B instead of being stranded in A.
    expect((await store.list()).map((e) => e.fileName)).toEqual(['saved-in-a.hew'])
    expect(Array.from(await store.read('saved-in-a.hew'))).toEqual([1])
  })

  it('a second tab re-reads the root after a broadcast, both directions (review HIGH)', async () => {
    const idb = fakeIdb()
    const opfs = new FakeDir('opfs-root')
    const folder = new FakeDir('Bound')
    const [chanA, chanB] = linkedChannels()
    const base = { idb, opfsRoot: async () => opfs as unknown as FileSystemDirectoryHandle }
    const tabA = makeWebLibraryStore({
      ...base,
      pickDirectory: async () => folder as unknown as FileSystemDirectoryHandle,
      requestPersist: async () => true,
      channel: chanA,
    })
    const tabB = makeWebLibraryStore({
      ...base,
      pickDirectory: null,
      requestPersist: async () => true,
      channel: chanB,
    })
    // Both tabs open in browser mode; A binds a folder; B must follow.
    expect((await tabB.webStorage!()).mode).toBe('browser')
    await tabA.chooseFolder()
    await tabB.write('from-b.hew', bytes(2))
    expect(folder.files.has('from-b.hew'), 'tab B wrote to the bound folder').toBe(true)
    // And back: A returns to browser storage; B must follow again.
    await tabA.useBrowserStorage!()
    expect((await tabB.webStorage!()).mode).toBe('browser')
  })

  it('removing an already-absent item succeeds (matches the desktop shell)', async () => {
    const { deps } = makeDeps()
    const store = makeWebLibraryStore(deps)
    await expect(store.remove('never-existed.hew')).resolves.toBeUndefined()
    await store.write('once.hew', bytes(1))
    await store.remove('once.hew')
    await expect(store.remove('once.hew')).resolves.toBeUndefined()
  })

  it('useBrowserStorage switches back and leaves the folder contents alone', async () => {
    const folder = new FakeDir('Bound')
    const { deps } = makeDeps({
      pickDirectory: async () => folder as unknown as FileSystemDirectoryHandle,
    })
    const store = makeWebLibraryStore(deps)
    await store.chooseFolder()
    await store.write('in-folder.hew', bytes(1))
    await store.useBrowserStorage!()
    expect(await store.webStorage!()).toEqual({ mode: 'browser', needsReconnect: false })
    expect((await store.list()).map((e) => e.fileName)).toEqual([])
    expect(folder.files.has('in-folder.hew')).toBe(true)
  })
})

describe('folder permission / reconnect', () => {
  async function boundStoreNeedingPermission() {
    const folder = new FakeDir('Locked')
    folder.files.set('x.hew', { bytes: bytes(1), mtime: 1 })
    const { deps } = makeDeps({
      pickDirectory: async () => folder as unknown as FileSystemDirectoryHandle,
    })
    const store = makeWebLibraryStore(deps)
    await store.chooseFolder()
    folder.permission = 'prompt'
    folder.grantOnRequest = false // simulates requestPermission without a user gesture
    return { store, folder }
  }

  it('list() returns [] and flags needsReconnect instead of throwing', async () => {
    const { store } = await boundStoreNeedingPermission()
    expect(await store.list()).toEqual([])
    expect((await store.webStorage!()).needsReconnect).toBe(true)
  })

  it('mutations reject loudly rather than silently dropping data', async () => {
    const { store } = await boundStoreNeedingPermission()
    await expect(store.write('y.hew', bytes(2))).rejects.toThrow(/permission/)
  })

  it('reconnect() with a granted request restores access', async () => {
    const { store, folder } = await boundStoreNeedingPermission()
    await store.list()
    folder.grantOnRequest = true // the Reconnect click carries the gesture
    expect(await store.reconnect!()).toBe(true)
    expect((await store.webStorage!()).needsReconnect).toBe(false)
    expect((await store.list()).map((e) => e.fileName)).toEqual(['x.hew'])
  })
})

describe('change notification', () => {
  it('notifies local subscribers on write/remove and relays channel messages', async () => {
    const channel = fakeChannel()
    const { deps } = makeDeps({ channel })
    const store = makeWebLibraryStore(deps)
    const seen = vi.fn()
    store.subscribe(seen)
    await store.write('a.hew', bytes(1))
    expect(seen).toHaveBeenCalledTimes(1)
    await store.remove('a.hew')
    expect(seen).toHaveBeenCalledTimes(2)
    channel.emit() // another tab wrote something
    expect(seen).toHaveBeenCalledTimes(3)
  })
})
