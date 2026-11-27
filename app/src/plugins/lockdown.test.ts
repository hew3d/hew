/**
 * The realm Worker's lockdown (`sdk/plugin/src/lockdown.ts`) against a fake
 * Worker global shaped like the engines' (members on the global, on the
 * scope's prototypes, and on `navigator`'s prototype), and its recursion
 * into nested Workers.
 */

import { beforeEach, describe, expect, it } from 'vitest'
import { requiresPreview } from '../test/requiresPreview'
import { lockdown } from '../../../sdk/plugin/src/lockdown'

/** A fake dedicated Worker global: interfaces on the global itself, members
 *  on its prototype chain as accessors and methods, as both engines lay
 *  them out. */
function fakeGlobal() {
  const blobs = new Map<string, Blob>()
  let n = 0
  class FakeURL {
    static createObjectURL(obj: Blob): string {
      const url = `blob:fake/${++n}`
      blobs.set(url, obj)
      return url
    }
    static revokeObjectURL(url: string): void {
      blobs.delete(url)
    }
  }
  const started: { url: string; options?: WorkerOptions }[] = []
  class FakeWorker {
    constructor(url: string, options?: WorkerOptions) {
      started.push({ url, options })
    }
  }
  const scopeProto = {
    fetch() {},
    importScripts() {},
    get caches() {
      return { open() {} }
    },
    get indexedDB() {
      return {}
    },
    setTimeout,
    structuredClone,
    get crypto() {
      return {}
    },
  }
  const navProto = {
    get locks() {
      return {}
    },
    get serviceWorker() {
      return {}
    },
    get hardwareConcurrency() {
      return 4
    },
  }
  const g = Object.create(Object.create(scopeProto)) as Record<string, unknown>
  Object.assign(g, {
    URL: FakeURL,
    Blob,
    Worker: FakeWorker,
    WebAssembly,
    MessageChannel,
    TextEncoder,
    BroadcastChannel: class {},
    CacheStorage: class {},
    IDBFactory: class {},
    XMLHttpRequest: class {},
    WebSocket: class {},
    navigator: Object.create(navProto),
  })
  return { g, blobs, started, FakeWorker, scopeProto, navProto }
}

const REMOVED = ['caches', 'indexedDB', 'fetch', 'importScripts', 'BroadcastChannel', 'CacheStorage', 'IDBFactory', 'XMLHttpRequest', 'WebSocket']

describe('the realm Worker lockdown', () => {
  beforeEach((ctx) => requiresPreview(ctx, 'plugins'))

  it('removes each API from the global and its prototype chain, and leaves a hole that cannot be filled', () => {
    const f = fakeGlobal()
    lockdown(f.g as unknown as typeof globalThis)
    for (const name of REMOVED) {
      expect(f.g[name], name).toBeUndefined()
      for (let o = Object.getPrototypeOf(f.g); o !== null; o = Object.getPrototypeOf(o)) {
        expect(Object.prototype.hasOwnProperty.call(o, name), `${name} on a prototype`).toBe(false)
      }
      const d = Object.getOwnPropertyDescriptor(f.g, name)
      expect(d, name).toMatchObject({ value: undefined, writable: false, configurable: false })
      expect(() => {
        'use strict'
        f.g[name] = 1
      }).toThrow(TypeError)
    }
    expect(f.scopeProto).not.toHaveProperty('fetch')
  })

  it('removes navigator.locks and navigator.serviceWorker, and keeps the rest of navigator', () => {
    const f = fakeGlobal()
    lockdown(f.g as unknown as typeof globalThis)
    const nav = f.g.navigator as Record<string, unknown>
    expect(nav.locks).toBeUndefined()
    expect(nav.serviceWorker).toBeUndefined()
    expect(Object.getOwnPropertyDescriptor(f.navProto, 'locks')).toBeUndefined()
    expect(nav.hardwareConcurrency).toBe(4)
  })

  it('keeps what a plugin needs', () => {
    const f = fakeGlobal()
    lockdown(f.g as unknown as typeof globalThis)
    for (const name of ['Blob', 'WebAssembly', 'MessageChannel', 'TextEncoder', 'setTimeout', 'structuredClone', 'crypto']) {
      expect(f.g[name], name).toBeDefined()
    }
    expect(typeof f.g.Worker).toBe('function')
    expect(typeof (f.g.URL as typeof URL).createObjectURL).toBe('function')
  })

  it('starts a nested Worker only from a blob URL the plugin made, with the lockdown first', async () => {
    const f = fakeGlobal()
    lockdown(f.g as unknown as typeof globalThis)
    const W = f.g.Worker as unknown as new (url: string, options?: WorkerOptions) => object
    const U = f.g.URL as typeof URL
    const own = U.createObjectURL(new Blob(['postMessage(1)']))
    const w = new W(own)
    expect(w).toBeInstanceOf(f.FakeWorker)
    expect(Object.getPrototypeOf(w).constructor).toBe(W)
    expect(f.started).toHaveLength(1)
    // The Worker that starts runs a new blob: the lockdown, then the plugin's.
    const text = await f.blobs.get(f.started[0].url)!.text()
    expect(text.startsWith('(function lockdown(')).toBe(true)
    expect(text.endsWith('\n;\npostMessage(1)')).toBe(true)

    expect(() => new W('data:text/javascript,1')).toThrow(expect.objectContaining({ name: 'SecurityError' }))
    expect(() => new W('blob:fake/999')).toThrow(expect.objectContaining({ name: 'SecurityError' }))
    expect(() => new W(own, { type: 'module' })).toThrow(expect.objectContaining({ name: 'SecurityError' }))
    expect(() => (W as unknown as (u: string) => void)(own)).toThrow(TypeError)
    U.revokeObjectURL(own)
    expect(() => new W(own)).toThrow(expect.objectContaining({ name: 'SecurityError' }))
    // Not reachable again through the global or the prototype.
    expect(Object.getOwnPropertyDescriptor(f.g, 'Worker')).toMatchObject({ writable: false, configurable: false })
    expect(Object.getOwnPropertyDescriptor(f.g.URL, 'createObjectURL')).toMatchObject({ writable: false, configurable: false })
    expect(f.started).toHaveLength(1)
  })

  it('locks a nested Worker the same way, and the one that one starts', async () => {
    const outer = fakeGlobal()
    lockdown(outer.g as unknown as typeof globalThis)
    const W = outer.g.Worker as unknown as new (url: string) => object
    new W((outer.g.URL as typeof URL).createObjectURL(new Blob([''])))
    const source = (await outer.blobs.get(outer.started[0].url)!.text()).replace(/\n;\n$/, '')
    // What the nested Worker runs first, on its own fresh global.
    const nested = fakeGlobal()
    new Function('self', source)(nested.g)
    for (const name of REMOVED) expect(nested.g[name], name).toBeUndefined()
    expect((nested.g.navigator as Record<string, unknown>).locks).toBeUndefined()
    const W2 = nested.g.Worker as unknown as new (url: string) => object
    expect(() => new W2('data:text/javascript,1')).toThrow(expect.objectContaining({ name: 'SecurityError' }))
    new W2((nested.g.URL as typeof URL).createObjectURL(new Blob(['x'])))
    const again = await nested.blobs.get(nested.started[0].url)!.text()
    expect(again.startsWith('(function lockdown(')).toBe(true)
    expect(again.endsWith('\n;\nx')).toBe(true)
  })
})
