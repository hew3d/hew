/**
 * Locks a plugin Worker's global scope down before any plugin code runs:
 * the storage, network and messaging APIs a plugin has no sanctioned use
 * for are removed from the global object, from every object on its
 * prototype chain, and from `navigator` and its chain, and the holes are
 * filled with non-configurable `undefined` where the engine allows. A
 * plugin keeps what it needs: Workers, Blobs and blob URLs, WebAssembly,
 * message channels, crypto, timers, text coding, `structuredClone`,
 * `performance`, typed arrays, and the console the runtime wraps.
 *
 * `Worker` is replaced: a nested Worker runs this same lockdown first (a
 * blob of this function's source followed by the plugin's own blob), so a
 * fresh global gets nothing back. Only a blob URL the plugin made here
 * (`URL.createObjectURL` remembers each) can start one, and only a classic
 * Worker: a `type: 'module'` Worker is refused, since a module could not
 * be given the lockdown first.
 *
 * Defense in depth, not a boundary the host relies on: what the host
 * enforces is on the port and in the frame's policy. The function is
 * self-contained (it refers to nothing outside its own body) because its
 * source text is what a nested Worker runs.
 */
export function lockdown(g: typeof globalThis): void {
  'use strict'
  // Captured before plugin code can touch anything.
  const SOURCE = `(${lockdown.toString()})(self);`
  const O = Object
  const hasOwn = (o: object, k: string): boolean => O.prototype.hasOwnProperty.call(o, k)

  /** Names on the global scope and its prototypes. */
  const GLOBAL = [
    // Storage that outlives the plugin, or is shared, outside its own store.
    'caches', 'CacheStorage', 'Cache',
    'indexedDB', 'IDBFactory', 'IDBDatabase', 'IDBTransaction', 'IDBRequest', 'IDBOpenDBRequest', 'IDBObjectStore',
    'IDBIndex', 'IDBCursor', 'IDBCursorWithValue', 'IDBKeyRange', 'IDBVersionChangeEvent', 'IDBRecord',
    'StorageManager', 'LockManager', 'Lock', 'CookieStore', 'cookieStore',
    'webkitRequestFileSystem', 'webkitRequestFileSystemSync', 'webkitResolveLocalFileSystemURL',
    'webkitResolveLocalFileSystemSyncURL', 'requestFileSystem',
    'FileSystemHandle', 'FileSystemFileHandle', 'FileSystemDirectoryHandle', 'FileSystemSyncAccessHandle',
    'FileSystemWritableFileStream',
    // Channels to other code or to the network.
    'BroadcastChannel', 'SharedWorker', 'WebSocket', 'WebSocketStream', 'WebSocketError', 'EventSource',
    'XMLHttpRequest', 'XMLHttpRequestEventTarget', 'XMLHttpRequestUpload', 'fetch', 'importScripts',
    'WebTransport', 'WebTransportError', 'WebTransportBidirectionalStream', 'WebTransportDatagramDuplexStream',
    'RTCDataChannel', 'FontFace', 'FontFaceSet', 'fonts',
    // Ambient authority a background script has no use for.
    'Notification', 'PushManager', 'PushSubscription', 'PushSubscriptionOptions',
    'ServiceWorker', 'ServiceWorkerContainer', 'ServiceWorkerRegistration', 'NavigationPreloadManager',
    'BackgroundFetchManager', 'BackgroundFetchRecord', 'BackgroundFetchRegistration',
    'SyncManager', 'PeriodicSyncManager',
  ]
  /** Names on `navigator` and its prototypes. */
  const NAVIGATOR = ['serviceWorker', 'locks', 'storage', 'storageBuckets', 'sendBeacon', 'usb', 'serial', 'hid', 'bluetooth']

  const strip = (obj: object, name: string): void => {
    for (let o: object | null = obj; o !== null; o = O.getPrototypeOf(o) as object | null) {
      if (!hasOwn(o, name)) continue
      try {
        delete (o as Record<string, unknown>)[name]
      } catch {
        /* non-configurable: overwritten below where it can be */
      }
      if (hasOwn(o, name)) {
        try {
          ;(o as Record<string, unknown>)[name] = undefined
        } catch {
          /* read-only and fixed: left, and reported by the specs */
        }
      }
    }
    try {
      O.defineProperty(obj, name, { value: undefined, writable: false, enumerable: false, configurable: false })
    } catch {
      /* already fixed on the object itself */
    }
  }

  // Worker and blob URLs, wrapped before anything is removed.
  const URL_ = g.URL
  const Blob_ = g.Blob
  const Worker_ = g.Worker as typeof Worker | undefined
  if (typeof URL_ === 'function' && typeof Blob_ === 'function') {
    const create = URL_.createObjectURL
    const revoke = URL_.revokeObjectURL
    const minted = new Map<string, Blob>()
    const createObjectURL = function createObjectURL(obj: Blob | MediaSource): string {
      const url = create.call(URL_, obj)
      if (obj instanceof Blob_) minted.set(url, obj)
      return url
    }
    const revokeObjectURL = function revokeObjectURL(url: string): void {
      minted.delete(String(url))
      revoke.call(URL_, url)
    }
    O.defineProperty(URL_, 'createObjectURL', { value: createObjectURL, writable: false, enumerable: true, configurable: false })
    O.defineProperty(URL_, 'revokeObjectURL', { value: revokeObjectURL, writable: false, enumerable: true, configurable: false })

    if (typeof Worker_ === 'function') {
      const Locked = function Worker(this: unknown, url: string | URL, options?: WorkerOptions): Worker {
        if (new.target === undefined) throw new TypeError("Failed to construct 'Worker': Please use the 'new' operator.")
        if (options != null && (options as { type?: unknown }).type === 'module') {
          throw new DOMException('A plugin starts only classic Workers.', 'SecurityError')
        }
        const blob = minted.get(String(url))
        if (blob === undefined) throw new DOMException('A plugin starts a Worker only from a blob: URL it made.', 'SecurityError')
        const inner = create.call(URL_, new Blob_([SOURCE, '\n;\n', blob], { type: 'text/javascript' }))
        return new Worker_(inner, options)
      } as unknown as typeof Worker
      O.defineProperty(Locked, 'prototype', { value: Worker_.prototype, writable: false, enumerable: false, configurable: false })
      try {
        O.defineProperty(Worker_.prototype, 'constructor', { value: Locked, writable: false, enumerable: false, configurable: false })
      } catch {
        /* the prototype's constructor is fixed: the specs report it */
      }
      for (let o: object | null = g; o !== null; o = O.getPrototypeOf(o) as object | null) {
        if (o !== g && hasOwn(o, 'Worker')) strip(o, 'Worker')
      }
      O.defineProperty(g, 'Worker', { value: Locked, writable: false, enumerable: false, configurable: false })
    }
  }

  for (const name of GLOBAL) strip(g, name)
  const nav = (g as { navigator?: object }).navigator
  if (nav !== undefined && nav !== null) for (const name of NAVIGATOR) strip(nav, name)
}
