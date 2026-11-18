/**
 * The SDK runtime: what `globalThis.hew` is inside a plugin's Worker.
 *
 * `createRuntime` builds it over a `post` function and returns `receive` for
 * the messages the host sends; `runtime.ts` wires both to the realm port.
 * Nothing here trusts the host less than the host trusts the realm (the
 * plugin shares this global and can bypass all of it), so this file only
 * aims to be correct for a plugin that uses it as documented, and to keep
 * every message inside the limits the host enforces.
 */

import { HewApiClient, HewApiError, type JsonRpcRequest, type JsonRpcResponse } from '../../../app/src/api/hewApi.gen'
import {
  RUNTIME_LIMITS,
  type ActivationReason,
  type ConsoleLevel,
  type ForwardReplyJson,
  type HostToRealm,
  type RealmToHost,
  type StorageOp,
  type StorageScope,
  type UnloadReason,
} from './protocol'

/** A refusal a command handler answers with: `throw new HewRefusal(name,
 *  explanation, detail)`. `name` is snake_case and is what the caller
 *  branches on. */
export class HewRefusal extends Error {
  readonly explanation: string
  readonly detail: unknown
  constructor(name: string, explanation: string, detail?: unknown) {
    super(explanation)
    if (!/^[a-z][a-z0-9_]{0,63}$/.test(name)) {
      throw new TypeError(`HewRefusal: "${String(name).slice(0, 80)}" is not a snake_case refusal name`)
    }
    this.name = name
    this.explanation = String(explanation)
    this.detail = detail === undefined ? null : detail
  }
}

/** What a command handler is given beside its params. */
export interface HandlerContext {
  /** Who called the command (`identity`, `plugin`, `app`). */
  caller: unknown
  /** Aborts when the caller cancels, the call times out, or the plugin stops. */
  signal: AbortSignal
  /** Reports how far along the command is (0 to 1, or null when unknown). */
  progress(fraction: number | null, label?: string): void
  document: string
  activation: ActivationReason | null
}

export type CommandHandler = (params: unknown, ctx: HandlerContext) => unknown

export interface PluginInfo {
  id: string
  version: string
  grant: unknown
  api: 1
  document: string
  /** The document's own persistent id, or null while it has none. */
  documentId: string | null
  features: string[]
}

/** One storage scope's calls. */
export interface StorageArea {
  get(key: string): Promise<unknown>
  set(key: string, value: unknown): Promise<void>
  delete(key: string): Promise<void>
  list(): Promise<string[]>
  onChange(fn: (keys: string[]) => void): () => void
}

/** The `hew` global a plugin sees. */
export interface Hew {
  /** Resolves once the host has introduced itself; `plugin`, `locale` and
   *  `theme` are null before. */
  readonly ready: Promise<void>
  readonly plugin: PluginInfo | null
  readonly locale: string | null
  readonly theme: 'light' | 'dark' | null
  readonly api: HewApiClient
  transact(commands: unknown[], opts?: { label?: string; expect?: number; coalesce?: string }): Promise<unknown>
  commit(fn: (doc: unknown) => unknown[] | Promise<unknown[]>, opts?: { label?: string }): Promise<unknown>
  readonly commands: { handle(local: string, handler: CommandHandler): void }
  readonly events: { on(name: string, fn: (params: unknown) => void): void; off(name: string, fn: (params: unknown) => void): void }
  /** The plugin's own storage, and (`document`) its storage for the
   *  document it has now: kept on this device under the document's id,
   *  not in the file, not part of Undo. A `document` call made before the
   *  document changed and reaching Hew after is refused
   *  `document_changed`, with nothing read or written. */
  readonly storage: StorageArea & { readonly document: StorageArea }
  readonly assets: { bytes(path: string): Promise<ArrayBuffer>; url(path: string): Promise<string> }
  readonly lifecycle: {
    /** `unload`: the realm is stopping, with the reason (`disable`,
     *  `reload`, `window_closed`, `idle`). A handler may return a promise;
     *  storage writes it makes are kept, and the realm waits for it up to
     *  `RUNTIME_LIMITS.unloadWaitMs`. `grant`: the person granted the plugin
     *  more while it runs; the handler gets the new grant (also in
     *  `hew.plugin.grant`), and events it was refused are asked for again. */
    on(event: 'unload', fn: (reason: UnloadReason) => unknown): void
    on(event: 'grant', fn: (grant: unknown) => void): void
    /** `document`: the realm's document changed (another opened, or the
     *  first save gave it an id); the handler gets the new id or null. */
    on(event: 'document', fn: (documentId: string | null) => void): void
  }
  supports(name: string): boolean
}

export interface RuntimeEnv {
  /** Sends one message to the host. */
  post(message: RealmToHost): void
  /** The console the plugin's own calls go to before they are mirrored. */
  console: Pick<Console, ConsoleLevel>
  now(): number
}

export interface Runtime {
  hew: Hew
  receive(message: unknown): void
  /** Reports an error nothing caught (the Worker's `error` event). */
  reportError(message: string, stack?: string): void
}

const cut = (text: string, max: number): string => (text.length > max ? text.slice(0, max - 1) + '…' : text)

/** One console argument as text. */
function consoleText(value: unknown): string {
  if (typeof value === 'string') return cut(value, RUNTIME_LIMITS.consoleArgChars)
  if (value instanceof Error) return cut(`${value.name}: ${value.message}`, RUNTIME_LIMITS.consoleArgChars)
  try {
    const json = JSON.stringify(value)
    if (typeof json === 'string') return cut(json, RUNTIME_LIMITS.consoleArgChars)
  } catch {
    /* a cycle or a BigInt: fall back to String */
  }
  try {
    return cut(String(value), RUNTIME_LIMITS.consoleArgChars)
  } catch {
    return '[unprintable]'
  }
}

const STORAGE_TEXT: Record<string, string> = {
  storage_quota: "This plugin's storage is full.",
  storage_key_invalid: 'A storage key must be text of 1 to 256 bytes.',
  storage_value_invalid: 'A stored value must be well-formed text: no lone surrogate.',
  storage_value_too_large: `A stored value must be at most ${RUNTIME_LIMITS.storageValueChars} characters of JSON.`,
  storage_unavailable: "This plugin's storage cannot be reached.",
  document_changed: 'The document changed before this call reached Hew, so nothing was read or written. Try again for the document open now.',
}

/** Whether `text` is well-formed UTF-16: no surrogate without its pair. */
export function wellFormed(text: string): boolean {
  for (let i = 0; i < text.length; i++) {
    const c = text.charCodeAt(i)
    if (c < 0xd800 || c > 0xdfff) continue
    if (c > 0xdbff || i + 1 >= text.length || (text.charCodeAt(i + 1) & 0xfc00) !== 0xdc00) return false
    i++
  }
  return true
}

/** The UTF-8 length of `text`, counting no further than `max + 1`. */
export function utf8Bytes(text: string, max: number): number {
  let n = 0
  for (let i = 0; i < text.length && n <= max; i++) {
    const c = text.charCodeAt(i)
    if (c < 0x80) n += 1
    else if (c < 0x800) n += 2
    else if (c >= 0xd800 && c <= 0xdbff && i + 1 < text.length && (text.charCodeAt(i + 1) & 0xfc00) === 0xdc00) {
      n += 4
      i++
    } else n += 3
  }
  return n
}

/**
 * Runs at most `limit` of the tasks given to it at once; the rest wait their
 * turn, in order. Keeps the requests a plugin has open within what the host
 * allows, however many the plugin makes.
 */
function limiter(limit: number): <T>(task: () => Promise<T>) => Promise<T> {
  let running = 0
  const queue: (() => void)[] = []
  const next = (): void => {
    if (running >= limit) return
    const go = queue.shift()
    if (go === undefined) return
    running++
    go()
  }
  return <T>(task: () => Promise<T>): Promise<T> =>
    new Promise<T>((resolve, reject) => {
      queue.push(() => {
        task()
          .then(resolve, reject)
          .finally(() => {
            running--
            next()
          })
      })
      next()
    })
}

export function createRuntime(env: RuntimeEnv): Runtime {
  let info: { plugin: PluginInfo; locale: string; theme: 'light' | 'dark'; activation: ActivationReason } | null = null
  let markReady: () => void = () => {}
  const ready = new Promise<void>((resolve) => {
    markReady = resolve
  })

  // Until the host has said hello, everything waits here, in order.
  let outbox: RealmToHost[] | null = []
  const send = (message: RealmToHost): void => {
    if (outbox !== null) outbox.push(message)
    else env.post(message)
  }

  // ------------------------------------------------------------ console
  let consoleWindow = 0
  let consoleCount = 0
  let consoleDropped = 0
  /** Mirrors one console call to the host; false when it is over the
   *  per-second budget and dropped. */
  const mirror = (level: ConsoleLevel, args: unknown[]): boolean => {
    const now = env.now()
    if (now - consoleWindow >= 1000) {
      if (consoleDropped > 0) {
        send({ t: 'console', level: 'warn', args: [`${consoleDropped} console messages were dropped`], at: now })
      }
      consoleWindow = now
      consoleCount = 0
      consoleDropped = 0
    }
    if (consoleCount >= RUNTIME_LIMITS.consolePerSecond) {
      consoleDropped++
      return false
    }
    consoleCount++
    send({ t: 'console', level, args: args.slice(0, RUNTIME_LIMITS.consoleArgs).map(consoleText), at: now })
    return true
  }

  const reportError = (message: string, stack?: string): void => {
    const out: RealmToHost = { t: 'error', message: cut(message, RUNTIME_LIMITS.errorMessageChars) }
    if (typeof stack === 'string' && stack !== '') out.stack = cut(stack, RUNTIME_LIMITS.errorStackChars)
    send(out)
  }

  // ---------------------------------------------------------------- bus
  const pendingBus = new Map<string | number, { resolve(r: JsonRpcResponse): void; reject(e: unknown): void }>()
  let lastGeneration: number | undefined
  const noteGeneration = (value: unknown): void => {
    if (typeof value === 'object' && value !== null) {
      const g = (value as { generation?: unknown }).generation
      if (typeof g === 'number' && Number.isSafeInteger(g)) lastGeneration = g
    }
  }

  // The forward a request is made while serving: the one open forward, or,
  // with several, the one entered last (a handler must not interleave bus
  // calls across concurrent handlers; mutating ones never run concurrently).
  const openForwards = new Map<number, { controller: AbortController; cancelled: boolean }>()
  let lastEntered: number | undefined
  const activeForward = (): number | undefined => {
    if (openForwards.size === 0) return undefined
    if (openForwards.size === 1) return openForwards.keys().next().value
    return lastEntered !== undefined && openForwards.has(lastEntered) ? lastEntered : undefined
  }

  const dispatch = (request: JsonRpcRequest): Promise<JsonRpcResponse> => {
    const frame = JSON.stringify(request)
    if (frame.length > RUNTIME_LIMITS.frameChars) {
      return Promise.reject(new RangeError(`the request to ${request.method} is too large to send`))
    }
    return new Promise((resolve, reject) => {
      pendingBus.set(request.id, { resolve, reject })
      const forward = activeForward()
      send(forward === undefined ? { t: 'bus', frame } : { t: 'bus', forward, frame })
    })
  }
  const api = new HewApiClient({ dispatch })

  let directId = 0
  const call = async (method: string, params: unknown): Promise<unknown> => {
    const response = await dispatch({ jsonrpc: '2.0', id: `h${++directId}`, method, params })
    if (response.error) throw new HewApiError(response.error)
    return response.result
  }

  const eventListeners = new Map<string, Set<(params: unknown) => void>>()
  /** Event names the connection's grant refused at subscribe time, asked
   *  for again when a wider grant arrives. */
  const deniedEvents = new Set<string>()
  const subscribeEvent = (name: string): void => {
    api.event
      .subscribe({ events: [name as never] })
      .then((result) => {
        const denied = (result as { denied?: unknown } | null)?.denied
        if (Array.isArray(denied) && denied.includes(name) && eventListeners.has(name)) deniedEvents.add(name)
        else deniedEvents.delete(name)
      })
      .catch((e: unknown) => mirror('warn', [`could not subscribe to ${name}`, e]))
  }
  const onBusFrame = (frame: string): void => {
    let parsed: unknown
    try {
      parsed = JSON.parse(frame)
    } catch {
      return
    }
    if (typeof parsed !== 'object' || parsed === null) return
    const msg = parsed as { id?: unknown; method?: unknown; params?: unknown; result?: unknown }
    if (typeof msg.id === 'string' || typeof msg.id === 'number') {
      const waiter = pendingBus.get(msg.id)
      if (waiter === undefined) return
      pendingBus.delete(msg.id)
      noteGeneration(msg.result)
      waiter.resolve(parsed as JsonRpcResponse)
      return
    }
    if (typeof msg.method === 'string' && msg.method.startsWith('hew.event.')) {
      const name = msg.method.slice('hew.event.'.length)
      if (name === 'document_changed') noteGeneration(msg.params)
      for (const fn of [...(eventListeners.get(name) ?? [])]) {
        try {
          fn(msg.params)
        } catch (e) {
          mirror('error', [`a listener for ${name} threw`, e])
        }
      }
    }
  }

  // ------------------------------------------------------------ storage
  /** The host's count of document changes, as last heard (`hello`, then
   *  each `document`); unknown before hello. */
  let documentEpoch: number | undefined
  /** The epoch hello named: what a call made before hello is for, even
   *  when it waits for a slot past a later document change. */
  let helloEpoch: number | undefined
  const readEpoch = (value: unknown): number | undefined => (typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : undefined)
  let nextRequest = 0
  const pendingStorage = new Map<number, { resolve(m: Extract<HostToRealm, { t: 'storage_reply' }>): void }>()
  const storageSlot = limiter(RUNTIME_LIMITS.storageOpen)
  /** Storage requests made and not yet answered, and who waits for none. */
  let storageOpen = 0
  let storageIdle: (() => void)[] = []
  const storageSettled = (): void => {
    if (--storageOpen > 0) return
    const waiting = storageIdle
    storageIdle = []
    for (const fn of waiting) fn()
  }
  const storage = (scope: StorageScope, op: StorageOp, key?: string, value?: string): Promise<Extract<HostToRealm, { t: 'storage_reply' }>> => {
    storageOpen++
    // A document call is for the document the plugin had when it made the
    // call, even when it waits for a slot past a document change: the host
    // then refuses it `document_changed`. Made before hello, it is for the
    // document hello names (filled in when the outbox is sent, or, for one
    // that waits for a slot, taken from the epoch hello named).
    const epochAtCall = documentEpoch
    const request = storageSlot(
      () =>
        new Promise<Extract<HostToRealm, { t: 'storage_reply' }>>((resolve) => {
          const id = ++nextRequest
          pendingStorage.set(id, { resolve })
          const msg: RealmToHost = { t: 'storage', id, op, scope }
          if (key !== undefined) msg.key = key
          if (value !== undefined) msg.value = value
          const epoch = epochAtCall ?? helloEpoch
          if (scope === 'document' && epoch !== undefined) msg.epoch = epoch
          send(msg)
        }),
    )
    void request.finally(storageSettled)
    return request
  }
  const storageFailed = (reply: Extract<HostToRealm, { t: 'storage_reply' }>): HewRefusal | null => {
    if (reply.ok) return null
    const name = /^[a-z][a-z0-9_]{0,63}$/.test(reply.error) ? reply.error : 'storage_unavailable'
    return new HewRefusal(name, STORAGE_TEXT[name] ?? STORAGE_TEXT.storage_unavailable)
  }
  const checkKey = (key: unknown): string => {
    if (typeof key !== 'string' || key === '') throw new TypeError('a storage key is a non-empty string')
    if (!wellFormed(key) || utf8Bytes(key, RUNTIME_LIMITS.storageKeyBytes) > RUNTIME_LIMITS.storageKeyBytes) {
      throw new HewRefusal('storage_key_invalid', STORAGE_TEXT.storage_key_invalid)
    }
    return key
  }
  const storageListeners: Record<StorageScope, Set<(keys: string[]) => void>> = { plugin: new Set(), document: new Set() }
  const documentListeners: ((documentId: string | null) => void)[] = []

  /** The calls of one storage scope. The same key and value checks and
   *  refusal names in both. */
  function area(scope: StorageScope): StorageArea {
    return {
      async get(key) {
        const reply = await storage(scope, 'get', checkKey(key))
        const failed = storageFailed(reply)
        if (failed !== null) throw failed
        const value = reply.ok ? reply.value : null
        return typeof value === 'string' ? JSON.parse(value) : undefined
      },
      async set(key, value) {
        const k = checkKey(key)
        const text = JSON.stringify(value)
        if (typeof text !== 'string') throw new TypeError('a stored value must be JSON (not undefined or a function)')
        if (text.length > RUNTIME_LIMITS.storageValueChars) {
          throw new HewRefusal('storage_value_too_large', STORAGE_TEXT.storage_value_too_large)
        }
        const failed = storageFailed(await storage(scope, 'set', k, text))
        if (failed !== null) throw failed
      },
      async delete(key) {
        const failed = storageFailed(await storage(scope, 'delete', checkKey(key)))
        if (failed !== null) throw failed
      },
      async list() {
        const reply = await storage(scope, 'list')
        const failed = storageFailed(reply)
        if (failed !== null) throw failed
        return reply.ok ? [...(reply.keys ?? [])] : []
      },
      onChange(fn) {
        storageListeners[scope].add(fn)
        return () => {
          storageListeners[scope].delete(fn)
        }
      },
    }
  }

  // -------------------------------------------------------------- files
  const pendingFiles = new Map<number, { path: string; resolve(b: ArrayBuffer): void; reject(e: unknown): void }>()
  const fileSlot = limiter(RUNTIME_LIMITS.filesOpen)

  // ---------------------------------------------------------- commands
  const handlers = new Map<string, CommandHandler>()
  const unloadListeners: ((reason: UnloadReason) => unknown)[] = []
  const grantListeners: ((grant: unknown) => void)[] = []

  const replyForward = (id: number, reply: ForwardReplyJson): void => {
    const entry = openForwards.get(id)
    openForwards.delete(id)
    // The host already answered a cancelled forward.
    if (entry?.cancelled === true) return
    let text: string
    try {
      text = JSON.stringify(reply)
    } catch (e) {
      reportError(`the result could not be sent: ${e instanceof Error ? e.message : String(e)}`)
      text = '{"failed":"crashed"}'
    }
    if (text.length > RUNTIME_LIMITS.frameChars) {
      reportError('the result is too large to send')
      text = '{"failed":"crashed"}'
    }
    send({ t: 'forward_reply', id, reply: text })
  }

  const onForward = (m: Extract<HostToRealm, { t: 'forward' }>): void => {
    const handler = handlers.get(m.local)
    if (handler === undefined) {
      send({ t: 'forward_reply', id: m.id, reply: '{"failed":"no_handler"}' })
      return
    }
    const controller = new AbortController()
    openForwards.set(m.id, { controller, cancelled: false })
    lastEntered = m.id
    let params: unknown
    let caller: unknown
    try {
      params = JSON.parse(m.params)
      caller = JSON.parse(m.caller)
    } catch {
      replyForward(m.id, { failed: 'crashed' })
      return
    }
    const ctx: HandlerContext = {
      caller,
      signal: controller.signal,
      progress(fraction, label) {
        if (!openForwards.has(m.id)) return
        const f = typeof fraction === 'number' && Number.isFinite(fraction) ? Math.min(1, Math.max(0, fraction)) : null
        const msg: RealmToHost = { t: 'forward_progress', id: m.id, fraction: f }
        if (typeof label === 'string') msg.label = cut(label, RUNTIME_LIMITS.progressLabelChars)
        send(msg)
      },
      document: info?.plugin.document ?? '',
      activation: info?.activation ?? null,
    }
    let result: unknown
    try {
      result = handler(params, ctx)
    } catch (e) {
      settleThrown(m.id, e)
      return
    }
    Promise.resolve(result).then(
      (value) => replyForward(m.id, { ok: value === undefined ? null : value }),
      (e: unknown) => settleThrown(m.id, e),
    )
  }

  const settleThrown = (id: number, e: unknown): void => {
    // Once the caller gave up, the handler failing is expected (its signal
    // aborted, its calls refused `forward_ended`): nobody is told.
    if (openForwards.get(id)?.cancelled === true) {
      openForwards.delete(id)
      return
    }
    if (e instanceof HewRefusal) {
      replyForward(id, { refusal: { name: e.name, explanation: cut(e.explanation, RUNTIME_LIMITS.refusalExplanationChars), detail: e.detail } })
      return
    }
    // A handler that fails with anything but a refusal is a plugin bug: the
    // caller is told it crashed, and the host stops the plugin.
    replyForward(id, { failed: 'crashed' })
    reportError(e instanceof Error ? `${e.name}: ${e.message}` : `a command handler failed: ${consoleText(e)}`, e instanceof Error ? e.stack : undefined)
  }

  // ------------------------------------------------------------ receive
  let unloading = false
  const receive = (message: unknown): void => {
    if (typeof message !== 'object' || message === null) return
    const m = message as HostToRealm
    switch (m.t) {
      case 'hello': {
        if (info !== null) return
        let grant: unknown = null
        try {
          grant = JSON.parse(m.grant)
        } catch {
          /* the host sends valid JSON; null if not */
        }
        info = {
          plugin: {
            id: m.plugin,
            version: m.version,
            grant,
            api: 1,
            document: m.document,
            documentId: typeof m.document_id === 'string' ? m.document_id : null,
            features: [...m.features],
          },
          locale: m.locale,
          theme: m.theme,
          activation: m.activation,
        }
        documentEpoch = readEpoch(m.document_epoch) ?? 0
        helloEpoch = documentEpoch
        env.post({ t: 'hello_ack' })
        const queued = outbox ?? []
        outbox = null
        for (const q of queued) {
          if (q.t === 'storage' && q.scope === 'document' && q.epoch === undefined) q.epoch = documentEpoch
          env.post(q)
        }
        markReady()
        return
      }
      case 'bus':
        onBusFrame(m.frame)
        return
      case 'forward':
        onForward(m)
        return
      case 'forward_cancel': {
        const entry = openForwards.get(m.id)
        if (entry !== undefined) {
          entry.cancelled = true
          entry.controller.abort()
        }
        return
      }
      case 'storage_reply': {
        const waiter = pendingStorage.get(m.id)
        if (waiter === undefined) return
        pendingStorage.delete(m.id)
        waiter.resolve(m)
        return
      }
      case 'storage_changed':
        for (const fn of [...storageListeners[m.scope === 'document' ? 'document' : 'plugin']]) {
          try {
            fn([...m.keys])
          } catch (e) {
            mirror('error', ['a storage listener threw', e])
          }
        }
        return
      case 'files': {
        const waiter = pendingFiles.get(m.id)
        if (waiter === undefined) return
        pendingFiles.delete(m.id)
        const entry = m.entries.find((e) => e.path === waiter.path)
        if (entry !== undefined) waiter.resolve(entry.bytes)
        else waiter.reject(new HewRefusal('file_missing', `The plugin has no file "${cut(waiter.path, 200)}".`))
        return
      }
      case 'unload': {
        if (unloading) return
        unloading = true
        for (const [, entry] of openForwards) entry.controller.abort()
        const waits: Promise<unknown>[] = []
        for (const fn of unloadListeners) {
          try {
            const result = fn(m.reason)
            if (typeof (result as { then?: unknown } | null)?.then === 'function') {
              waits.push(Promise.resolve(result).catch((e: unknown) => mirror('error', ['an unload listener failed', e])))
            }
          } catch (e) {
            mirror('error', ['an unload listener threw', e])
          }
        }
        // `unloaded` goes after the storage writes the listeners made, so
        // the host keeps them; it waits for them and for the listeners'
        // promises, at most `unloadWaitMs`.
        const storageDone = (): Promise<void> => (storageOpen === 0 ? Promise.resolve() : new Promise<void>((resolve) => storageIdle.push(resolve)))
        if (waits.length === 0 && storageOpen === 0) {
          env.post({ t: 'unloaded' })
          return
        }
        let timer: ReturnType<typeof setTimeout> | undefined
        const limit = new Promise<void>((resolve) => {
          timer = setTimeout(resolve, RUNTIME_LIMITS.unloadWaitMs)
        })
        void Promise.race([Promise.allSettled(waits).then(storageDone), limit]).then(() => {
          clearTimeout(timer)
          env.post({ t: 'unloaded' })
        })
        return
      }
      case 'document': {
        if (info === null) return
        const documentId = typeof m.document_id === 'string' ? m.document_id : null
        info.plugin.documentId = documentId
        documentEpoch = readEpoch(m.epoch) ?? documentEpoch
        for (const fn of [...documentListeners]) {
          try {
            fn(documentId)
          } catch (e) {
            mirror('error', ['a document listener threw', e])
          }
        }
        return
      }
      case 'grant': {
        if (info === null || typeof m.grant !== 'string') return
        let grant: unknown = null
        try {
          grant = JSON.parse(m.grant)
        } catch {
          return
        }
        info.plugin.grant = grant
        // Events the old grant refused are asked for again; one the new
        // grant still refuses comes back denied and waits for the next.
        for (const name of [...deniedEvents]) {
          if (eventListeners.has(name)) subscribeEvent(name)
          else deniedEvents.delete(name)
        }
        for (const fn of [...grantListeners]) {
          try {
            fn(grant)
          } catch (e) {
            mirror('error', ['a grant listener threw', e])
          }
        }
        return
      }
      case 'ping':
        // Answered at once, even before hello: it is how the host knows the
        // Worker is alive.
        env.post({ t: 'pong', n: m.n })
        return
    }
  }

  // ---------------------------------------------------------------- hew
  const hew: Hew = {
    ready,
    get plugin() {
      return info?.plugin ?? null
    },
    get locale() {
      return info?.locale ?? null
    },
    get theme() {
      return info?.theme ?? null
    },
    api,
    transact(commands, opts = {}) {
      const params: Record<string, unknown> = { commands }
      if (opts.label !== undefined) params.label = opts.label
      if (opts.coalesce !== undefined) params.coalesce = opts.coalesce
      const expect = opts.expect ?? lastGeneration
      if (expect !== undefined) params.expect_generation = expect
      return call('hew.doc.transact', params)
    },
    async commit(fn, opts = {}) {
      for (let attempt = 0; ; attempt++) {
        const doc = await api.query.scene({})
        const commands = await fn(doc)
        try {
          return await hew.transact(commands, { label: opts.label, expect: doc.generation })
        } catch (e) {
          if (attempt === 0 && e instanceof HewApiError && e.refusal?.refusal === 'generation_moved') continue
          throw e
        }
      }
    },
    commands: {
      handle(local, handler) {
        if (typeof local !== 'string' || local === '') throw new TypeError('a command id is a non-empty string')
        if (typeof handler !== 'function') throw new TypeError('a command handler is a function')
        handlers.set(local, handler)
      },
    },
    events: {
      on(name, fn) {
        let set = eventListeners.get(name)
        if (set === undefined) {
          set = new Set()
          eventListeners.set(name, set)
          subscribeEvent(name)
        }
        set.add(fn)
      },
      off(name, fn) {
        const set = eventListeners.get(name)
        if (set === undefined) return
        set.delete(fn)
        if (set.size === 0) {
          eventListeners.delete(name)
          deniedEvents.delete(name)
          api.event.unsubscribe({ events: [name as never] }).catch(() => {})
        }
      },
    },
    storage: Object.assign(area('plugin'), { document: area('document') }),
    assets: {
      bytes(path) {
        if (typeof path !== 'string' || path === '') return Promise.reject(new TypeError('a path is a non-empty string'))
        if (path.length > RUNTIME_LIMITS.pathChars) {
          return Promise.reject(new HewRefusal('file_missing', 'The plugin has no file with a path that long.'))
        }
        return fileSlot(
          () =>
            new Promise<ArrayBuffer>((resolve, reject) => {
              const id = ++nextRequest
              pendingFiles.set(id, { path, resolve, reject })
              send({ t: 'files', id, paths: [path] })
            }),
        )
      },
      async url(path) {
        const bytes = await hew.assets.bytes(path)
        return URL.createObjectURL(new Blob([bytes]))
      },
    },
    lifecycle: {
      on(event: 'unload' | 'grant' | 'document', fn: (arg: never) => unknown) {
        if (typeof fn !== 'function') return
        if (event === 'unload') unloadListeners.push(fn as (reason: UnloadReason) => unknown)
        else if (event === 'grant') grantListeners.push(fn as (grant: unknown) => void)
        else if (event === 'document') documentListeners.push(fn as (documentId: string | null) => void)
      },
    },
    supports(name) {
      return info?.plugin.features.includes(name) ?? false
    },
  }

  // Mirror the plugin's console to the host, after the Worker's own. A
  // call over the per-second budget goes to neither: with developer tools
  // attached the Worker's own console is costly, and a flood of it alone
  // can keep the Worker from answering the host's pings.
  for (const level of ['log', 'info', 'warn', 'error'] as const) {
    const original = env.console[level].bind(env.console)
    env.console[level] = (...args: unknown[]) => {
      if (mirror(level, args)) original(...args)
    }
  }

  return { hew, receive, reportError }
}
