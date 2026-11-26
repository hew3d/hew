// @vitest-environment jsdom
/**
 * The plugin host against a fake Scene and an in-memory realm that runs the
 * real SDK runtime (`sdk/plugin/src/core.ts`), every message structured-
 * cloned as a port would. What a real frame and Worker add (the policy, the
 * sandbox) is the browser specs' to prove.
 */

import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'
import { requiresPreview } from '../test/requiresPreview'
import { createRuntime, HewRefusal, type Hew } from '../../../sdk/plugin/src/core'
import { RUNTIME_LIMITS, type HostToRealm } from '../../../sdk/plugin/src/protocol'
import { getBusy, setBusy } from '../api/busyState'
import { pumpEvents, setPumpDocumentId } from '../api/eventPump'
import type { LiveBridgeDeps } from '../api/liveBridge'
import type { ForwardRequestJson } from '../api/dispatchResult'
import type { Scene } from '../wasm/loader'
import { createPluginHost, DEFAULT_TIMINGS, MAX_FORWARD_TIMEOUT_MS, MAX_PENDING_BUS, type HostScheduler, type HostTimings, forwardTimeoutMs, packagePath, readForwardReply, type HostEnv, type PluginHost, type PortLike, type RealmChannel } from './hostCore'
import type { PluginStorageBackend } from './storageBackend'
import type { PluginStoreAccess } from './storeAccess'
import type { DevChange, Installed, Listed, PluginManifest } from './types'
import { PluginError } from './pluginError'
import { compareCodePoints } from './pluginText'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')
const ID = 'com.example.hello'

// ------------------------------------------------------------------ fakes

interface Decl {
  id: string
  class: 'read_only' | 'mutating'
  timeout_ms?: number
}

/** Just enough of the kernel's dispatcher: hello, attach, a scene read,
 *  plugin commands forwarded, compounds and busy. */
function fakeScene() {
  let nextConn = 1
  let nextToken = 1
  let nextKernel = 0
  const conns = new Map<number, { plugin: string | null; permissions?: string[]; subscriptions: Set<string> }>()
  /** Events emitted and not yet drained: they go, at the drain, to every
   *  connection subscribed then, as the kernel's do. */
  const emitted: { name: string; params: unknown }[] = []
  /** What a connection's grant receives, as the kernel's `may_receive`. */
  const mayReceive = (conn: number, name: string): boolean => {
    const c = conns.get(conn)
    if (c === undefined) return false
    if (c.plugin === null) return true
    const held = c.permissions ?? []
    if (name === 'selection_changed' || name === 'tool_changed') return held.includes('view')
    if (name === 'plugin_state') return true
    return held.includes('model.read')
  }
  const registered = new Map<string, Decl[]>()
  const pending = new Map<string, { requestId: unknown; mutating: boolean; plugin: string; method: string }>()
  const compounds: { kernel: bigint; forward: number }[] = []
  const events: { conn: number; frame: string }[] = []
  let generation = 1
  const lifecycle: string[] = []
  const s = {
    docId: undefined as string | undefined,
    document_id: vi.fn((): string | undefined => s.docId),
    /** A lifecycle step, as the app notes one: drained to the host's
     *  broadcast sink. */
    lifecycle(kind: 'opened' | 'saved' | 'closed', documentId: string | null) {
      lifecycle.push(JSON.stringify({ jsonrpc: '2.0', method: `hew.event.document_${kind}`, params: { document: 'doc_1', title: 'Untitled', document_id: documentId } }))
    },
    conns,
    registered,
    compounds,
    events,
    states: [] as [string, string, string | undefined][],
    busy: vi.fn(() => compounds.length > 0),
    set_display_units: vi.fn(),
    set_ui_gesture_armed: vi.fn(),
    set_host_document_id: vi.fn(),
    api_method_mutates: vi.fn((m: string) => m === 'hew.tag.create'),
    take_pending_view_directives: vi.fn(() => '[]'),
    emitted,
    /** Queues an event the next drain sends to every connection
     *  subscribed to it then. */
    emit(name: string, params: unknown = {}) {
      emitted.push({ name, params })
    },
    drain_events: vi.fn(() => {
      const out: { conn: number | null; frames: string[] }[] = events.splice(0).map((e) => ({ conn: e.conn, frames: [e.frame] }))
      if (lifecycle.length > 0) out.push({ conn: null, frames: lifecycle.splice(0) })
      for (const e of emitted.splice(0)) {
        for (const [conn, c] of conns) {
          if (c.subscriptions.has(e.name) && mayReceive(conn, e.name)) {
            out.push({ conn, frames: [JSON.stringify({ jsonrpc: '2.0', method: `hew.event.${e.name}`, params: e.params })] })
          }
        }
      }
      return JSON.stringify(out)
    }),
    api_connection_open: vi.fn(() => {
      const id = nextConn++
      conns.set(id, { plugin: null, subscriptions: new Set() })
      return id
    }),
    api_plugin_connection_open: vi.fn((grant: string) => {
      const id = nextConn++
      const g = JSON.parse(grant) as { plugin: string; permissions: string[] }
      conns.set(id, { plugin: g.plugin, permissions: g.permissions, subscriptions: new Set() })
      return id
    }),
    // The kernel's in-place grant change: the same plugin only; what the
    // new grant cannot receive is unsubscribed.
    api_connection_set_grant: vi.fn((id: number, grant: string) => {
      const c = conns.get(id)
      const g = JSON.parse(grant) as { plugin: string; permissions: string[] }
      if (c === undefined || c.plugin === null || c.plugin !== g.plugin) throw new Error('GRANT: refused')
      c.permissions = g.permissions
      for (const name of [...c.subscriptions]) if (!mayReceive(id, name)) c.subscriptions.delete(name)
    }),
    api_connection_close: vi.fn((id: number) => {
      conns.delete(id)
    }),
    register_plugin_commands: vi.fn((id: string, json: string) => {
      registered.set(id, JSON.parse(json) as Decl[])
    }),
    unregister_plugin_commands: vi.fn((id: string) => {
      registered.delete(id)
    }),
    note_plugin_state: vi.fn((id: string, state: string, detail?: string) => {
      s.states.push([id, state, detail])
    }),
    api_dispatch: vi.fn((conn: number, frame: string) => s.run(conn, frame, null)),
    api_dispatch_in_forward: vi.fn((conn: number, token: bigint, frame: string) => s.run(conn, frame, Number(token))),
    run(conn: number, frame: string, stamp: number | null): string | undefined {
      const req = JSON.parse(frame) as { id?: unknown; method: string; params?: unknown }
      if (req.id === undefined) return undefined
      const reply = (body: object) => JSON.stringify({ reply: JSON.stringify({ jsonrpc: '2.0', id: req.id, ...body }) })
      if (!conns.has(conn)) return reply({ error: { code: -32003, message: 'unknown API connection' } })
      if (compounds.length > 0 && stamp !== compounds[compounds.length - 1].forward) {
        return reply({ error: { code: -32000, message: 'refused', data: { refusal: 'document_busy' } } })
      }
      if (req.method === 'hew.meta.hello' || req.method === 'hew.doc.attach') return reply({ result: {} })
      if (req.method === 'hew.doc.save') {
        const c = conns.get(conn)!
        if (c.plugin !== null && !(c.permissions ?? []).includes('documents.save')) {
          return reply({ error: { code: -32001, message: 'not permitted', data: { method: req.method, permission: 'documents.save' } } })
        }
        return reply({ result: { format: 'hew' } })
      }
      if (req.method === 'hew.view.enter') return reply({ result: {} })
      if (
        req.method === 'hew.doc.transact' &&
        (req.params as { commands?: { method?: string }[] } | undefined)?.commands?.[0]?.method === 'hew.view.enter'
      ) return reply({ result: { results: [{}], label: 'hew.view.enter' } })
      if (req.method === 'hew.event.subscribe' || req.method === 'hew.event.unsubscribe') {
        const c = conns.get(conn)!
        const denied: string[] = []
        for (const name of (req.params as { events: string[] }).events) {
          if (req.method === 'hew.event.unsubscribe') c.subscriptions.delete(name)
          else if (mayReceive(conn, name)) c.subscriptions.add(name)
          else denied.push(name)
        }
        return reply({ result: { subscribed: [...c.subscriptions].sort(), denied } })
      }
      // Served by the page's host for a plugin's own realm; anyone else
      // reaching the kernel is refused, as the kernel's handler does.
      if (req.method === 'hew.permissions.request') {
        return reply({ error: { code: -32000, message: 'refused', data: { refusal: 'host_capability_missing' } } })
      }
      if (req.method === 'hew.query.scene') return reply({ result: { tree: [{ kind: 'object' }, { kind: 'object' }], generation } })
      if (req.method === 'hew.tag.create') {
        generation++
        return reply({ result: { results: [{}], label: 'Tag' } })
      }
      // A transaction of one plugin command forwards it, as the kernel does.
      const commands = req.method === 'hew.doc.transact' ? (req.params as { commands?: { method: string; params?: unknown }[] }).commands : undefined
      const method = commands?.length === 1 ? commands[0].method : req.method
      const params = commands?.length === 1 ? (commands[0].params ?? {}) : req.params
      for (const [plugin, decls] of registered) {
        const decl = decls.find((d) => `${plugin}.${d.id}` === method)
        if (decl === undefined) continue
        const token = nextToken++
        pending.set(`${conn}:${token}`, { requestId: req.id, mutating: decl.class === 'mutating', plugin, method })
        return JSON.stringify({
          pending: {
            token,
            forward: {
              token,
              plugin,
              local: decl.id,
              method,
              request_id: req.id,
              params: params ?? {},
              mutating: decl.class === 'mutating',
              label: null,
              timeout_ms: decl.timeout_ms ?? 50_000,
              concurrency: 'parallel',
              caller: { identity: `live:${conn}`, plugin: conns.get(conn)?.plugin ?? null, app: null, forward: stamp },
            },
          },
        })
      }
      return reply({ error: { code: -32601, message: 'method not found' } })
    },
    // The kernel's own mapping (crates/api/src/dispatch.rs, `complete_forward`).
    api_complete_forward: vi.fn((conn: number, token: bigint, replyJson: string) => {
      const p = pending.get(`${conn}:${token}`)
      if (p === undefined) throw new Error('FORWARD: unknown forward token')
      pending.delete(`${conn}:${token}`)
      const r = JSON.parse(replyJson) as { ok?: unknown; refusal?: { name: string; explanation: string; detail?: unknown }; failed?: string }
      const id = p.requestId
      if ('ok' in r) return JSON.stringify({ jsonrpc: '2.0', id, result: r.ok })
      const refused = (refusal: string, explanation: string, detail: unknown = {}) =>
        JSON.stringify({ jsonrpc: '2.0', id, error: { code: -32000, message: 'refused', data: { refusal, failed_index: 0, failed_method: p.method, detail, explanation } } })
      if (r.refusal !== undefined) return refused(`${p.plugin}.${r.refusal.name}`, r.refusal.explanation, r.refusal.detail ?? {})
      switch (r.failed) {
        case 'timeout':
          return JSON.stringify({ jsonrpc: '2.0', id, error: { code: -32003, message: 'the plugin did not answer in time', data: { plugin: p.plugin, method: p.method, reason: 'timeout' } } })
        case 'crashed':
          return JSON.stringify({ jsonrpc: '2.0', id, error: { code: -32003, message: 'the plugin stopped while handling this command', data: { plugin: p.plugin, method: p.method, reason: 'crashed' } } })
        case 'cancelled':
          return refused('plugin_command_cancelled', 'The user cancelled this command while it was running.')
        case 'unavailable':
          return refused('plugin_unavailable', 'This plugin is not running. Start it or reload it, then try again.', { plugin: p.plugin })
        case 'no_handler':
          return JSON.stringify({ jsonrpc: '2.0', id, error: { code: -32601, message: 'the plugin registers no handler for this command', data: { plugin: p.plugin, method: p.method } } })
      }
      throw new Error('FORWARD: malformed forward reply')
    }),
    compound_begin: vi.fn((_conn: number, token: bigint) => {
      const kernel = BigInt(100 + nextKernel++)
      compounds.push({ kernel, forward: Number(token) })
      return kernel
    }),
    // Like the kernel: only the innermost closes; a mismatch changes nothing.
    compound_end: vi.fn((kernel: bigint) => {
      if (compounds[compounds.length - 1]?.kernel !== kernel) throw new Error('CompoundTokenMismatch')
      compounds.pop()
      return '{}'
    }),
    compound_abort: vi.fn((kernel: bigint) => {
      if (compounds[compounds.length - 1]?.kernel !== kernel) throw new Error('CompoundTokenMismatch')
      compounds.pop()
    }),
    compound_depth: vi.fn(() => compounds.length),
  }
  return s
}

type FakeScene = ReturnType<typeof fakeScene>

function manifest(over: Partial<Record<string, unknown>> = {}): PluginManifest {
  return {
    id: ID,
    name: 'Hello',
    version: '0.1.0',
    main: 'dist/background.js',
    permissions: ['model.read'],
    activation: ['on_command'],
    contributes: { commands: [{ id: 'greet', class: 'read_only', summary: 'Greets.' }, { id: 'tag', class: 'mutating', summary: 'Adds a tag.' }] },
    ...over,
  } as PluginManifest
}

function installed(over: Partial<Installed> = {}): Installed {
  return {
    schema: 1,
    id: ID,
    version: '0.1.0',
    source: { kind: 'file', name: 'hello.hewplugin' },
    installed_at_ms: 0,
    grant: ['model.read'],
    optional_granted: ['model.write'],
    enabled: true,
    startup_allowed: false,
    data_version: 0,
    ...over,
  }
}

/** A plugin's background, run by the real runtime over a fake port. */
type Main = (hew: Hew, H: typeof HewRefusal) => void

interface Harness {
  scene: FakeScene
  host: PluginHost
  env: HostEnv
  realms: { channel: RealmChannel; toRealm: HostToRealm[]; send(data: unknown, ports?: MessagePort[]): void; silent: boolean }[]
  list: Listed[]
  changed(): void
  storageChanged(id: string, keys: string[], scope?: 'plugin' | 'document', documentId?: string | null): void
  /** Document storage, by `<plugin id>/<document id>`. */
  docs: Map<string, Map<string, string>>
  /** Sends `plugins-dev-changed`, as the store access passes it on. */
  devChanged(change: DevChange): void
  stored: Map<string, string>
  storage: PluginStorageBackend
  toasts: string[]
  logs: string[]
  setMain(main: Main): void
  grantCalls: [string, string, string][]
  /** The `viewer` flag of each grant built, in order. */
  viewerCalls: boolean[]
  bridge: LiveBridgeDeps
}

const flush = async (n = 20): Promise<void> => {
  for (let i = 0; i < n; i++) await Promise.resolve()
}

/** A scheduler whose later tasks can be held: while held, the host checks
 *  and queues what a realm sends but handles none of it. */
function holdingSched(): { sched: HostScheduler; hold(): void; release(): void } {
  const queued: (() => void)[] = []
  let holding = false
  return {
    sched: {
      now: () => Date.now(),
      later(fn) {
        if (holding) queued.push(fn)
        else queueMicrotask(fn)
      },
    },
    hold: () => (holding = true),
    release() {
      holding = false
      for (const fn of queued.splice(0)) fn()
    },
  }
}

interface HarnessOpts {
  list?: Listed[]
  main?: Main
  /** Each plugin's own background, by id; `main` for the rest. */
  mains?: Record<string, Main>
  timings?: Partial<HostTimings>
  sched?: HostScheduler
  platform?: 'desktop' | 'web' | 'shop'
  /** Stands in for the store's read of a package file. */
  readFile?: (id: string, path: string) => Promise<ArrayBuffer>
  /** Runs before a realm frame answers. */
  beforeOpen?: () => Promise<void>
  /** The shell's developer mode; none (as on the web) when absent. */
  devMode?: () => Promise<boolean>
  /** The person's answers to permission requests. */
  grants?: HostEnv['grants']
  /** Document storage shared with another harness (two windows). */
  docs?: Map<string, Map<string, string>>
  /** Makes the store's document writes fail while it answers true. */
  docSetFails?: () => boolean
  /** The document's persistent id when the host starts. */
  documentId?: string | null
  report?: HostEnv['report']
  console?: HostEnv['console']
}

function harness(opts: HarnessOpts = {}): Harness {
  const scene = fakeScene()
  if (opts.documentId !== undefined && opts.documentId !== null) scene.docId = opts.documentId
  let main: Main = opts.main ?? ((hew) => hew.commands.handle('greet', (p) => ({ greeting: `Hello, ${(p as { name?: string }).name ?? 'Hew'}!` })))
  let onChanged: (() => void) | null = null
  let onStorage: ((id: string, keys: string[], scope: 'plugin' | 'document', documentId: string | null) => void) | null = null
  let onDev: ((change: DevChange) => void) | null = null
  const stored = new Map<string, string>()
  const toasts: string[] = []
  const logs: string[] = []
  const grantCalls: [string, string, string][] = []
  const viewerCalls: boolean[] = []
  const realms: Harness['realms'] = []
  const h = {} as Harness
  const store: PluginStoreAccess = {
    list: async () => h.list,
    // The main script is the plugin's id, so the realm knows whose it runs.
    readFile: (id, path) => (opts.readFile ?? (async (i: string) => new TextEncoder().encode(i).buffer as ArrayBuffer))(id, path),
    onChanged: (fn) => {
      onChanged = fn
      return () => (onChanged = null)
    },
    onStorageChanged: (fn) => {
      onStorage = fn
      return () => (onStorage = null)
    },
    ...(opts.devMode === undefined ? {} : { devMode: opts.devMode }),
    onDevChanged: (fn) => {
      onDev = fn
      return () => (onDev = null)
    },
  }
  const docs = opts.docs ?? new Map<string, Map<string, string>>()
  const docMap = (id: string, doc: string): Map<string, string> => {
    const k = `${id}/${doc}`
    let m = docs.get(k)
    if (m === undefined) {
      m = new Map()
      docs.set(k, m)
    }
    return m
  }
  const storage: PluginStorageBackend = {
    docGet: vi.fn(async (id: string, doc: string, key: string) => docMap(id, doc).get(key) ?? null),
    docSet: vi.fn(async (id: string, doc: string, key: string, value: string) => {
      if (opts.docSetFails?.() === true) throw new PluginError('io', 'the disk is busy')
      if (value.length > 100) throw new PluginError('storage_quota', 'full')
      docMap(id, doc).set(key, value)
    }),
    docDelete: vi.fn(async (id: string, doc: string, key: string) => {
      docMap(id, doc).delete(key)
    }),
    // Code-point order, as both real stores list (a sorted map on the desktop).
    docList: vi.fn(async (id: string, doc: string) => [...docMap(id, doc).keys()].sort(compareCodePoints)),
    get: vi.fn(async (_id: string, key: string) => stored.get(key) ?? null),
    set: vi.fn(async (_id: string, key: string, value: string) => {
      if (value.length > 100) throw new PluginError('storage_quota', 'full')
      stored.set(key, value)
    }),
    delete: vi.fn(async (_id: string, key: string) => {
      stored.delete(key)
    }),
    list: vi.fn(async () => [...stored.keys()].sort()),
  }
  const bridge: LiveBridgeDeps = {
    getScene: () => scene as unknown as Scene,
    getViewportApi: () => null,
    reconcile: vi.fn(),
    reconcileVisibility: vi.fn(),
    reseedViewState: vi.fn(),
  }
  const env: HostEnv = {
    scene: () => scene as unknown as Scene,
    bridge,
    platform: opts.platform ?? 'web',
    store,
    storage,
    grantJson: (id, doc, names, viewer) => {
      grantCalls.push([id, doc, names])
      viewerCalls.push(viewer)
      const all = JSON.parse(names) as string[]
      const permissions = viewer ? all.filter((n) => n === 'model.read' || n === 'view') : all
      return JSON.stringify({ ceiling: 'core', permissions, plugin: id, app: null, document: doc })
    },
    ...(opts.grants === undefined ? {} : { grants: opts.grants }),
    ...(opts.report === undefined ? {} : { report: opts.report }),
    ...(opts.console === undefined ? {} : { console: opts.console }),
    async openRealm(code) {
      await opts.beforeOpen?.()
      const pluginId = new TextDecoder().decode(code)
      const entry = { toRealm: [] as HostToRealm[], silent: false } as Harness['realms'][number]
      let hostPort: PortLike
      // The realm side: the real runtime, its messages cloned to the host.
      const rt = createRuntime({
        post: (m) => {
          if (entry.silent) return
          queueMicrotask(() => hostPort.onmessage?.({ data: structuredClone(m), ports: [] } as unknown as MessageEvent))
        },
        console: { log: () => {}, info: () => {}, warn: () => {}, error: () => {} },
        now: () => Date.now(),
      })
      ;(opts.mains?.[pluginId] ?? main)(rt.hew, HewRefusal)
      hostPort = {
        onmessage: null,
        postMessage: (m: unknown) => {
          entry.toRealm.push(m as HostToRealm)
          const copy = structuredClone(m)
          queueMicrotask(() => {
            if (!entry.silent || (copy as { t: string }).t !== 'ping') rt.receive(copy)
          })
        },
        close: () => {},
      }
      const control: PortLike = { onmessage: null, postMessage: () => {}, close: () => {} }
      entry.channel = { control, port: hostPort, remove: vi.fn() }
      entry.send = (data, ports = []) => hostPort.onmessage?.({ data, ports } as unknown as MessageEvent)
      realms.push(entry)
      return entry.channel
    },
    toast: (m) => toasts.push(m),
    log: (level, m) => logs.push(`${level}: ${m}`),
    locale: () => 'en',
    theme: () => 'dark',
  }
  Object.assign(h, {
    scene,
    env,
    realms,
    list: opts.list ?? [{ kind: 'installed', installed: installed(), manifest: manifest(), previous_version: null }],
    changed: () => onChanged?.(),
    storageChanged: (id: string, keys: string[], scope: 'plugin' | 'document' = 'plugin', documentId: string | null = null) => onStorage?.(id, keys, scope, documentId),
    docs,
    devChanged: (change: DevChange) => onDev?.(change),
    stored,
    storage,
    toasts,
    logs,
    setMain: (m: Main) => (main = m),
    grantCalls,
    viewerCalls,
    bridge,
  })
  h.host = createPluginHost(
    env,
    { pingMs: 2_000, startMs: 10_000, storageDebounceMs: 20, sliceMs: 8, ...opts.timings },
    opts.sched ?? { now: () => Date.now(), later: (fn) => queueMicrotask(fn) },
  )
  return h
}

/** A forward to the hello plugin's `greet`, as the kernel hands one out. */
function forward(over: Partial<ForwardRequestJson> = {}): ForwardRequestJson {
  return {
    token: 1,
    plugin: ID,
    local: 'greet',
    method: `${ID}.greet`,
    request_id: 1,
    params: {},
    mutating: false,
    label: null,
    timeout_ms: 30_000,
    concurrency: 'parallel',
    caller: { identity: 'live:1', plugin: null, app: null, forward: null },
    ...over,
  }
}

/** Calls `method` as a socket client would, through the host's own path. */
type Reply = { result?: unknown; error?: { code?: number; data?: { refusal?: string; reason?: string; plugin?: string } } }
async function call(h: Harness, method: string, params: unknown = {}): Promise<Reply> {
  const conn = h.scene.api_connection_open()
  const reply = await h.host.dispatch(conn, JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }))
  return JSON.parse(reply!) as Reply
}

// ------------------------------------------------------------------ tests

describe('the plugin host', () => {
  let h: Harness
  beforeEach((ctx) => {
    requiresPreview(ctx, 'plugins')
    setBusy(null)
  })
  afterEach(() => {
    h?.host.dispose()
    vi.useRealTimers()
  })

  it('registers the stored commands of enabled plugins only, and starts nothing', async () => {
    h = harness({
      list: [
        { kind: 'installed', installed: installed(), manifest: manifest(), previous_version: null },
        { kind: 'installed', installed: installed({ id: 'com.example.off', enabled: false }), manifest: manifest({ id: 'com.example.off' }), previous_version: null },
        { kind: 'unreadable', id: 'com.example.bad', reason: 'x' },
      ],
    })
    await h.host.sync()
    expect([...h.scene.registered.keys()]).toEqual([ID])
    expect(h.scene.registered.get(ID)!.map((d) => d.id)).toEqual(['greet', 'tag'])
    expect(h.host.states()).toEqual({ [ID]: 'enabled' })
    expect(h.realms).toHaveLength(0)
  })

  it('starts a realm on the first forward, with the grant the kernel builds from the stored record', async () => {
    h = harness()
    await h.host.sync()
    const reply = await call(h, `${ID}.greet`, { name: 'Kurt' })
    expect(reply.result).toEqual({ greeting: 'Hello, Kurt!' })
    expect(h.grantCalls).toEqual([[ID, 'doc_1', '["model.read","model.write"]']])
    expect(h.host.states()).toEqual({ [ID]: 'activated' })
    const hello = h.realms[0].toRealm[0]
    expect(hello).toMatchObject({ t: 'hello', plugin: ID, document: 'doc_1', activation: 'command', realm: 'background' })
    // hello and attach went out on the plugin's own connection.
    const pluginConn = [...h.scene.conns].find(([, c]) => c.plugin === ID)![0]
    const sent = h.scene.api_dispatch.mock.calls.filter(([conn]) => conn === pluginConn).map(([, f]) => (JSON.parse(f) as { method: string }).method)
    expect(sent.slice(0, 2)).toEqual(['hew.meta.hello', 'hew.doc.attach'])
    expect(h.scene.states).toContainEqual([ID, 'loaded', undefined])
    // A second call reuses the realm.
    await call(h, `${ID}.greet`, {})
    expect(h.realms).toHaveLength(1)
  })

  it("routes a handler's bus calls on its own connection and returns the replies", async () => {
    h = harness({
      main: (hew) =>
        hew.commands.handle('greet', async () => {
          const scene = (await hew.api.query.scene({})) as { tree: unknown[] }
          return { greeting: 'Hello!', objects: scene.tree.length }
        }),
    })
    await h.host.sync()
    expect((await call(h, `${ID}.greet`)).result).toEqual({ greeting: 'Hello!', objects: 2 })
  })

  it('serves a mutating forward it owns inside a compound, ended on ok and aborted on a refusal', async () => {
    h = harness({
      main: (hew, H) =>
        hew.commands.handle('tag', async (p) => {
          if ((p as { fail?: boolean }).fail) throw new H('not_today', 'Not today.')
          await hew.api.tag.create({ path: 'x' } as never)
          return { done: true }
        }),
    })
    await h.host.sync()
    const busy: unknown[] = []
    const ok = call(h, `${ID}.tag`)
    await flush()
    busy.push(getBusy()?.plugin)
    const okReply = await ok
    expect(okReply, JSON.stringify(okReply) + JSON.stringify(h.logs)).toMatchObject({ result: { done: true } })
    expect(busy).toEqual([ID])
    expect(h.scene.compound_begin).toHaveBeenCalledTimes(1)
    expect(h.scene.compound_end).toHaveBeenCalledTimes(1)
    expect(h.scene.compound_abort).not.toHaveBeenCalled()
    // The handler's own write ran inside the compound (stamped), not refused.
    expect(h.scene.api_dispatch_in_forward).toHaveBeenCalled()
    expect(getBusy()).toBeNull()

    const refused = await call(h, `${ID}.tag`, { fail: true })
    expect(refused.error?.data?.refusal).toBe(`${ID}.not_today`)
    expect(h.scene.compound_abort).toHaveBeenCalledTimes(1)
    expect(h.bridge.reseedViewState).toHaveBeenCalled()
    expect(getBusy()).toBeNull()
    expect(h.host.states()[ID]).toBe('activated')
  })

  it("queues this window's calls while a plugin command holds the document", async () => {
    let release: () => void = () => {}
    h = harness({
      main: (hew) =>
        hew.commands.handle('tag', () => new Promise((r) => (release = () => r({ ok: 1 })))),
    })
    await h.host.sync()
    const held = call(h, `${ID}.tag`)
    await flush(60)
    expect(h.scene.busy()).toBe(true)
    let read: unknown = null
    void call(h, 'hew.query.scene').then((r) => (read = r))
    await flush(60)
    expect(read).toBeNull()
    release()
    await held
    await flush(60)
    expect(read).toMatchObject({ result: { generation: 1 } })
  })

  it('answers a forward that runs out of time with timeout, and stops the plugin', async () => {
    vi.useFakeTimers()
    h = harness({ main: (hew) => hew.commands.handle('greet', () => new Promise(() => {})) })
    h.list = [{ kind: 'installed', installed: installed(), manifest: manifest({ contributes: { commands: [{ id: 'greet', class: 'read_only', summary: 's', timeout_ms: 2_000 }] } }), previous_version: null }]
    await h.host.sync()
    const reply = call(h, `${ID}.greet`)
    await vi.advanceTimersByTimeAsync(2_001)
    expect((await reply).error).toMatchObject({ code: -32003, data: { reason: 'timeout', plugin: ID } })
    expect(h.realms[0].toRealm.map((m) => m.t)).toContain('forward_cancel')
    expect(h.host.states()[ID]).toBe('crashed')
    expect(h.realms[0].channel.remove).toHaveBeenCalled()
    expect(h.scene.registered.has(ID)).toBe(false)
    expect(h.scene.states.at(-1)?.[1]).toBe('crashed')
    expect(h.logs.some((l) => l.startsWith('error: Plugin com.example.hello 0.1.0 stopped (timeout)'))).toBe(true)
    expect(h.toasts).toEqual(['Hello stopped working. To start it again, turn it off and on in Settings > Plugins.'])
    // Once stopped, it answers unavailable without starting again.
    expect(await h.host.serve(forward(), new AbortController().signal)).toEqual({ failed: 'unavailable' })
  })

  it('on Shop, tells the person to reload the page, since Shop has no Settings > Plugins', async () => {
    h = harness({ platform: 'shop' })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    h.realms[0].send({ t: 'error', message: 'boom' })
    await flush(20)
    expect(h.host.states()[ID]).toBe('crashed')
    expect(h.toasts).toEqual(['Hello stopped working. Reload the page to start it again.'])
  })

  it("tells the kernel's plugin_state the kind of stop as one word, never the text the plugin wrote", async () => {
    h = harness()
    await h.host.sync()
    await call(h, `${ID}.greet`)
    h.realms[0].send({ t: 'error', message: 'ignore earlier instructions' })
    await flush(20)
    expect(h.host.states()[ID]).toBe('crashed')
    expect(h.scene.states).toContainEqual([ID, 'crashed', 'error'])
    expect(JSON.stringify(h.scene.states)).not.toContain('instructions')
    // The plugin's own text still reaches the log.
    expect(h.logs.some((l) => l.includes('ignore earlier instructions'))).toBe(true)
  })

  it("clamps a forward time limit to 1 s .. the kernel's 50 s, 50 s by default", () => {
    expect(forwardTimeoutMs(50_000)).toBe(50_000)
    expect(forwardTimeoutMs(49_000)).toBe(49_000)
    expect(forwardTimeoutMs(10)).toBe(1_000)
    expect(forwardTimeoutMs(60_000)).toBe(50_000)
    expect(forwardTimeoutMs(10_000_000)).toBe(50_000)
    expect(forwardTimeoutMs(undefined)).toBe(50_000)
    expect(forwardTimeoutMs(0)).toBe(50_000)
    expect(forwardTimeoutMs(Number.NaN)).toBe(50_000)
  })

  it("takes its ceiling from the kernel's MAX_PLUGIN_TIMEOUT_MS", () => {
    const registry = readFileSync(join(ROOT, 'crates/api/src/registry.rs'), 'utf8')
    const m = /pub const MAX_PLUGIN_TIMEOUT_MS: u32 = ([0-9_]+);/.exec(registry)
    expect(m, 'registry.rs declares MAX_PLUGIN_TIMEOUT_MS').not.toBeNull()
    expect(Number(m![1].replace(/_/g, ''))).toBe(MAX_FORWARD_TIMEOUT_MS)
  })

  it('stops a plugin that misses two pings', async () => {
    vi.useFakeTimers()
    h = harness()
    await h.host.sync()
    const first = call(h, `${ID}.greet`)
    await vi.advanceTimersByTimeAsync(10)
    await first
    h.realms[0].silent = true
    await vi.advanceTimersByTimeAsync(2_000 * 3 + 10)
    expect(h.host.states()[ID]).toBe('crashed')
    expect(h.logs.some((l) => l.includes('(unresponsive)'))).toBe(true)
  })

  it('keeps a plugin that answers its pings', async () => {
    vi.useFakeTimers()
    h = harness()
    await h.host.sync()
    const first = call(h, `${ID}.greet`)
    await vi.advanceTimersByTimeAsync(10)
    await first
    await vi.advanceTimersByTimeAsync(60_000)
    expect(h.host.states()[ID]).toBe('activated')
  })

  describe('stops a realm that sends', () => {
    const cases: [string, (h: Harness) => void][] = [
      ['an unknown message', (h) => h.realms[0].send({ t: 'eval', code: '1' })],
      ['a message with an extra field', (h) => h.realms[0].send({ t: 'pong', n: 1, x: 1 })],
      ['a non-object', (h) => h.realms[0].send('hello')],
      ['an oversized frame', (h) => h.realms[0].send({ t: 'bus', frame: 'x'.repeat((1 << 20) + 1) })],
      ['a reply to a forward it was never given', (h) => h.realms[0].send({ t: 'forward_reply', id: 999, reply: '{"ok":1}' })],
      ['a bus call naming a forward it was never given', (h) => h.realms[0].send({ t: 'bus', forward: 999, frame: '{"jsonrpc":"2.0","id":1,"method":"hew.query.scene"}' })],
      ['a port', (h) => h.realms[0].send({ t: 'pong', n: 1 }, [new MessageChannel().port1])],
      ['a reused storage id', (h) => {
        h.realms[0].send({ t: 'storage', id: 5, op: 'list' })
        h.realms[0].send({ t: 'storage', id: 5, op: 'list' })
      }],
      ['an error', (h) => h.realms[0].send({ t: 'error', message: 'TypeError: boom' })],
      ['a flood', (h) => {
        for (let i = 0; i < 1_001; i++) h.realms[0].send({ t: 'pong', n: 0 })
      }],
    ]
    for (const [name, act] of cases) {
      it(name, async () => {
        h = harness()
        await h.host.sync()
        await call(h, `${ID}.greet`)
        expect(() => act(h)).not.toThrow()
        await flush()
        expect(h.host.states()[ID]).toBe('crashed')
        expect(h.scene.api_connection_close).toHaveBeenCalled()
        expect(h.scene.registered.has(ID)).toBe(false)
      })
    }
  })

  it("puts at most 20 of a realm's console lines a second in the log, cleaned", async () => {
    h = harness()
    await h.host.sync()
    await call(h, `${ID}.greet`)
    const before = h.logs.length
    for (let i = 0; i < 30; i++) h.realms[0].send({ t: 'console', level: 'log', args: [`line ${i}\u202e`], at: 0 })
    await flush()
    const lines = h.logs.slice(before)
    expect(lines).toHaveLength(20)
    expect(lines[0]).toBe(`info: [${ID}] line 0`)
    expect(h.host.states()[ID]).toBe('activated')
  })

  it('stops a realm whose message could not be read', async () => {
    h = harness()
    await h.host.sync()
    await call(h, `${ID}.greet`)
    h.realms[0].channel.port.onmessageerror?.({} as MessageEvent)
    expect(h.host.states()[ID]).toBe('crashed')
  })

  describe('the inbox', () => {
    /** A scheduler the test turns by hand once `manual` is set. */
    interface Manual {
      clock: number
      manual: boolean
      tasks: (() => void)[]
      sched: HostScheduler
      runOne(): void
      runAll(): Promise<void>
    }
    function manualSched(): Manual {
      const tasks: (() => void)[] = []
      const m: Manual = {
        clock: 0,
        manual: false,
        tasks,
        sched: {
          now: () => m.clock,
          later: (fn: () => void) => (m.manual ? tasks.push(fn) : queueMicrotask(fn)),
        } as HostScheduler,
        runOne() {
          tasks.shift()?.()
        },
        async runAll() {
          for (let i = 0; i < 10_000 && tasks.length > 0; i++) {
            tasks.shift()!()
            await flush(5)
          }
        },
      }
      return m
    }
    const busFrame = (id: string, method = 'hew.query.scene'): string => JSON.stringify({ jsonrpc: '2.0', id, method, params: {} })
    const dispatchedIds = (h: Harness): string[] =>
      h.scene.api_dispatch.mock.calls
        .map(([, f]) => JSON.parse(f) as { id: string; method: string })
        .filter((f) => f.method === 'hew.query.scene' && String(f.id).startsWith('b'))
        .map((f) => String(f.id))

    it('only checks and counts on arrival, then handles a slice at a time and yields', async () => {
      const m = manualSched()
      h = harness({ sched: m.sched })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      m.manual = true
      // Each scene read costs 3 ms of the clock.
      const run = h.scene.run.bind(h.scene)
      h.scene.run = (conn, frame, stamp) => {
        if (frame.includes('hew.query.scene')) m.clock += 3
        return run(conn, frame, stamp)
      }
      for (let i = 0; i < 10; i++) h.realms[0].send({ t: 'bus', frame: busFrame(`b${i}`) })
      expect(dispatchedIds(h)).toEqual([])
      expect(m.tasks).toHaveLength(1)
      m.runOne()
      // 3 + 3 + 3 >= 8: the slice ends after the third, and the rest waits for another task.
      expect(dispatchedIds(h)).toEqual(['b0', 'b1', 'b2'])
      expect(m.tasks).toHaveLength(1)
      await m.runAll()
      expect(dispatchedIds(h)).toEqual(['b0', 'b1', 'b2', 'b3', 'b4', 'b5', 'b6', 'b7', 'b8', 'b9'])
      expect(h.host.states()[ID]).toBe('activated')
    })

    it('keeps a forward reply behind the calls the plugin made while serving it', async () => {
      const m = manualSched()
      h = harness({ sched: m.sched, main: (hew) => hew.commands.handle('tag', () => new Promise(() => {})) })
      await h.host.sync()
      const pending = call(h, `${ID}.tag`)
      await vi.waitFor(() => expect(h.realms[0]?.toRealm.some((x) => x.t === 'forward')).toBe(true))
      const forward = h.realms[0].toRealm.find((x) => x.t === 'forward') as { id: number }
      m.manual = true
      h.realms[0].send({ t: 'bus', forward: forward.id, frame: busFrame('t1', 'hew.tag.create') })
      h.realms[0].send({ t: 'forward_reply', id: forward.id, reply: '{"ok":{"done":1}}' })
      await m.runAll()
      expect((await pending).result).toEqual({ done: 1 })
      const write = h.scene.api_dispatch_in_forward.mock.invocationCallOrder.at(-1)!
      expect(write).toBeLessThan(h.scene.compound_end.mock.invocationCallOrder[0])
    })

    it('stops a realm whose burst passes the per-second limit on arrival, before handling any of it', async () => {
      const m = manualSched()
      h = harness({ sched: m.sched })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      m.manual = true
      for (let i = 0; i < 1_001; i++) h.realms[0].send({ t: 'bus', frame: busFrame(`b${i}`) })
      expect(h.host.states()[ID]).toBe('crashed')
      expect(h.logs.some((l) => l.includes('more than the allowed messages in one second'))).toBe(true)
      await m.runAll()
      expect(dispatchedIds(h)).toEqual([])
    })

    it('stops a realm that keeps more calls waiting than the host holds', async () => {
      const m = manualSched()
      h = harness({ sched: m.sched })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      m.manual = true
      for (let i = 0; i < 600; i++) h.realms[0].send({ t: 'bus', frame: busFrame(`b${i}`) })
      m.clock += 1_100
      for (let i = 600; i < MAX_PENDING_BUS; i++) h.realms[0].send({ t: 'bus', frame: busFrame(`b${i}`) })
      expect(h.host.states()[ID]).toBe('activated')
      h.realms[0].send({ t: 'bus', frame: busFrame('one-too-many') })
      expect(h.host.states()[ID]).toBe('crashed')
      expect(h.logs.some((l) => l.includes('queued too many calls'))).toBe(true)
    })

    it('drops what was queued when the realm stops', async () => {
      const m = manualSched()
      h = harness({ sched: m.sched })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      m.manual = true
      for (let i = 0; i < 5; i++) h.realms[0].send({ t: 'bus', frame: busFrame(`b${i}`) })
      h.realms[0].send({ t: 'error', message: 'boom' })
      expect(h.host.states()[ID]).toBe('crashed')
      await m.runAll()
      expect(dispatchedIds(h)).toEqual([])
    })

    it('logs a few hundred lines of one run of console output, says so once, and shows more after a quiet spell', async () => {
      const m = manualSched()
      h = harness({ sched: m.sched })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      const before = h.logs.length
      // Ten lines a second for a minute: under the per-second cap throughout.
      for (let i = 0; i < 600; i++) {
        m.clock += 100
        h.realms[0].send({ t: 'console', level: 'log', args: [`line ${i}`], at: 0 })
        await flush(2)
      }
      const flood = h.logs.slice(before)
      expect(flood.filter((l) => l.startsWith(`info: [${ID}] line`))).toHaveLength(200)
      expect(flood.filter((l) => l.includes('Further output from this plugin is not shown'))).toHaveLength(1)
      m.clock += 11_000
      h.realms[0].send({ t: 'console', level: 'log', args: ['after'], at: 0 })
      await flush()
      expect(h.logs.at(-1)).toBe(`info: [${ID}] after`)
    })
  })

  it('answers every open forward crashed when the realm stops', async () => {
    h = harness({ main: (hew) => hew.commands.handle('greet', () => new Promise(() => {})) })
    await h.host.sync()
    const reply = call(h, `${ID}.greet`)
    await flush(60)
    h.realms[0].send({ t: 'error', message: 'boom' })
    expect((await reply).error).toMatchObject({ code: -32003, data: { reason: 'crashed', plugin: ID } })
  })

  it('a reply that is not a reply stops the realm', () => {
    expect(readForwardReply('{"ok":1}')).toEqual({ ok: 1 })
    expect(readForwardReply('{"refusal":{"name":"too_big","explanation":"x"}}')).toEqual({ refusal: { name: 'too_big', explanation: 'x', detail: null } })
    expect(readForwardReply('{"failed":"no_handler"}')).toEqual({ failed: 'no_handler' })
    for (const bad of ['', 'x', '1', '[]', '{}', '{"ok":1,"failed":"crashed"}', '{"failed":"timeout"}', '{"refusal":{"name":"Bad","explanation":"x"}}', '{"refusal":{"name":"a","explanation":"x","more":1}}']) {
      expect(readForwardReply(bad), bad).toBeNull()
    }
  })

  it('reads only package paths from a realm', () => {
    expect(packagePath('assets/icon.svg')).toBe('assets/icon.svg')
    for (const bad of ['', '/etc/passwd', '../x', 'a/../b', 'a//b', './a', 'a\\b', 'C:x', 'a\u0000b']) expect(packagePath(bad), bad).toBeNull()
  })

  it('tears a plugin down when it is turned off, and registers it again when it comes back', async () => {
    h = harness({ main: (hew) => hew.commands.handle('greet', () => new Promise(() => {})) })
    await h.host.sync()
    const reply = call(h, `${ID}.greet`)
    await flush(60)
    h.list = [{ kind: 'installed', installed: installed({ enabled: false }), manifest: manifest(), previous_version: null }]
    h.changed()
    await flush(60)
    expect((await reply).error?.data?.refusal).toBe('plugin_unavailable')
    expect(h.realms[0].toRealm.at(-1)).toEqual({ t: 'unload', reason: 'disable' })
    expect(h.realms[0].channel.remove).toHaveBeenCalled()
    expect(h.scene.registered.has(ID)).toBe(false)
    expect(h.host.states()).toEqual({})
    expect(h.scene.states.at(-1)).toEqual([ID, 'unloaded', 'disable'])
    h.list = [{ kind: 'installed', installed: installed(), manifest: manifest(), previous_version: null }]
    await h.host.sync()
    expect(h.host.states()).toEqual({ [ID]: 'enabled' })
    expect(h.scene.registered.has(ID)).toBe(true)
  })

  it('restarts a plugin whose version or grant changed, and leaves a repeat notice alone', async () => {
    h = harness()
    await h.host.sync()
    await call(h, `${ID}.greet`)
    h.changed()
    await flush(60)
    expect(h.host.states()[ID]).toBe('activated')
    h.list = [{ kind: 'installed', installed: installed({ optional_granted: [] }), manifest: manifest(), previous_version: null }]
    await h.host.sync()
    expect(h.host.states()[ID]).toBe('enabled')
    expect(h.realms[0].toRealm.at(-1)).toEqual({ t: 'unload', reason: 'reload' })
    await call(h, `${ID}.greet`)
    expect(h.grantCalls.at(-1)).toEqual([ID, 'doc_1', '["model.read"]'])
  })

  it('starts a plugin with Hew only when it declares on_startup and the user allowed it', async () => {
    const startup = manifest({ activation: ['on_startup', 'on_command'] })
    h = harness({ list: [{ kind: 'installed', installed: installed(), manifest: startup, previous_version: null }] })
    await h.host.sync()
    await flush(60)
    expect(h.realms).toHaveLength(0)
    h.host.dispose()
    h = harness({ list: [{ kind: 'installed', installed: installed({ startup_allowed: true }), manifest: startup, previous_version: null }] })
    await h.host.sync()
    await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
    expect((h.realms[0].toRealm[0] as { activation: string }).activation).toBe('startup')
  })

  it("keeps a plugin's storage, debouncing writes and refusing over quota by name", async () => {
    let hew!: Hew
    h = harness({
      main: (api, H) => {
        hew = api
        api.commands.handle('greet', () => 'x')
        void H
      },
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    const a = hew.storage.set('a', 1)
    const b = hew.storage.set('a', 2)
    // Read-your-writes before the write lands.
    await expect(hew.storage.get('a')).resolves.toBe(2)
    await Promise.all([a, b])
    expect(h.storage.set).toHaveBeenCalledTimes(1)
    expect(h.stored.get('a')).toBe('2')
    await expect(hew.storage.list()).resolves.toEqual(['a'])
    await expect(hew.storage.set('big', 'x'.repeat(200))).rejects.toMatchObject({ name: 'storage_quota' })
    await hew.storage.delete('a')
    expect(h.stored.has('a')).toBe(false)
    const changed = vi.fn()
    hew.storage.onChange(changed)
    h.storageChanged(ID, ['a'])
    await flush()
    expect(changed).toHaveBeenCalledWith(['a'])
  })

  it("delivers events owed to a realm's connection, and nobody else's", async () => {
    const seen: unknown[] = []
    h = harness({
      main: (hew) => {
        hew.commands.handle('greet', () => 'x')
        hew.events.on('document_changed', (p) => seen.push(p))
      },
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    const pluginConn = [...h.scene.conns].find(([, c]) => c.plugin === ID)![0]
    h.scene.events.push({ conn: pluginConn, frame: '{"jsonrpc":"2.0","method":"hew.event.document_changed","params":{"generation":9}}' })
    h.scene.events.push({ conn: 999, frame: '{"jsonrpc":"2.0","method":"hew.event.document_changed","params":{"generation":10}}' })
    pumpEvents(h.scene as unknown as Scene)
    await flush()
    expect(seen).toEqual([{ generation: 9 }])
  })

  it('stops every realm when the window closes', async () => {
    h = harness()
    await h.host.sync()
    await call(h, `${ID}.greet`)
    h.host.dispose()
    expect(h.realms[0].toRealm.at(-1)).toEqual({ t: 'unload', reason: 'window_closed' })
    expect(h.realms[0].channel.remove).toHaveBeenCalled()
    expect(h.scene.registered.size).toBe(0)
  })

  it('answers a forward to an unknown plugin unavailable', async () => {
    h = harness()
    await h.host.sync()
    const reply = await h.host.serve(forward({ plugin: 'com.example.none', method: 'com.example.none.x' }), new AbortController().signal)
    expect(reply).toEqual({ failed: 'unavailable' })
  })

  it('tells the plugin when the caller gives up, and ignores its late answer', async () => {
    let finish: () => void = () => {}
    h = harness({ main: (hew) => hew.commands.handle('greet', () => new Promise((r) => (finish = () => r('late')))) })
    await h.host.sync()
    const controller = new AbortController()
    const reply = h.host.serve(forward(), controller.signal)
    await flush(60)
    controller.abort()
    await expect(reply).resolves.toEqual({ failed: 'cancelled' })
    expect(h.realms[0].toRealm.at(-1)).toMatchObject({ t: 'forward_cancel' })
    finish()
    await flush(60)
    expect(h.host.states()[ID]).toBe('activated')
  })
})

// ------------------------------------------------------- lifecycle edges

const OTHER = 'com.example.other'

/** The hello plugin's manifest with `slow` (mutating) beside its commands. */
const withSlow = (): PluginManifest =>
  manifest({
    contributes: {
      commands: [
        { id: 'greet', class: 'read_only', summary: 'Greets.' },
        { id: 'tag', class: 'mutating', summary: 'Adds a tag.' },
        { id: 'slow', class: 'mutating', summary: 'Takes its time.' },
      ],
    },
  })

/** A promise and the function that settles it. */
function gate<T = void>(): { promise: Promise<T>; open(value: T): void } {
  let open!: (value: T) => void
  const promise = new Promise<T>((r) => (open = r))
  return { promise, open }
}

describe('the plugin host, at the edges of a command', () => {
  let h: Harness
  beforeEach((ctx) => {
    requiresPreview(ctx, 'plugins')
    setBusy(null)
  })
  afterEach(() => {
    h?.host.dispose()
    vi.useRealTimers()
    setPumpDocumentId(null)
  })

  /** Every message the host sent the plugin's realm, of type `t`. */
  const sentOf = (h: Harness, t: HostToRealm['t'], realm = 0): HostToRealm[] => h.realms[realm].toRealm.filter((m) => m.t === t)

  describe('a nested command still open when its outer command ends', () => {
    /** `tag` fires `slow` without waiting for it and answers at once; `slow` never answers. */
    const firesAndForgets = (seen: { innerError?: unknown; slowCancelled?: boolean }): Main => (hew) => {
      hew.commands.handle('tag', async () => {
        await hew.api.tag.create({ path: 'outer' } as never)
        hew.transact([{ method: `${ID}.slow`, params: {} }]).catch((e: unknown) => (seen.innerError = e))
        return { fired: true }
      })
      hew.commands.handle('slow', (_p, ctx) => {
        ctx.signal.addEventListener('abort', () => (seen.slowCancelled = true))
        return new Promise(() => {})
      })
    }

    it('is ended (aborted, its plugin told) before the outer compound commits, and the document is free', async () => {
      const seen: { innerError?: unknown; slowCancelled?: boolean } = {}
      h = harness({ main: firesAndForgets(seen), list: [{ kind: 'installed', installed: installed(), manifest: withSlow(), previous_version: null }] })
      await h.host.sync()
      const reply = await call(h, `${ID}.tag`)
      expect(reply.result).toEqual({ fired: true })
      await flush(60)
      expect(h.scene.compounds, 'no compound left open').toEqual([])
      expect(h.scene.busy()).toBe(false)
      expect(getBusy()).toBeNull()
      // The inner one was aborted before the outer one ended.
      expect(h.scene.compound_abort).toHaveBeenCalledTimes(1)
      expect(h.scene.compound_end).toHaveBeenCalledTimes(1)
      expect(h.scene.compound_abort.mock.invocationCallOrder[0]).toBeLessThan(h.scene.compound_end.mock.invocationCallOrder[0])
      expect(h.scene.compound_end.mock.calls[0][0]).toBe(100n)
      expect(seen.slowCancelled).toBe(true)
      expect((seen.innerError as { refusal?: { refusal?: string } }).refusal?.refusal).toBe('plugin_command_cancelled')
      expect(h.host.states()[ID]).toBe('activated')
      expect(h.logs.filter((l) => l.startsWith('error:'))).toEqual([])
      // Hew takes the next call at once.
      expect(await call(h, 'hew.query.scene')).toMatchObject({ result: { generation: 2 } })
    })

    it('is ended when the outer command is cancelled, runs out of time, or its plugin stops', async () => {
      for (const how of ['cancel', 'timeout', 'crash'] as const) {
        const seen: { innerError?: unknown; slowCancelled?: boolean } = {}
        let finishOuter: () => void = () => {}
        h = harness({
          list: [
            {
              kind: 'installed',
              installed: installed(),
              manifest: manifest({
                contributes: {
                  commands: [
                    { id: 'tag', class: 'mutating', summary: 'Adds a tag.', timeout_ms: 1_000 },
                    { id: 'slow', class: 'mutating', summary: 'Takes its time.' },
                  ],
                },
              }),
              previous_version: null,
            },
          ],
          main: (hew) => {
            hew.commands.handle('tag', () => {
              hew.transact([{ method: `${ID}.slow`, params: {} }]).catch((e: unknown) => (seen.innerError = e))
              return new Promise((r) => (finishOuter = () => r({ late: true })))
            })
            hew.commands.handle('slow', (_p, ctx) => {
              ctx.signal.addEventListener('abort', () => (seen.slowCancelled = true))
              return new Promise(() => {})
            })
          },
        })
        await h.host.sync()
        const reply = call(h, `${ID}.tag`)
        await vi.waitFor(() => expect(h.scene.compounds).toHaveLength(2))
        if (how === 'cancel') getBusy()!.cancel()
        else if (how === 'crash') h.realms[0].send({ t: 'error', message: 'boom' })
        const r = await reply
        if (how === 'cancel') expect(r.error, how).toMatchObject({ code: -32000, data: { refusal: 'plugin_command_cancelled' } })
        else expect(r.error, how).toMatchObject({ code: -32003, data: { reason: how === 'timeout' ? 'timeout' : 'crashed', plugin: ID } })
        await flush(60)
        expect(h.scene.compounds, how).toEqual([])
        expect(getBusy(), how).toBeNull()
        expect(h.scene.compound_end, how).not.toHaveBeenCalled()
        expect(h.scene.compound_abort, how).toHaveBeenCalledTimes(2)
        finishOuter()
        h.host.dispose()
        setBusy(null)
      }
    }, 10_000)

    it('is ended when it is served by another plugin, and the other plugin keeps running', async () => {
      const seen: { innerError?: unknown; slowCancelled?: boolean } = {}
      h = harness({
        list: [
          { kind: 'installed', installed: installed(), manifest: manifest(), previous_version: null },
          {
            kind: 'installed',
            installed: installed({ id: OTHER }),
            manifest: manifest({ id: OTHER, contributes: { commands: [{ id: 'slow', class: 'mutating', summary: 'Takes its time.' }] } }),
            previous_version: null,
          },
        ],
        mains: {
          [ID]: (hew) =>
            hew.commands.handle('tag', () => {
              hew.transact([{ method: `${OTHER}.slow`, params: {} }]).catch((e: unknown) => (seen.innerError = e))
              // Answered once the other plugin's command is under way.
              return new Promise((r) => setTimeout(() => r({ fired: true }), 20))
            }),
          [OTHER]: (hew) =>
            hew.commands.handle('slow', (_p, ctx) => {
              ctx.signal.addEventListener('abort', () => (seen.slowCancelled = true))
              return new Promise(() => {})
            }),
        },
      })
      await h.host.sync()
      expect((await call(h, `${ID}.tag`)).result).toEqual({ fired: true })
      await flush(60)
      expect(h.scene.compounds).toEqual([])
      expect(getBusy()).toBeNull()
      expect(seen.slowCancelled).toBe(true)
      expect(h.host.states()).toEqual({ [ID]: 'activated', [OTHER]: 'activated' })
    })

    it("is ended when the outer command's plugin is turned off", async () => {
      const seen: { innerError?: unknown; slowCancelled?: boolean } = {}
      h = harness({
        list: [
          { kind: 'installed', installed: installed(), manifest: manifest(), previous_version: null },
          {
            kind: 'installed',
            installed: installed({ id: OTHER }),
            manifest: manifest({ id: OTHER, contributes: { commands: [{ id: 'slow', class: 'mutating', summary: 'Takes its time.' }] } }),
            previous_version: null,
          },
        ],
        mains: {
          [ID]: (hew) =>
            hew.commands.handle('tag', () => {
              hew.transact([{ method: `${OTHER}.slow`, params: {} }]).catch((e: unknown) => (seen.innerError = e))
              return new Promise(() => {})
            }),
          [OTHER]: (hew) =>
            hew.commands.handle('slow', (_p, ctx) => {
              ctx.signal.addEventListener('abort', () => (seen.slowCancelled = true))
              return new Promise(() => {})
            }),
        },
      })
      await h.host.sync()
      const reply = call(h, `${ID}.tag`)
      await vi.waitFor(() => expect(h.scene.compounds).toHaveLength(2))
      h.list = h.list.filter((l) => l.kind !== 'installed' || l.installed.id !== ID)
      await h.host.sync()
      expect((await reply).error?.data?.refusal).toBe('plugin_unavailable')
      await flush(60)
      expect(h.scene.compounds).toEqual([])
      expect(getBusy()).toBeNull()
      expect(seen.slowCancelled).toBe(true)
      expect(h.host.states()).toEqual({ [OTHER]: 'activated' })
    })
  })

  describe('a call that names a forward that has ended', () => {
    it('after Cancel is refused forward_ended and never runs as a write of its own', async () => {
      const results: unknown[] = []
      const go = gate()
      h = harness({
        main: (hew) =>
          hew.commands.handle('tag', async () => {
            // Ignores its signal, and writes after the caller gave up.
            await go.promise
            try {
              results.push(await hew.api.tag.create({ path: 'late' } as never))
            } catch (e) {
              results.push((e as { refusal?: { refusal?: string } }).refusal?.refusal)
            }
            return { done: true }
          }),
      })
      await h.host.sync()
      const reply = call(h, `${ID}.tag`)
      await vi.waitFor(() => expect(h.realms[0]?.toRealm.some((m) => m.t === 'forward')).toBe(true))
      getBusy()!.cancel()
      expect((await reply).error?.data?.refusal).toBe('plugin_command_cancelled')
      const tagsBefore = h.scene.api_dispatch.mock.calls.filter(([, f]) => f.includes('hew.tag.create')).length
      go.open()
      await vi.waitFor(() => expect(results).toHaveLength(1))
      expect(results).toEqual(['forward_ended'])
      expect(h.scene.api_dispatch.mock.calls.filter(([, f]) => f.includes('hew.tag.create'))).toHaveLength(tagsBefore)
      expect(h.scene.api_dispatch_in_forward.mock.calls.filter(([, , f]) => f.includes('hew.tag.create'))).toHaveLength(0)
      await flush(60)
      expect(h.host.states()[ID]).toBe('activated')
    })

    it('after it was answered is refused forward_ended, and the plugin keeps running', async () => {
      h = harness()
      await h.host.sync()
      await call(h, `${ID}.greet`)
      const answered = (sentOf(h, 'forward').at(-1) as { id: number }).id
      h.realms[0].send({ t: 'bus', forward: answered, frame: '{"jsonrpc":"2.0","id":"late","method":"hew.tag.create","params":{}}' })
      await flush(60)
      const back = sentOf(h, 'bus').map((m) => JSON.parse((m as { frame: string }).frame) as { id?: unknown; error?: { data?: { refusal?: string } } })
      expect(back.find((f) => f.id === 'late')?.error?.data?.refusal).toBe('forward_ended')
      expect(h.scene.api_dispatch.mock.calls.some(([, f]) => f.includes('"late"'))).toBe(false)
      expect(h.host.states()[ID]).toBe('activated')
      // One it was never given still stops it.
      h.realms[0].send({ t: 'bus', forward: answered + 1, frame: '{"jsonrpc":"2.0","id":"x","method":"hew.query.scene"}' })
      await flush(60)
      expect(h.host.states()[ID]).toBe('crashed')
    })

    it('a nested read made for a compound that has closed is refused, not run outside it', async () => {
      const gotRead = gate()
      let readReply: unknown = null
      h = harness({
        list: [
          { kind: 'installed', installed: installed(), manifest: manifest(), previous_version: null },
          {
            kind: 'installed',
            installed: installed({ id: OTHER }),
            manifest: manifest({ id: OTHER, contributes: { commands: [{ id: 'look', class: 'read_only', summary: 'Looks.' }] } }),
            previous_version: null,
          },
        ],
        mains: {
          [ID]: (hew) =>
            hew.commands.handle('tag', () => {
              hew.transact([{ method: `${OTHER}.look`, params: {} }]).catch(() => {})
              return new Promise((r) => setTimeout(() => r({ fired: true }), 20))
            }),
          [OTHER]: (hew) =>
            hew.commands.handle('look', async () => {
              await gotRead.promise
              try {
                readReply = await hew.api.tag.create({ path: 'from-a-read' } as never)
              } catch (e) {
                readReply = (e as { refusal?: { refusal?: string } }).refusal?.refusal
              }
              return {}
            }),
        },
      })
      await h.host.sync()
      expect((await call(h, `${ID}.tag`)).result).toEqual({ fired: true })
      expect(h.scene.compounds).toEqual([])
      gotRead.open()
      await vi.waitFor(() => expect(readReply).not.toBeNull())
      expect(readReply).toBe('forward_ended')
      expect(h.scene.api_dispatch_in_forward.mock.calls.some(([, , f]) => f.includes('from-a-read'))).toBe(false)
    })
  })

  it('keeps late progress and answers for hundreds of given-up forwards from stopping a healthy plugin', async () => {
    h = harness({ main: (hew) => hew.commands.handle('greet', () => new Promise(() => {})) })
    await h.host.sync()
    const controllers: AbortController[] = []
    const replies: Promise<unknown>[] = []
    for (let i = 0; i < 300; i++) {
      const c = new AbortController()
      controllers.push(c)
      replies.push(h.host.serve(forward(), c.signal))
    }
    await vi.waitFor(() => expect(sentOf(h, 'forward')).toHaveLength(300))
    for (const c of controllers) c.abort()
    await Promise.all(replies)
    const first = (sentOf(h, 'forward')[0] as { id: number }).id
    h.realms[0].send({ t: 'forward_progress', id: first, fraction: 0.5 })
    h.realms[0].send({ t: 'forward_reply', id: first, reply: '{"ok":1}' })
    await flush(60)
    expect(h.host.states()[ID]).toBe('activated')
  })

  describe('a reply to a forward that is no longer open', () => {
    it('a second answer while the first is remembered stops the plugin', async () => {
      h = harness()
      await h.host.sync()
      await call(h, `${ID}.greet`)
      const id = (sentOf(h, 'forward')[0] as { id: number }).id
      h.realms[0].send({ t: 'forward_reply', id, reply: '{"ok":1}' })
      await flush(60)
      expect(h.host.states()[ID]).toBe('crashed')
      expect(h.logs.some((l) => l.includes('answered a forward twice'))).toBe(true)
    })

    it('a reply after the memory of its forward expired is dropped quietly', async () => {
      vi.useFakeTimers()
      h = harness({ timings: { pingMs: 1_000_000 } })
      await h.host.sync()
      const first = call(h, `${ID}.greet`)
      await vi.advanceTimersByTimeAsync(10)
      await first
      const id = (sentOf(h, 'forward')[0] as { id: number }).id
      await vi.advanceTimersByTimeAsync(5 * 60_000 + 1)
      h.realms[0].send({ t: 'forward_reply', id, reply: '{"ok":1}' })
      await vi.advanceTimersByTimeAsync(10)
      expect(h.host.states()[ID]).toBe('activated')
    })

    it('a reply after its given-up entry was evicted is dropped quietly', async () => {
      h = harness({ main: (hew) => hew.commands.handle('greet', () => new Promise(() => {})) })
      await h.host.sync()
      const controllers: AbortController[] = []
      const replies: Promise<unknown>[] = []
      for (let i = 0; i < 4_100; i++) {
        const c = new AbortController()
        controllers.push(c)
        replies.push(h.host.serve(forward(), c.signal))
      }
      await vi.waitFor(() => expect(sentOf(h, 'forward')).toHaveLength(4_100), { timeout: 10_000 })
      for (const c of controllers) c.abort()
      await Promise.all(replies)
      const oldest = (sentOf(h, 'forward')[0] as { id: number }).id
      h.realms[0].send({ t: 'forward_reply', id: oldest, reply: '{"ok":1}' })
      await flush(60)
      expect(h.host.states()[ID]).toBe('activated')
    }, 20_000)
  })

  describe('starting a plugin', () => {
    const startupList = (): Listed[] => [
      { kind: 'installed', installed: installed({ startup_allowed: true }), manifest: manifest({ activation: ['on_startup', 'on_command'] }), previous_version: null },
    ]
    const tagMain: Main = (hew) => {
      hew.commands.handle('greet', () => 'hi')
      hew.commands.handle('tag', async () => {
        await hew.api.tag.create({ path: 'x' } as never)
        return { done: true }
      })
    }

    it('lets a mutating command that arrives during a startup say hello inside its compound', async () => {
      const read = gate<ArrayBuffer>()
      h = harness({ list: startupList(), main: tagMain, readFile: () => read.promise })
      await h.host.sync()
      expect(h.host.states()[ID]).toBe('activating')
      const reply = call(h, `${ID}.tag`)
      await vi.waitFor(() => expect(h.scene.compounds).toHaveLength(1))
      read.open(new TextEncoder().encode(ID).buffer as ArrayBuffer)
      await expect(reply).resolves.toMatchObject({ result: { done: true } })
      expect(h.host.states()[ID]).toBe('activated')
      expect(h.scene.compounds).toEqual([])
      // Hello went inside the compound, stamped with its forward.
      expect(h.scene.api_dispatch_in_forward.mock.calls.some(([, , f]) => f.includes('hew.meta.hello'))).toBe(true)
      expect((sentOf(h, 'hello')[0] as { activation: string }).activation).toBe('startup')
    })

    it('serves every forward that arrives while it starts, from one start', async () => {
      const read = gate<ArrayBuffer>()
      h = harness({ main: tagMain, readFile: () => read.promise })
      await h.host.sync()
      const a = call(h, `${ID}.greet`)
      const b = call(h, `${ID}.tag`)
      await vi.waitFor(() => expect(h.scene.compounds).toHaveLength(1))
      read.open(new TextEncoder().encode(ID).buffer as ArrayBuffer)
      await expect(a).resolves.toMatchObject({ result: 'hi' })
      await expect(b).resolves.toMatchObject({ result: { done: true } })
      expect(h.realms).toHaveLength(1)
      expect(h.scene.compounds).toEqual([])
    })

    it('answers unavailable and starts nothing when it is turned off while starting', async () => {
      const read = gate<ArrayBuffer>()
      h = harness({ main: tagMain, readFile: () => read.promise })
      await h.host.sync()
      const reply = call(h, `${ID}.tag`)
      await vi.waitFor(() => expect(h.scene.compounds).toHaveLength(1))
      h.list = []
      await h.host.sync()
      read.open(new TextEncoder().encode(ID).buffer as ArrayBuffer)
      expect((await reply).error?.data?.refusal).toBe('plugin_unavailable')
      await flush(60)
      expect(h.realms).toHaveLength(0)
      expect(h.scene.compounds).toEqual([])
      expect(getBusy()).toBeNull()
      expect(h.host.states()).toEqual({})
    })

    it('carries on when the list is read again while it starts', async () => {
      const read = gate<ArrayBuffer>()
      h = harness({ main: tagMain, readFile: () => read.promise })
      await h.host.sync()
      const reply = call(h, `${ID}.tag`)
      await vi.waitFor(() => expect(h.scene.compounds).toHaveLength(1))
      await h.host.sync()
      read.open(new TextEncoder().encode(ID).buffer as ArrayBuffer)
      await expect(reply).resolves.toMatchObject({ result: { done: true } })
      expect(h.host.states()[ID]).toBe('activated')
    })

    it('does not say hello with the stamp of a command that was cancelled while it started', async () => {
      const read = gate<ArrayBuffer>()
      h = harness({ main: tagMain, readFile: () => read.promise })
      await h.host.sync()
      const reply = call(h, `${ID}.tag`)
      await vi.waitFor(() => expect(getBusy()).not.toBeNull())
      getBusy()!.cancel()
      expect((await reply).error?.data?.refusal).toBe('plugin_command_cancelled')
      // Something else holds the document now.
      h.scene.compounds.push({ kernel: 999n, forward: 999 })
      setBusy({ plugin: 'someone.else', label: null, startedAt: 0, cancel: () => {} })
      read.open(new TextEncoder().encode(ID).buffer as ArrayBuffer)
      await flush(60)
      expect(h.host.states()[ID]).toBe('activating')
      h.scene.compounds.pop()
      setBusy(null)
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
      expect(h.logs.filter((l) => l.startsWith('error:'))).toEqual([])
      // The cancelled command's forward never stamped anything: hello and
      // attach went out plain, on the plugin's own connection, once the
      // document was free.
      const cancelled = h.scene.compound_begin.mock.calls[0][1]
      expect(h.scene.api_dispatch_in_forward.mock.calls.filter(([, token]) => token === cancelled)).toEqual([])
      const pluginConn = [...h.scene.conns].find(([, c]) => c.plugin === ID)![0]
      const plain = h.scene.api_dispatch.mock.calls.filter(([c]) => c === pluginConn).map(([, f]) => (JSON.parse(f) as { method: string }).method)
      expect(plain.slice(0, 2)).toEqual(['hew.meta.hello', 'hew.doc.attach'])
    })

    it("gives a slow start its own limits: a forward's clock starts when the plugin has it", async () => {
      vi.useFakeTimers()
      h = harness({
        list: [{ kind: 'installed', installed: installed(), manifest: manifest({ contributes: { commands: [{ id: 'greet', class: 'read_only', summary: 's', timeout_ms: 1_000 }] } }), previous_version: null }],
        beforeOpen: () => new Promise((r) => setTimeout(r, 2_500)),
      })
      await h.host.sync()
      const reply = call(h, `${ID}.greet`)
      await vi.advanceTimersByTimeAsync(3_000)
      expect((await reply).result).toEqual({ greeting: 'Hello, Hew!' })
      expect(h.host.states()[ID]).toBe('activated')
    })

    it('on the desktop waits for the document id, and without one does not start (and is not stopped)', async () => {
      vi.useFakeTimers()
      h = harness({ platform: 'desktop' })
      await h.host.sync()
      const reply = call(h, `${ID}.greet`)
      await vi.advanceTimersByTimeAsync(10_100)
      expect((await reply).error?.data?.refusal).toBe('plugin_unavailable')
      expect(h.grantCalls).toEqual([])
      expect(h.realms).toHaveLength(0)
      expect(h.host.states()[ID]).toBe('enabled')
      expect(h.logs.some((l) => l.startsWith('error:'))).toBe(false)
      expect(h.logs.some((l) => l.startsWith('warn:') && l.includes('did not start'))).toBe(true)
      expect(h.toasts).toEqual([])
      // Once the id comes, the next call starts it under that id.
      const later = call(h, `${ID}.greet`)
      await vi.advanceTimersByTimeAsync(500)
      setPumpDocumentId('doc_7')
      await vi.advanceTimersByTimeAsync(200)
      expect((await later).result).toEqual({ greeting: 'Hello, Hew!' })
      expect(h.grantCalls).toEqual([[ID, 'doc_7', '["model.read","model.write"]']])
    })
  })

  it('answers a window-declined hew.view.enter with the named refusal, after the window acted', async () => {
    h = harness()
    const order: string[] = []
    h.bridge.enterNode = vi.fn(() => {
      order.push('enter')
      return { refused: { name: 'nested_component_in_context' } }
    })
    h.scene.take_pending_view_directives.mockReturnValueOnce(
      JSON.stringify([{ kind: 'enter_context', path: [], leaf: { kind: 'instance', id: '9' } }]),
    )
    const reply = await call(h, 'hew.view.enter', { id: '9' })
    expect(order).toEqual(['enter'])
    expect(reply.result).toBeUndefined()
    expect(reply.error).toMatchObject({
      code: -32000,
      data: { refusal: 'nested_component_in_context', failed_method: 'hew.view.enter' },
    })
  })

  it('answers a refusal, not the empty result, when applying a hew.view.enter throws', async () => {
    h = harness()
    h.bridge.enterNode = vi.fn(() => {
      throw new Error('boom')
    })
    h.scene.take_pending_view_directives.mockReturnValueOnce(
      JSON.stringify([{ kind: 'enter_context', path: [], leaf: { kind: 'instance', id: '9' } }]),
    )
    const reply = await call(h, 'hew.view.enter', { id: '9' })
    expect(reply.result).toBeUndefined()
    expect(reply.error).toMatchObject({ code: -32000, data: { refusal: 'enter_refused' } })
  })

  const enterTransaction = { commands: [{ method: 'hew.view.enter', params: { id: '9' } }] }

  it('answers a window-declined hew.view.enter sent as a one-command transaction with the named refusal', async () => {
    h = harness()
    h.bridge.enterNode = vi.fn(() => ({ refused: { name: 'nested_component_in_context' } }))
    h.scene.take_pending_view_directives.mockReturnValueOnce(
      JSON.stringify([{ kind: 'enter_context', path: [], leaf: { kind: 'instance', id: '9' } }]),
    )
    const reply = await call(h, 'hew.doc.transact', enterTransaction)
    expect(reply.result).toBeUndefined()
    expect(reply.error).toMatchObject({
      code: -32000,
      data: { refusal: 'nested_component_in_context', failed_method: 'hew.view.enter' },
    })
  })

  it('answers a refusal when applying a one-command transaction around hew.view.enter throws', async () => {
    h = harness()
    h.bridge.enterNode = vi.fn(() => ({ ok: true }) as const)
    h.scene.api_method_mutates.mockImplementation(() => {
      throw new Error('dead scene')
    })
    h.scene.take_pending_view_directives.mockReturnValueOnce(
      JSON.stringify([{ kind: 'enter_context', path: [], leaf: { kind: 'instance', id: '9' } }]),
    )
    const reply = await call(h, 'hew.doc.transact', enterTransaction)
    expect(reply.result).toBeUndefined()
    expect(reply.error).toMatchObject({ code: -32000, data: { refusal: 'enter_refused' } })
  })

  it('answers the original reply for a hew.view.enter the window took', async () => {
    h = harness()
    h.bridge.enterNode = vi.fn(() => ({ ok: true }) as const)
    h.scene.take_pending_view_directives.mockReturnValueOnce(
      JSON.stringify([{ kind: 'enter_context', path: [], leaf: { kind: 'instance', id: '9' } }]),
    )
    const reply = await call(h, 'hew.view.enter', { id: '9' })
    expect(reply).toEqual({ jsonrpc: '2.0', id: 1, result: {} })
    expect(h.bridge.enterNode).toHaveBeenCalledTimes(1)
  })

  it("pushes the viewport's camera before a save, as the live bridge does, but not for a save that is refused or while the document is held", async () => {
    const saves = (h: Harness): { id?: unknown; result?: unknown; error?: { code?: number } }[] =>
      h.realms[0].toRealm
        .filter((m) => m.t === 'bus')
        .map((m) => JSON.parse((m as { frame: string }).frame) as { id?: unknown; result?: unknown; error?: { code?: number } })
        .filter((f) => typeof f.id === 'string' && f.id.startsWith('save'))
    h = harness({
      main: (hew) => {
        hew.commands.handle('greet', () => 'x')
        // Saves from inside its own command, while that command holds the document.
        hew.commands.handle('tag', async () => {
          await hew.api.doc.save({} as never).catch(() => {})
          return { done: true }
        })
      },
    })
    const pushCameraState = vi.fn()
    h.bridge.pushCameraState = pushCameraState
    await h.host.sync()
    await call(h, 'hew.doc.save', {})
    expect(pushCameraState).toHaveBeenCalledTimes(1)
    await call(h, 'hew.query.scene', {})
    expect(pushCameraState).toHaveBeenCalledTimes(1)
    // A JSON-escaped method name is still a save.
    const conn = h.scene.api_connection_open()
    await h.host.dispatch(conn, '{"jsonrpc":"2.0","id":7,"method":"hew.doc.sav\\u0065","params":{}}')
    expect(pushCameraState).toHaveBeenCalledTimes(2)
    // A plugin whose grant cannot save leaves the camera alone: its save is
    // refused, and a refusal changes nothing.
    await call(h, `${ID}.greet`)
    h.realms[0].send({ t: 'bus', frame: '{"jsonrpc":"2.0","id":"save-1","method":"hew.doc.save","params":{}}' })
    await flush(60)
    expect(saves(h)).toEqual([expect.objectContaining({ id: 'save-1', error: expect.objectContaining({ code: -32001 }) })])
    expect(pushCameraState).toHaveBeenCalledTimes(2)
    h.host.dispose()

    // One that may save pushes the camera, except while a command holds the
    // document: the Scene refuses to set it then, yet the save itself runs.
    h = harness({
      list: [{ kind: 'installed', installed: installed({ optional_granted: ['model.write', 'documents.save'] }), manifest: manifest(), previous_version: null }],
      main: (hew) => {
        hew.commands.handle('greet', () => 'x')
        hew.commands.handle('tag', async () => {
          await hew.api.doc.save({} as never)
          return { done: true }
        })
      },
    })
    const push2 = vi.fn()
    h.bridge.pushCameraState = push2
    await h.host.sync()
    await call(h, `${ID}.greet`)
    h.realms[0].send({ t: 'bus', frame: '{"jsonrpc":"2.0","id":"save-2","method":"hew.doc.save","params":{}}' })
    await flush(60)
    expect(saves(h)).toEqual([expect.objectContaining({ id: 'save-2', result: { format: 'hew' } })])
    expect(push2).toHaveBeenCalledTimes(1)
    expect((await call(h, `${ID}.tag`)).result).toEqual({ done: true })
    expect(h.scene.api_dispatch_in_forward.mock.calls.some(([, , f]) => f.includes('hew.doc.save'))).toBe(true)
    expect(push2).toHaveBeenCalledTimes(1)
  })

  describe('storage written behind', () => {
    /** A store whose writes of `held` wait until released. */
    function slowStore(h: Harness, held: string): () => void {
      let release: () => void = () => {}
      const opened = new Promise<void>((r) => (release = r))
      const set = h.storage.set as Mock<PluginStorageBackend['set']>
      const plain = set.getMockImplementation()!
      set.mockImplementation(async (id: string, key: string, value: string) => {
        if (key === held) await opened
        return plain(id, key, value)
      })
      return release
    }

    it('lands writes to one key in the order they were made, across overlapping batches', async () => {
      let hew!: Hew
      h = harness({ main: (api) => ((hew = api), api.commands.handle('greet', () => 'x')) })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      const release = slowStore(h, 'b')
      const first = [hew.storage.set('b', 'x'), hew.storage.set('K', 1)]
      // The debounced batch [b, K=1] starts and waits on b.
      await vi.waitFor(() => expect(h.storage.set).toHaveBeenCalledWith(ID, 'b', '"x"'))
      const second = hew.storage.set('K', 2)
      const listed = hew.storage.list()
      await flush(60)
      release()
      await Promise.all([...first, second])
      await expect(listed).resolves.toEqual(['K', 'b'])
      expect(h.stored.get('K')).toBe('2')
    })

    it('reads its own write while that write is on its way to the store', async () => {
      let hew!: Hew
      h = harness({ main: (api) => ((hew = api), api.commands.handle('greet', () => 'x')) })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      const release = slowStore(h, 'k')
      const write = hew.storage.set('k', 'a')
      await vi.waitFor(() => expect(h.storage.set).toHaveBeenCalledWith(ID, 'k', '"a"'))
      const read = hew.storage.get('k')
      await flush(60)
      release()
      await write
      await expect(read).resolves.toBe('a')
    })

    it('reads what is really stored after a write that failed, and the failure reaches the write', async () => {
      let hew!: Hew
      h = harness({ main: (api) => ((hew = api), api.commands.handle('greet', () => 'x')) })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      await hew.storage.set('k', 'small')
      // The expectation is attached before anything is awaited, so the
      // rejection always has a handler when it arrives.
      const refused = expect(hew.storage.set('k', 'x'.repeat(200))).rejects.toMatchObject({ name: 'storage_quota' })
      await vi.waitFor(() => expect(h.storage.set).toHaveBeenCalledTimes(2))
      const read = hew.storage.get('k')
      await refused
      await expect(read).resolves.toBe('small')
    })
  })

  describe('file requests', () => {
    const MiB = 1 << 20
    function sizedFiles(sizes: Record<string, number>): { readFile: HarnessOpts['readFile']; reads: string[]; maxAtOnce: () => number } {
      const reads: string[] = []
      let open = 0
      let most = 0
      return {
        reads,
        maxAtOnce: () => most,
        readFile: async (id, path) => {
          if (path === 'dist/background.js') return new TextEncoder().encode(id).buffer as ArrayBuffer
          reads.push(path)
          open++
          most = Math.max(most, open)
          await new Promise((r) => setTimeout(r, 1))
          open--
          if (!(path in sizes)) throw new Error('no such file')
          return new ArrayBuffer(sizes[path])
        },
      }
    }
    const filesReply = (h: Harness, id: number) =>
      h.realms[0].toRealm.find((m) => m.t === 'files' && m.id === id) as Extract<HostToRealm, { t: 'files' }> | undefined

    it('reads a path named many times once', async () => {
      const files = sizedFiles({ 'a.bin': 1024 })
      h = harness({ readFile: files.readFile })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      h.realms[0].send({ t: 'files', id: 900, paths: Array.from({ length: 16 }, () => 'a.bin') })
      await vi.waitFor(() => expect(filesReply(h, 900)).toBeDefined())
      expect(files.reads).toEqual(['a.bin'])
      expect(filesReply(h, 900)!.entries.map((e) => e.path)).toEqual(['a.bin'])
      expect(filesReply(h, 900)!.missing).toEqual([])
      // Every path named is answered once, a missing one included.
      h.realms[0].send({ t: 'files', id: 901, paths: ['nope.bin', 'a.bin', 'nope.bin', 'a.bin'] })
      await vi.waitFor(() => expect(filesReply(h, 901)).toBeDefined())
      expect(filesReply(h, 901)!.entries.map((e) => e.path)).toEqual(['a.bin'])
      expect(filesReply(h, 901)!.missing).toEqual(['nope.bin'])
      expect(h.host.states()[ID]).toBe('activated')
    })

    it('answers a request that failed in the host, every path missing', async () => {
      let failFiles = false
      h = harness({
        sched: {
          // Fails only where the host counts file bytes, standing in for any
          // fault in serving a file request.
          now: () => {
            if (failFiles && (new Error().stack ?? '').includes('filesReadLastSecond')) throw new Error('boom')
            return Date.now()
          },
          later: (fn) => queueMicrotask(fn),
        },
      })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      failFiles = true
      h.realms[0].send({ t: 'files', id: 905, paths: ['a.bin', 'b.bin', 'a.bin'] })
      await vi.waitFor(() => expect(filesReply(h, 905)).toBeDefined())
      expect(filesReply(h, 905)).toMatchObject({ entries: [], missing: ['a.bin', 'b.bin'] })
      expect(h.logs.some((l) => l.includes('a file request from'))).toBe(true)
      expect(h.host.states()[ID]).toBe('activated')
    })

    it('bounds the bytes of one request and of one second, serves requests one at a time, and answers the rest missing', async () => {
      const files = sizedFiles({ 'a.bin': 40 * MiB, 'b.bin': 40 * MiB, 'c.bin': 40 * MiB, 'd.bin': 40 * MiB })
      h = harness({ readFile: files.readFile })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      h.realms[0].send({ t: 'files', id: 901, paths: ['a.bin', 'b.bin'] })
      h.realms[0].send({ t: 'files', id: 902, paths: ['c.bin'] })
      h.realms[0].send({ t: 'files', id: 903, paths: ['d.bin'] })
      h.realms[0].send({ t: 'files', id: 904, paths: ['a.bin'] })
      await vi.waitFor(() => expect(filesReply(h, 904)).toBeDefined())
      // One request carries at most 64 MiB: a, not b.
      expect(filesReply(h, 901)!.entries.map((e) => e.path)).toEqual(['a.bin'])
      expect(filesReply(h, 901)!.missing).toEqual(['b.bin'])
      expect(filesReply(h, 902)!.entries.map((e) => e.path)).toEqual(['c.bin'])
      // d is read (120 MiB so far) but takes the second past 128 MiB, so it is
      // missing; after that nothing more is read in this second.
      expect(filesReply(h, 903)!.missing).toEqual(['d.bin'])
      expect(filesReply(h, 904)!.missing).toEqual(['a.bin'])
      expect(files.reads).toEqual(['a.bin', 'b.bin', 'c.bin', 'd.bin'])
      expect(files.maxAtOnce()).toBe(1)
      expect(h.host.states()[ID]).toBe('activated')
    })
  })
})

// ---------------------------------------------------------- folder plugins

describe('the plugin host, a folder plugin', () => {
  let h: Harness
  beforeEach((ctx) => {
    requiresPreview(ctx, 'plugins')
    setBusy(null)
  })
  afterEach(() => {
    h?.host.dispose()
    vi.useRealTimers()
  })

  const FOLDER = { kind: 'dir', path: '/home/me/hello' } as const
  const folder = (over: Partial<Installed> = {}, m: PluginManifest = manifest()): Listed[] => [
    { kind: 'installed', installed: installed({ source: FOLDER, ...over }), manifest: m, previous_version: null },
  ]
  const startup = (): PluginManifest => manifest({ activation: ['on_startup', 'on_command'] })
  const unchanged: DevChange = { id: ID, manifest_changed: false, reason: null, problem: null }
  const sentTo = (h: Harness, realm: number, t: HostToRealm['t']): HostToRealm[] => h.realms[realm].toRealm.filter((m) => m.t === t)
  /** Not stopped as a crash: no crash record, no toast, no error. */
  const notCrashed = (h: Harness): void => {
    expect(h.scene.states.filter(([, s]) => s === 'crashed')).toEqual([])
    expect(h.toasts).toEqual([])
    expect(h.logs.filter((l) => l.startsWith('error:'))).toEqual([])
  }

  describe('not startable', () => {
    it('while developer mode is off: blocked, its commands kept and answering unavailable, nothing started', async () => {
      h = harness({ list: folder(), devMode: async () => false })
      await h.host.sync()
      expect(h.host.states()).toEqual({ [ID]: 'blocked' })
      expect(h.host.reasons()).toEqual({ [ID]: 'needs_developer_mode' })
      expect(h.scene.registered.get(ID)!.map((d) => d.id)).toEqual(['greet', 'tag'])
      expect((await call(h, `${ID}.greet`)).error?.data?.refusal).toBe('plugin_unavailable')
      expect((await call(h, `${ID}.tag`)).error?.data?.refusal).toBe('plugin_unavailable')
      expect(h.scene.compounds).toEqual([])
      expect(getBusy()).toBeNull()
      expect(h.realms).toHaveLength(0)
      notCrashed(h)
    })

    it('while developer mode cannot be read, a folder plugin is held as if it were off; a package is not', async () => {
      h = harness({
        list: [
          ...folder(),
          { kind: 'installed', installed: installed({ id: OTHER_ID }), manifest: manifest({ id: OTHER_ID }), previous_version: null },
        ],
        devMode: async () => {
          throw new Error('no shell')
        },
      })
      await h.host.sync()
      expect(h.host.states()).toEqual({ [ID]: 'blocked', [OTHER_ID]: 'enabled' })
    })

    it('runs again as soon as developer mode is on, without being turned off and on', async () => {
      let on = false
      h = harness({ list: folder({ startup_allowed: true }, startup()), devMode: async () => on })
      await h.host.sync()
      await flush(60)
      expect(h.realms).toHaveLength(0)
      on = true
      h.changed()
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
      expect(h.host.reasons()).toEqual({})
      expect((await call(h, `${ID}.greet`)).result).toEqual({ greeting: 'Hello, Hew!' })
    })

    it('stops a running plugin when developer mode goes off, its open forward answered unavailable', async () => {
      let on = true
      h = harness({ list: folder(), devMode: async () => on, main: (hew) => hew.commands.handle('greet', () => new Promise(() => {})) })
      await h.host.sync()
      const reply = call(h, `${ID}.greet`)
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
      await flush(20)
      on = false
      h.changed()
      expect((await reply).error?.data?.refusal).toBe('plugin_unavailable')
      expect(h.host.states()).toEqual({ [ID]: 'blocked' })
      expect(h.host.reasons()).toEqual({ [ID]: 'needs_developer_mode' })
      expect(h.realms[0].toRealm.at(-1)).toEqual({ t: 'unload', reason: 'disable' })
      expect(h.realms[0].channel.remove).toHaveBeenCalled()
      expect(h.scene.registered.has(ID)).toBe(true)
      notCrashed(h)
    })

    it('holds a record waiting for review, whatever its marker holds but false, and runs it once the store clears it', async () => {
      for (const marker of [true, 'permissions_changed', null, 0, {}]) {
        h = harness({ list: folder({ needs_consent: marker }), devMode: async () => true })
        await h.host.sync()
        expect(h.host.reasons(), String(marker)).toEqual({ [ID]: 'needs_review' })
        expect((await call(h, `${ID}.greet`)).error?.data?.refusal).toBe('plugin_unavailable')
        h.host.dispose()
      }
      h = harness({ list: folder({ needs_consent: false }), devMode: async () => true })
      await h.host.sync()
      expect(h.host.states()).toEqual({ [ID]: 'enabled' })
      h.host.dispose()

      // A packaged plugin's record with the marker is held too.
      h = harness({ list: [{ kind: 'installed', installed: installed({ needs_consent: true }), manifest: manifest(), previous_version: null }] })
      await h.host.sync()
      expect(h.host.reasons()).toEqual({ [ID]: 'needs_review' })
      h.list = [{ kind: 'installed', installed: installed(), manifest: manifest(), previous_version: null }]
      h.changed()
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('enabled'))
      expect((await call(h, `${ID}.greet`)).result).toEqual({ greeting: 'Hello, Hew!' })
      notCrashed(h)
    })

    it("marked while running: stopped at once, and a review's acceptance starts it with Hew again", async () => {
      h = harness({ list: folder({ startup_allowed: true }, startup()), devMode: async () => true })
      await h.host.sync()
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
      h.list = folder({ startup_allowed: true, needs_consent: true, needs_consent_reason: 'permissions_changed' }, startup())
      h.changed()
      h.devChanged({ id: ID, manifest_changed: true, reason: 'permissions_changed', problem: null })
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('blocked'))
      expect(h.host.reasons()).toEqual({ [ID]: 'needs_review' })
      expect(h.realms[0].toRealm.at(-1)).toEqual({ t: 'unload', reason: 'disable' })
      await flush(60)
      expect(h.realms).toHaveLength(1)
      // Accepted: the shell clears the marker and says nothing changed.
      h.list = folder({ startup_allowed: true }, startup())
      h.changed()
      h.devChanged(unchanged)
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
      notCrashed(h)
    })

    it('a start whose read is refused is blocked by name, not stopped, and the next call tries again', async () => {
      const refusals: [string, string][] = [
        ['folder_missing: Hew cannot open the plugin\'s folder "/home/me/hello".', 'folder_missing'],
        ['package_path_invalid: dist/background.js', 'main_missing'],
        ['developer_mode_off: Turn on developer mode.', 'needs_developer_mode'],
        ['dev_consent_pending: Review the change.', 'needs_review'],
      ]
      for (const [refusal, reason] of refusals) {
        let failing = true
        h = harness({
          list: folder(),
          devMode: async () => true,
          readFile: async (id) => {
            if (failing) throw refusal
            return new TextEncoder().encode(id).buffer as ArrayBuffer
          },
        })
        await h.host.sync()
        expect((await call(h, `${ID}.greet`)).error?.data?.refusal, reason).toBe('plugin_unavailable')
        expect(h.host.states()).toEqual({ [ID]: 'blocked' })
        expect(h.host.reasons()).toEqual({ [ID]: reason })
        expect(h.realms).toHaveLength(0)
        expect(h.scene.registered.has(ID)).toBe(true)
        notCrashed(h)
        failing = false
        if (reason === 'folder_missing' || reason === 'main_missing') {
          // The folder is back: the next call starts it.
          expect((await call(h, `${ID}.greet`)).result).toEqual({ greeting: 'Hello, Hew!' })
        } else {
          // Held by what the store says until the store says otherwise.
          expect((await call(h, `${ID}.greet`)).error?.data?.refusal).toBe('plugin_unavailable')
          h.changed()
          await flush(60)
          expect((await call(h, `${ID}.greet`)).result).toEqual({ greeting: 'Hello, Hew!' })
        }
        h.host.dispose()
      }
    })

    it('a packaged plugin whose code cannot be read is still stopped as before', async () => {
      h = harness({
        readFile: async () => {
          throw 'package_path_invalid: dist/background.js'
        },
      })
      await h.host.sync()
      expect((await call(h, `${ID}.greet`)).error?.data?.reason).toBe('crashed')
      expect(h.host.states()[ID]).toBe('crashed')
    })

    it('a refusal name in text the plugin could have written does not count', async () => {
      h = harness({
        list: folder(),
        devMode: async () => true,
        readFile: async () => {
          throw new Error('the frame said: folder_missing: x')
        },
      })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      expect(h.host.states()[ID]).toBe('crashed')
    })

    it("a folder's manifest whose main went missing after a refresh is blocked, not stopped, and runs again once saved with one", async () => {
      h = harness({ list: folder(), devMode: async () => true })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      const { main: _gone, ...rest } = manifest() as PluginManifest & { main?: unknown }
      h.list = folder({}, rest as PluginManifest)
      h.devChanged(unchanged)
      await vi.waitFor(() => expect(h.realms[0].toRealm.at(-1)).toEqual({ t: 'unload', reason: 'reload' }))
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('blocked'))
      expect(h.host.reasons()).toEqual({ [ID]: 'main_missing' })
      notCrashed(h)
      // Saved again with a main: reloaded and running.
      h.list = folder()
      h.devChanged(unchanged)
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
    })
  })

  describe('a reload', () => {
    it('starts a plugin again when a review mark that stopped it while it ran clears', async () => {
      h = harness({ list: folder(), devMode: async () => true })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      expect(h.host.states()[ID]).toBe('activated')
      h.list = folder({ needs_consent: true, needs_consent_reason: 'permissions_changed' })
      h.changed()
      await vi.waitFor(() => expect(h.host.reasons()[ID]).toBe('needs_review'))
      // The person undid the change in the folder: the mark is gone.
      h.list = folder()
      h.changed()
      await flush(60)
      h.devChanged(unchanged)
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
      notCrashed(h)
    })

    it('does not start again after a stop that was not a folder refresh: a grant change, or developer mode off', async () => {
      h = harness({ list: folder(), devMode: async () => true })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      // The person narrowed the grant: the plugin is rebuilt, not running.
      h.list = folder({ grant: [] })
      h.changed()
      await flush(60)
      expect(h.host.states()[ID]).toBe('enabled')
      // A later save in its folder reloads it, but it was not running.
      h.devChanged(unchanged)
      await flush(60)
      expect(h.host.states()[ID]).toBe('enabled')
      expect(h.realms).toHaveLength(1)
      h.host.dispose()

      let on = true
      h = harness({ list: folder(), devMode: async () => on })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      on = false
      h.changed()
      await vi.waitFor(() => expect(h.host.reasons()[ID]).toBe('needs_developer_mode'))
      on = true
      h.changed()
      await flush(60)
      h.devChanged(unchanged)
      await flush(60)
      expect(h.host.states()[ID]).toBe('enabled')
      expect(h.realms).toHaveLength(1)
    })

    it('starts a running plugin again when the refresh changed its version: the store change comes first, the reload after', async () => {
      h = harness({ list: folder(), devMode: async () => true })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      expect(h.host.states()[ID]).toBe('activated')
      // The shell refreshed the stored copy (a new version): it says the
      // store changed, then asks for the reload.
      h.list = folder({ version: '1.1.0' }, manifest({ version: '1.1.0' }))
      h.changed()
      // The store change is read before the reload is asked for.
      await flush(60)
      expect(h.host.states()[ID]).toBe('enabled')
      h.devChanged(unchanged)
      await vi.waitFor(() => expect(h.realms).toHaveLength(2))
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
      notCrashed(h)
    })

    it('keeps a reload asked for when the store cannot be listed once, and does it on a later try', async () => {
      h = harness({ list: folder(), devMode: async () => true })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      const list = h.env.store.list.bind(h.env.store)
      let fail = 1
      h.env.store.list = async () => {
        if (fail > 0) {
          fail--
          throw new Error('busy')
        }
        return list()
      }
      h.devChanged(unchanged)
      await vi.waitFor(() => expect(h.realms).toHaveLength(2), { timeout: 3_000 })
      expect(h.realms[0].toRealm.at(-1)).toEqual({ t: 'unload', reason: 'reload' })
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
      expect(h.logs.some((l) => l.startsWith('warn: The plugin list could not be read'))).toBe(true)
    })

    it('stops trying again after a few failed lists', async () => {
      h = harness({ list: folder(), devMode: async () => true })
      await h.host.sync()
      const list = vi.fn(async (): Promise<Listed[]> => {
        throw new Error('busy')
      })
      h.env.store.list = list
      h.devChanged(unchanged)
      await vi.waitFor(() => expect(list).toHaveBeenCalledTimes(4), { timeout: 3_000 })
      await new Promise((r) => setTimeout(r, 1_000))
      expect(list).toHaveBeenCalledTimes(4)
    })

    it("on a folder change stops the realm with reload, registers the stored manifest's commands again, and starts it again because it was running", async () => {
      h = harness({ list: folder(), devMode: async () => true })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      const m = manifest({ contributes: { commands: [{ id: 'greet', class: 'read_only', summary: 'Greets.' }, { id: 'wave', class: 'read_only', summary: 'Waves.' }] } })
      h.list = folder({}, m)
      h.devChanged(unchanged)
      await vi.waitFor(() => expect(h.realms).toHaveLength(2))
      expect(h.realms[0].toRealm.at(-1)).toEqual({ t: 'unload', reason: 'reload' })
      expect(h.realms[0].channel.remove).toHaveBeenCalled()
      expect(h.scene.registered.get(ID)!.map((d) => d.id)).toEqual(['greet', 'wave'])
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
      expect(h.scene.states.filter(([, s]) => s === 'unloaded')).toEqual([[ID, 'unloaded', 'reload']])
      expect((await call(h, `${ID}.greet`)).result).toEqual({ greeting: 'Hello, Hew!' })
      expect(h.realms).toHaveLength(2)
      notCrashed(h)
    })

    it('does not start a plugin that was not running and does not start with Hew, and starts one that does', async () => {
      h = harness({ list: folder(), devMode: async () => true })
      await h.host.sync()
      h.devChanged(unchanged)
      await flush(60)
      expect(h.realms).toHaveLength(0)
      expect(h.host.states()).toEqual({ [ID]: 'enabled' })
      h.host.dispose()
      h = harness({ list: folder({ startup_allowed: true }, startup()), devMode: async () => true })
      await h.host.sync()
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
      h.devChanged(unchanged)
      await vi.waitFor(() => expect(h.realms).toHaveLength(2))
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
      expect((sentTo(h, 1, 'hello')[0] as { activation: string }).activation).toBe('startup')
    })

    it('keeps the storage written behind: a write on its way lands, and the reloaded plugin reads it', async () => {
      let hew!: Hew
      h = harness({ list: folder(), devMode: async () => true, timings: { storageDebounceMs: 1_000 }, main: (api) => ((hew = api), api.commands.handle('greet', () => 'x')) })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      const first = hew
      const write = first.storage.set('k', 'before')
      await flush(20)
      expect(h.storage.set).not.toHaveBeenCalled()
      h.devChanged(unchanged)
      await vi.waitFor(() => expect(h.realms).toHaveLength(2))
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
      expect(hew).not.toBe(first)
      await expect(hew.storage.get('k')).resolves.toBe('before')
      expect(h.stored.get('k')).toBe('"before"')
      // The old realm is gone; its own promise is nobody's to settle.
      void write.catch(() => undefined)
    })

    it('during an open forward settles it unavailable as any reload does, and leaves no compound open', async () => {
      h = harness({
        list: folder({}, withSlowManifest()),
        devMode: async () => true,
        main: (hew) => {
          hew.commands.handle('greet', () => 'hi')
          hew.commands.handle('slow', () => new Promise(() => {}))
        },
      })
      await h.host.sync()
      const reply = call(h, `${ID}.slow`)
      await vi.waitFor(() => expect(h.scene.compounds).toHaveLength(1))
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
      h.devChanged(unchanged)
      expect((await reply).error?.data?.refusal).toBe('plugin_unavailable')
      expect(h.scene.compounds).toEqual([])
      expect(getBusy()).toBeNull()
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
      expect(h.realms).toHaveLength(2)
      // A late answer from the stopped realm reaches nobody.
      h.realms[0].send({ t: 'forward_reply', id: 1, reply: '{"ok":"late"}' })
      await flush(60)
      expect(h.host.states()[ID]).toBe('activated')
      expect((await call(h, `${ID}.greet`)).result).toBe('hi')
      notCrashed(h)
    })

    it('during activation starts it once, answers the waiting forward unavailable, and leaves no compound open', async () => {
      const read = gate<ArrayBuffer>()
      h = harness({
        list: folder({}, withSlowManifest()),
        devMode: async () => true,
        readFile: () => read.promise,
        main: (hew) => {
          hew.commands.handle('greet', () => 'hi')
          hew.commands.handle('tag', () => ({ done: true }))
        },
      })
      await h.host.sync()
      const reply = call(h, `${ID}.tag`)
      await vi.waitFor(() => expect(h.scene.compounds).toHaveLength(1))
      expect(h.host.states()[ID]).toBe('activating')
      h.devChanged(unchanged)
      await flush(60)
      read.open(new TextEncoder().encode(ID).buffer as ArrayBuffer)
      expect((await reply).error?.data?.refusal).toBe('plugin_unavailable')
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
      await flush(60)
      expect(h.realms).toHaveLength(1)
      expect(h.scene.compounds).toEqual([])
      expect(getBusy()).toBeNull()
      expect(h.scene.states.filter(([, s]) => s === 'loaded')).toHaveLength(1)
      expect((await call(h, `${ID}.greet`)).result).toBe('hi')
    })

    it('two change events in a row reload it once', async () => {
      h = harness({ list: folder(), devMode: async () => true })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      h.devChanged(unchanged)
      h.devChanged(unchanged)
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
      await flush(100)
      expect(h.realms).toHaveLength(2)
      expect(h.host.states()[ID]).toBe('activated')
      expect(sentTo(h, 1, 'unload')).toEqual([])
    })

    it('a change event for a plugin that is off, or not installed, starts nothing', async () => {
      h = harness({ list: folder({ enabled: false }), devMode: async () => true })
      await h.host.sync()
      h.devChanged(unchanged)
      h.devChanged({ ...unchanged, id: 'com.example.nobody' })
      await flush(100)
      expect(h.realms).toHaveLength(0)
      expect(h.host.states()).toEqual({})
      expect(h.scene.registered.size).toBe(0)
    })

    it('a change event racing a disable leaves nothing running and nothing registered', async () => {
      h = harness({ list: folder(), devMode: async () => true })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      h.devChanged(unchanged)
      h.list = folder({ enabled: false })
      h.changed()
      await flush(100)
      await h.host.sync()
      expect(h.host.states()).toEqual({})
      expect(h.scene.registered.has(ID)).toBe(false)
      for (const r of h.realms) expect(r.channel.remove).toHaveBeenCalled()
      expect(h.scene.conns.size === 0 || [...h.scene.conns.values()].every((c) => c.plugin !== ID)).toBe(true)
    })

    it('a folder problem or a marked change does not reload', async () => {
      h = harness({ list: folder(), devMode: async () => true })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      h.devChanged({ ...unchanged, problem: 'folder_missing' })
      await flush(100)
      expect(h.realms).toHaveLength(1)
      expect(h.host.states()[ID]).toBe('activated')
    })

    it('brings back a folder plugin that was stopped, since the person saved its folder', async () => {
      h = harness({ list: folder(), devMode: async () => true })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      h.realms[0].send({ t: 'error', message: 'boom' })
      await flush(20)
      expect(h.host.states()[ID]).toBe('crashed')
      h.devChanged(unchanged)
      await vi.waitFor(() => expect(h.host.states()[ID]).toBe('enabled'))
      expect(h.scene.registered.has(ID)).toBe(true)
      expect((await call(h, `${ID}.greet`)).result).toEqual({ greeting: 'Hello, Hew!' })
    })
  })
})

const OTHER_ID = 'com.example.other'

/** The hello plugin's manifest with `slow` (mutating) beside its commands. */
function withSlowManifest(): PluginManifest {
  return manifest({
    contributes: {
      commands: [
        { id: 'greet', class: 'read_only', summary: 'Greets.' },
        { id: 'tag', class: 'mutating', summary: 'Adds a tag.' },
        { id: 'slow', class: 'mutating', summary: 'Takes its time.' },
      ],
    },
  })
}

// ------------------------------------------- idle, grants, events, requests

/** The plugin's own connections open in the kernel: its realm's and its
 *  listener's. */
const pluginConns = (h: Harness, id = ID): number[] => [...h.scene.conns].filter(([, c]) => c.plugin === id).map(([conn]) => conn)

/** Every message of type `t` the host sent realm `n`. */
const sent = (h: Harness, n: number, t: HostToRealm['t']): HostToRealm[] => h.realms[n].toRealm.filter((m) => m.t === t)

/** The event frames realm `n` was sent, as `name:generation`. */
const eventsSent = (h: Harness, n: number): string[] =>
  sent(h, n, 'bus')
    .map((m) => JSON.parse((m as { frame: string }).frame) as { method?: string; params?: { n?: number } })
    .filter((f) => f.method?.startsWith('hew.event.') === true)
    .map((f) => `${f.method!.slice('hew.event.'.length)}:${f.params?.n ?? ''}`)

const listed = (inst: Partial<Installed> = {}, man: Partial<Record<string, unknown>> = {}): Listed[] => [
  { kind: 'installed', installed: installed(inst), manifest: manifest(man), previous_version: null },
]

/** A manifest that starts the plugin on `document_changed`. */
const onEvent = (events: string[] = ['document_changed']): Partial<Record<string, unknown>> => ({
  activation: ['on_command', 'on_event'],
  contributes: { commands: [{ id: 'greet', class: 'read_only', summary: 'Greets.' }], events },
})

/** Emits `name` with a number, and drains. */
function emitAndPump(h: Harness, name: string, n: number): void {
  h.scene.emit(name, { n })
  pumpEvents(h.scene as unknown as Scene)
}

describe('the plugin host, idle unload', () => {
  let h: Harness
  beforeEach((ctx) => {
    requiresPreview(ctx, 'plugins')
    setBusy(null)
  })
  afterEach(() => {
    h?.host.dispose()
    vi.useRealTimers()
  })

  const quick = { pingMs: 10, idleMs: 60, drainMs: 200 }

  it('unloads a realm quiet for the idle time, keeps its commands, and starts it again on the next command', async () => {
    h = harness({ timings: quick })
    await h.host.sync()
    expect((await call(h, `${ID}.greet`)).result).toEqual({ greeting: 'Hello, Hew!' })
    expect(pluginConns(h)).toHaveLength(1)
    await vi.waitFor(() => expect(h.host.states()[ID]).toBe('enabled'))
    expect(sent(h, 0, 'unload')).toEqual([{ t: 'unload', reason: 'idle' }])
    expect(h.realms[0].channel.remove).toHaveBeenCalled()
    expect(h.scene.registered.has(ID)).toBe(true)
    expect(h.scene.states).toContainEqual([ID, 'unloaded', 'idle'])
    expect(pluginConns(h)).toEqual([])
    expect(h.toasts).toEqual([])
    expect((await call(h, `${ID}.greet`, { name: 'again' })).result).toEqual({ greeting: 'Hello, again!' })
    expect(h.realms).toHaveLength(2)
    expect((h.realms[1].toRealm[0] as { activation: string }).activation).toBe('command')
  })

  it('keeps a realm with a forward open', async () => {
    h = harness({ timings: quick, main: (hew) => hew.commands.handle('greet', () => new Promise(() => {})) })
    await h.host.sync()
    void call(h, `${ID}.greet`)
    await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
    await new Promise((r) => setTimeout(r, 250))
    expect(h.host.states()[ID]).toBe('activated')
    expect(sent(h, 0, 'unload')).toEqual([])
  })

  it('keeps a realm that keeps talking after its command answered, and unloads it once it stops', async () => {
    let timer: ReturnType<typeof setInterval> | undefined
    h = harness({
      timings: quick,
      main: (hew) => {
        hew.commands.handle('greet', () => {
          let n = 0
          timer = setInterval(() => {
            if (++n > 12) clearInterval(timer)
            else void hew.storage.get('k')
          }, 20)
          return 'started'
        })
      },
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    await new Promise((r) => setTimeout(r, 180))
    expect(h.host.states()[ID]).toBe('activated')
    await vi.waitFor(() => expect(h.host.states()[ID]).toBe('enabled'), { timeout: 2_000 })
    clearInterval(timer)
  })

  it('never unloads a plugin that starts with Hew', async () => {
    h = harness({ timings: quick, list: listed({ startup_allowed: true }, { activation: ['on_startup', 'on_command'] }) })
    await h.host.sync()
    await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
    await new Promise((r) => setTimeout(r, 250))
    expect(h.host.states()[ID]).toBe('activated')
    expect(sent(h, 0, 'unload')).toEqual([])
  })

  it('takes the storage writes an unload listener makes, drops anything else it sends then, and stops nothing', async () => {
    h = harness({
      timings: quick,
      main: (hew) => {
        hew.commands.handle('greet', () => 'hi')
        hew.lifecycle.on('unload', (reason) => {
          void hew.api.query.scene({}).catch(() => {})
          return hew.storage.set('bye', reason)
        })
      },
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    await vi.waitFor(() => expect(h.host.states()[ID]).toBe('enabled'))
    await vi.waitFor(() => expect(h.stored.get('bye')).toBe('"idle"'))
    // The bus call made during the drain never reached the kernel.
    const realmConnCalls = h.scene.api_dispatch.mock.calls.filter(([, f]) => (JSON.parse(f) as { method: string }).method === 'hew.query.scene')
    expect(realmConnCalls).toEqual([])
    expect(h.scene.states.filter(([, s]) => s === 'crashed')).toEqual([])
    expect(h.toasts).toEqual([])
    expect(pluginConns(h)).toEqual([])
  })

  it('tears down a realm that does not say unloaded within the drain time', async () => {
    h = harness({
      timings: quick,
      main: (hew) => {
        hew.commands.handle('greet', () => 'hi')
        hew.lifecycle.on('unload', () => new Promise(() => {}))
      },
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    await vi.waitFor(() => expect(sent(h, 0, 'unload')).toHaveLength(1))
    const at = Date.now()
    await vi.waitFor(() => expect(h.host.states()[ID]).toBe('enabled'))
    expect(Date.now() - at).toBeGreaterThanOrEqual(150)
    expect(h.realms[0].channel.remove).toHaveBeenCalled()
    expect(pluginConns(h)).toEqual([])
  })

  it('lands the storage writes still waiting in the inbox when the drain time ends', async () => {
    const s = holdingSched()
    h = harness({
      timings: quick,
      sched: s.sched,
      main: (hew) => {
        hew.commands.handle('greet', () => 'hi')
        hew.lifecycle.on('unload', () => {
          void hew.storage.set('bye', 1)
          void hew.storage.set('last', 2)
          return new Promise(() => {})
        })
      },
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    // From here the host takes the realm's messages but its drain does not
    // get to them before the drain time ends.
    s.hold()
    await vi.waitFor(() => expect(sent(h, 0, 'unload')).toHaveLength(1))
    await vi.waitFor(() => expect(h.host.states()[ID]).toBe('enabled'))
    await vi.waitFor(() => expect([h.stored.get('bye'), h.stored.get('last')]).toEqual(['1', '2']))
    expect(h.toasts).toEqual([])
    s.release()
  })

  it("gives a realm longer to unload than the SDK waits for its listeners, so a realm that keeps to its limit is heard out", () => {
    expect(DEFAULT_TIMINGS.drainMs).toBeGreaterThan(RUNTIME_LIMITS.unloadWaitMs)
  })

  it('makes commands that come while it unloads wait, then starts the plugin once for all of them', async () => {
    const release = gate()
    h = harness({
      timings: { ...quick, drainMs: 2_000 },
      main: (hew) => {
        hew.commands.handle('greet', (p) => ({ greeting: `Hello, ${(p as { name?: string }).name ?? 'Hew'}!` }))
        hew.lifecycle.on('unload', () => release.promise)
      },
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    await vi.waitFor(() => expect(sent(h, 0, 'unload')).toHaveLength(1))
    const a = call(h, `${ID}.greet`, { name: 'a' })
    const b = call(h, `${ID}.greet`, { name: 'b' })
    await flush(60)
    expect(h.realms).toHaveLength(1)
    release.open()
    expect((await a).result).toEqual({ greeting: 'Hello, a!' })
    expect((await b).result).toEqual({ greeting: 'Hello, b!' })
    expect(h.realms).toHaveLength(2)
    expect(sent(h, 0, 'forward')).toHaveLength(1)
    expect(pluginConns(h)).toHaveLength(1)
  })

  it('a realm turned off while it unloads is gone, and the waiting command hears unavailable', async () => {
    h = harness({
      timings: { ...quick, drainMs: 2_000 },
      main: (hew) => {
        hew.commands.handle('greet', () => 'hi')
        hew.lifecycle.on('unload', () => new Promise(() => {}))
      },
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    await vi.waitFor(() => expect(sent(h, 0, 'unload')).toHaveLength(1))
    const waiting = call(h, `${ID}.greet`)
    await flush(20)
    h.list = listed({ enabled: false })
    await h.host.sync()
    expect((await waiting).error?.data?.refusal).toBe('plugin_unavailable')
    expect(h.realms).toHaveLength(1)
    expect(pluginConns(h)).toEqual([])
  })
})

describe('the plugin host, a grant widened while the plugin runs', () => {
  let h: Harness
  beforeEach((ctx) => {
    requiresPreview(ctx, 'plugins')
    setBusy(null)
  })
  afterEach(() => h?.host.dispose())

  it('applies a grant that only adds optional permissions in place: same realm, same connection, the realm told', async () => {
    let hew!: Hew
    const grants: unknown[] = []
    h = harness({
      list: listed({ optional_granted: [] }, { optionalPermissions: ['model.write', 'view'] }),
      main: (api) => {
        hew = api
        api.commands.handle('greet', () => 'hi')
        api.lifecycle.on('grant', (g) => grants.push(g))
      },
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    const conn = pluginConns(h)[0]
    h.list = listed({ optional_granted: ['view'] }, { optionalPermissions: ['model.write', 'view'] })
    await h.host.sync()
    await flush(20)
    expect(h.realms).toHaveLength(1)
    expect(sent(h, 0, 'unload')).toEqual([])
    expect(h.scene.api_connection_set_grant).toHaveBeenCalledTimes(1)
    expect(h.scene.api_connection_set_grant.mock.calls[0][0]).toBe(conn)
    expect(h.scene.conns.get(conn)?.permissions).toEqual(['model.read', 'view'])
    const message = sent(h, 0, 'grant')
    expect(message).toHaveLength(1)
    expect(JSON.parse((message[0] as { grant: string }).grant)).toMatchObject({ permissions: ['model.read', 'view'], plugin: ID })
    expect(h.scene.states).toContainEqual([ID, 'grant_changed', undefined])
    expect(hew.plugin?.grant).toMatchObject({ permissions: ['model.read', 'view'] })
    expect(grants).toHaveLength(1)
    expect(h.host.states()[ID]).toBe('activated')
  })

  it('reloads for anything else: a taken-back permission, a required one, a new version', async () => {
    for (const next of [{ optional_granted: [] }, { grant: ['model.read', 'view'] }, { version: '0.2.0' }] as Partial<Installed>[]) {
      h = harness({ list: listed({ optional_granted: ['model.write'] }) })
      await h.host.sync()
      await call(h, `${ID}.greet`)
      h.list = listed({ optional_granted: ['model.write'], ...next })
      await h.host.sync()
      expect(sent(h, 0, 'unload'), JSON.stringify(next)).toEqual([{ t: 'unload', reason: 'reload' }])
      expect(sent(h, 0, 'grant')).toEqual([])
      expect(h.scene.api_connection_set_grant).not.toHaveBeenCalled()
      h.host.dispose()
    }
  })

  it('applies a widening that comes while the plugin starts once it is ready', async () => {
    h = harness({
      list: listed({ optional_granted: [] }),
      // The grant was built; the realm is not ready yet.
      beforeOpen: async () => {
        h.list = listed({ optional_granted: ['view'] })
        await h.host.sync()
      },
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    expect(h.grantCalls[0][2]).toBe('["model.read"]')
    // Started with what the record held when the start read it, then told.
    await vi.waitFor(() => expect(sent(h, 0, 'grant')).toHaveLength(1))
    expect(h.scene.conns.get(pluginConns(h)[0])?.permissions).toEqual(['model.read', 'view'])
    expect(h.realms).toHaveLength(1)
  })

  it('the runtime asks again for events its old grant refused, and they arrive', async () => {
    const seen: unknown[] = []
    h = harness({
      list: listed({ optional_granted: [] }),
      main: (api) => {
        api.commands.handle('greet', () => 'hi')
        api.events.on('selection_changed', (p) => seen.push(p))
      },
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    await flush(40)
    const conn = pluginConns(h)[0]
    expect(h.scene.conns.get(conn)?.subscriptions.has('selection_changed')).toBe(false)
    h.list = listed({ optional_granted: ['view'] })
    await h.host.sync()
    await vi.waitFor(() => expect(h.scene.conns.get(conn)?.subscriptions.has('selection_changed')).toBe(true))
    emitAndPump(h, 'selection_changed', 7)
    await vi.waitFor(() => expect(seen).toEqual([{ n: 7 }]))
  })
})

describe('the plugin host, a plugin asking for permissions', () => {
  let h: Harness
  beforeEach((ctx) => {
    requiresPreview(ctx, 'plugins')
    setBusy(null)
  })
  afterEach(() => {
    h?.host.dispose()
    vi.useRealTimers()
  })

  type Asked = { result?: { granted: string[] }; error?: { code: number; data?: { refusal?: string; detail?: { permissions?: string[] } } } }
  /** The plugin's own request, from its realm, as the runtime sends it. */
  let ask: (params: unknown) => Promise<Asked>

  /** A decider that answers what `answer` says, after `wait`, and grants
   *  in the store when it says yes. */
  function decider(answer: () => 'granted' | 'declined', wait: Promise<void> = Promise.resolve()) {
    const asked: [string, string[]][] = []
    return {
      asked,
      grants: {
        async decide(id: string, add: string[]) {
          asked.push([id, add])
          await wait
          const outcome = answer()
          if (outcome === 'granted') {
            const row = h.list[0] as Extract<Listed, { kind: 'installed' }>
            h.list = [{ ...row, installed: { ...row.installed, optional_granted: [...row.installed.optional_granted, ...add] } }]
          }
          return outcome
        },
      },
    }
  }

  async function start(opts: Partial<HarnessOpts> & { optional_granted?: string[] } = {}): Promise<void> {
    h = harness({
      list: listed({ optional_granted: opts.optional_granted ?? [] }, { optionalPermissions: ['model.write', 'view', 'network:api.example.com'] }),
      main: (hew) => {
        hew.commands.handle('greet', () => 'hi')
        ask = async (params) => {
          try {
            return { result: (await hew.api.permissions.request(params as never)) as { granted: string[] } }
          } catch (e) {
            const err = e as { code: number; data?: NonNullable<Asked['error']>['data'] }
            return { error: { code: err.code, data: err.data } }
          }
        }
      },
      ...opts,
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
  }

  it('refuses in order: not on Shop, bad params, not requestable, no act by the person', async () => {
    const d = decider(() => 'granted')
    await start({ platform: 'shop', grants: d.grants })
    h.host.noteUserActivation(ID)
    expect((await ask({ nonsense: true })).error?.data?.refusal).toBe('not_available_in_shop')
    h.host.dispose()
    await start({ grants: d.grants })
    expect((await ask({ permissions: [] })).error?.code).toBe(-32602)
    expect((await ask({ permissions: ['view'], extra: 1 })).error?.code).toBe(-32602)
    const notRequestable = await ask({ permissions: ['view', 'files'] })
    expect(notRequestable.error?.data?.refusal).toBe('permission_not_requestable')
    expect(notRequestable.error?.data?.detail?.permissions).toEqual(['files'])
    expect((await ask({ permissions: ['view'] })).error?.data?.refusal).toBe('no_user_activation')
    expect(d.asked).toEqual([])
  })

  it('a person granting answers with the names now held, applied to the running realm first', async () => {
    const d = decider(() => 'granted')
    await start({ grants: d.grants })
    h.host.noteUserActivation(ID)
    const answer = await ask({ permissions: ['view'], reason: 'To follow your selection.' })
    expect(answer.result).toEqual({ granted: ['view'] })
    expect(d.asked).toEqual([[ID, ['view']]])
    expect(sent(h, 0, 'grant')).toHaveLength(1)
    expect(h.scene.conns.get(pluginConns(h)[0])?.permissions).toEqual(['model.read', 'view'])
    expect(h.realms).toHaveLength(1)
  })

  it('a grant whose store read fails once reads the store again, answers the names granted, and closes the request', async () => {
    const d = decider(() => 'granted')
    let failNext = false
    const grants: NonNullable<HostEnv['grants']> = {
      async decide(id, add) {
        const outcome = await d.grants.decide(id, add)
        failNext = true
        return outcome
      },
    }
    await start({ grants })
    const read = h.env.store.list
    h.env.store.list = async () => {
      if (failNext) {
        failNext = false
        throw new PluginError('io', 'the disk is busy')
      }
      return read()
    }
    h.host.noteUserActivation(ID)
    expect((await ask({ permissions: ['view'] })).result).toEqual({ granted: ['view'] })
    expect(h.logs.some((l) => l.includes('could not be read'))).toBe(true)
    expect(sent(h, 0, 'grant')).toHaveLength(1)
    // The request closed: another may be made.
    h.host.noteUserActivation(ID)
    expect((await ask({ permissions: ['model.write'] })).result).toEqual({ granted: ['model.write'] })
  })

  it('the act by the person is spent by one request and lasts only a few seconds', async () => {
    const d = decider(() => 'granted')
    await start({ grants: d.grants, timings: { activationMs: 50 } })
    h.host.noteUserActivation(ID)
    await new Promise((r) => setTimeout(r, 80))
    expect((await ask({ permissions: ['view'] })).error?.data?.refusal).toBe('no_user_activation')
    h.host.noteUserActivation(ID)
    expect((await ask({ permissions: ['view'] })).result).toEqual({ granted: ['view'] })
    expect((await ask({ permissions: ['model.write'] })).error?.data?.refusal).toBe('no_user_activation')
    // An act for another plugin does not count.
    h.host.noteUserActivation('com.example.other')
    expect((await ask({ permissions: ['model.write'] })).error?.data?.refusal).toBe('no_user_activation')
  })

  it('names already held are granted without asking, and need no act', async () => {
    const d = decider(() => 'declined')
    await start({ grants: d.grants, optional_granted: ['view'] })
    expect((await ask({ permissions: ['view'] })).result).toEqual({ granted: ['view'] })
    expect(d.asked).toEqual([])
  })

  it('a no is refused request_declined, and so is every request for a minute after', async () => {
    const d = decider(() => 'declined')
    await start({ grants: d.grants, timings: { declineMs: 100 } })
    h.host.noteUserActivation(ID)
    expect((await ask({ permissions: ['view'] })).error?.data?.refusal).toBe('request_declined')
    h.host.noteUserActivation(ID)
    expect((await ask({ permissions: ['view'] })).error?.data?.refusal).toBe('request_declined')
    expect(d.asked).toHaveLength(1)
    await new Promise((r) => setTimeout(r, 120))
    h.host.noteUserActivation(ID)
    expect((await ask({ permissions: ['view'] })).error?.data?.refusal).toBe('request_declined')
    expect(d.asked).toHaveLength(2)
  })

  it('one request at a time: a second while the first waits is request_pending', async () => {
    const wait = gate()
    const d = decider(() => 'granted', wait.promise)
    await start({ grants: d.grants })
    h.host.noteUserActivation(ID)
    const first = ask({ permissions: ['view'] })
    await vi.waitFor(() => expect(d.asked).toHaveLength(1))
    h.host.noteUserActivation(ID)
    expect((await ask({ permissions: ['model.write'] })).error?.data?.refusal).toBe('request_pending')
    wait.open()
    expect((await first).result).toEqual({ granted: ['view'] })
  })

  it("is not answered for anyone but the plugin's own realm", async () => {
    const d = decider(() => 'granted')
    await start({ grants: d.grants })
    h.host.noteUserActivation(ID)
    const reply = await call(h, 'hew.permissions.request', { permissions: ['view'] })
    expect(reply.error?.data?.refusal).toBe('host_capability_missing')
    expect(d.asked).toEqual([])
  })

  it('stops the clock of the command it was asked in while the person decides', async () => {
    const wait = gate()
    const d = decider(() => 'granted', wait.promise)
    let hewApi!: Hew
    h = harness({
      list: listed({ optional_granted: [] }, { optionalPermissions: ['view'], contributes: { commands: [{ id: 'greet', class: 'read_only', summary: 'Greets.', timeout_ms: 1_000 }] } }),
      grants: d.grants,
      main: (hew) => {
        hewApi = hew
        hew.commands.handle('greet', async (p) => {
          if ((p as { ask?: boolean }).ask !== true) return 'running'
          return (await hew.api.permissions.request({ permissions: ['view'] })) as { granted: string[] }
        })
      },
    })
    await h.host.sync()
    // Started first: the person's act counts only for a running plugin.
    await call(h, `${ID}.greet`)
    h.host.noteUserActivation(ID)
    const reply = call(h, `${ID}.greet`, { ask: true })
    await vi.waitFor(() => expect(d.asked).toHaveLength(1))
    await new Promise((r) => setTimeout(r, 1_300))
    expect(h.host.states()[ID]).toBe('activated')
    wait.open()
    expect((await reply).result).toEqual({ granted: ['view'] })
    expect(hewApi.plugin).not.toBeNull()
  })

  it('refuses a request nobody answers in time, and the clock runs again', async () => {
    const d = decider(() => 'granted', new Promise(() => {}))
    h = harness({
      list: listed({ optional_granted: [] }, { optionalPermissions: ['view'], contributes: { commands: [{ id: 'greet', class: 'read_only', summary: 'Greets.', timeout_ms: 1_000 }] } }),
      grants: d.grants,
      timings: { decisionMs: 100 },
      main: (hew) => {
        hew.commands.handle('greet', async (p) => {
          if ((p as { ask?: boolean }).ask !== true) return 'running'
          try {
            await hew.api.permissions.request({ permissions: ['view'] })
            return 'granted'
          } catch (e) {
            return (e as { refusal?: { refusal?: string } }).refusal?.refusal ?? 'refused'
          }
        })
      },
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    h.host.noteUserActivation(ID)
    expect((await call(h, `${ID}.greet`, { ask: true })).result).toBe('request_declined')
  })

  it('a decision that comes after the plugin stopped answers nobody and leaves nothing open', async () => {
    const wait = gate()
    const d = decider(() => 'granted', wait.promise)
    await start({ grants: d.grants })
    h.host.noteUserActivation(ID)
    void ask({ permissions: ['view'] })
    await vi.waitFor(() => expect(d.asked).toHaveLength(1))
    h.realms[0].send({ t: 'error', message: 'boom' })
    await flush(20)
    expect(h.host.states()[ID]).toBe('crashed')
    const before = h.realms[0].toRealm.length
    wait.open()
    await flush(60)
    expect(h.realms[0].toRealm.length).toBe(before)
    expect(pluginConns(h)).toEqual([])
  })
})

describe('the plugin host, starting a plugin on an event', () => {
  let h: Harness
  beforeEach((ctx) => {
    requiresPreview(ctx, 'plugins')
    setBusy(null)
  })
  afterEach(() => h?.host.dispose())

  /** A plugin that records the events it hears. */
  const recording = (seen: string[]): Main => (hew) => {
    hew.commands.handle('greet', () => 'hi')
    hew.events.on('document_changed', (p) => seen.push(`document_changed:${(p as { n: number }).n}`))
  }

  it('keeps a listener for a plugin that lists events, and none for one that does not', async () => {
    h = harness({ list: listed({}, onEvent()) })
    await h.host.sync()
    await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]))
    const [conn] = pluginConns(h)
    const methods = h.scene.api_dispatch.mock.calls.filter(([c]) => c === conn).map(([, f]) => (JSON.parse(f) as { method: string }).method)
    expect(methods).toEqual(['hew.meta.hello', 'hew.doc.attach', 'hew.event.subscribe'])
    expect(h.realms).toHaveLength(0)
    h.host.dispose()
    h = harness()
    await h.host.sync()
    await flush(20)
    expect(h.host.listening()).toEqual([])
    expect(pluginConns(h)).toEqual([])
  })

  it('starts the plugin on the first event, hands it the event, and closes the listener', async () => {
    const seen: string[] = []
    h = harness({ list: listed({}, onEvent()), main: recording(seen) })
    await h.host.sync()
    await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]))
    emitAndPump(h, 'document_changed', 1)
    await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
    expect((h.realms[0].toRealm[0] as { activation: string }).activation).toBe('event')
    await vi.waitFor(() => expect(seen).toEqual(['document_changed:1']))
    expect(h.host.listening()).toEqual([])
    expect(pluginConns(h)).toHaveLength(1)
  })

  it('loses no event and delivers none twice across the handover', async () => {
    const seen: string[] = []
    let duringOpen = (): void => {}
    h = harness({ list: listed({}, onEvent()), main: recording(seen), beforeOpen: async () => duringOpen() })
    await h.host.sync()
    await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]))
    // 1 starts it; 2 comes before the handover (to the listener); 3 after
    // the handover and before the realm is ready (on its own connection);
    // 4 once it runs.
    duringOpen = () => emitAndPump(h, 'document_changed', 3)
    emitAndPump(h, 'document_changed', 1)
    emitAndPump(h, 'document_changed', 2)
    await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
    emitAndPump(h, 'document_changed', 4)
    await vi.waitFor(() => expect(seen).toHaveLength(4))
    await flush(40)
    expect(seen).toEqual(['document_changed:1', 'document_changed:2', 'document_changed:3', 'document_changed:4'])
    expect(eventsSent(h, 0)).toEqual(['document_changed:1', 'document_changed:2', 'document_changed:3', 'document_changed:4'])
    expect(pluginConns(h)).toHaveLength(1)
  })

  it('holds at most 64 events while the plugin starts, the newest, and logs the rest once', async () => {
    const seen: string[] = []
    let duringOpen = (): void => {}
    h = harness({ list: listed({}, onEvent()), main: recording(seen), beforeOpen: async () => duringOpen() })
    await h.host.sync()
    await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]))
    duringOpen = () => {
      for (let n = 2; n <= 70; n++) emitAndPump(h, 'document_changed', n)
    }
    emitAndPump(h, 'document_changed', 1)
    await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
    await vi.waitFor(() => expect(seen).toHaveLength(64))
    expect(seen[0]).toBe('document_changed:7')
    expect(seen.at(-1)).toBe('document_changed:70')
    expect(h.logs.filter((l) => l.includes('were dropped'))).toEqual([expect.stringContaining('6 events')])
  })

  it('does not listen for events its grant cannot receive', async () => {
    h = harness({ list: listed({}, onEvent(['selection_changed'])) })
    await h.host.sync()
    await flush(40)
    expect(h.host.listening()).toEqual([])
    expect(pluginConns(h)).toEqual([])
  })

  it('one start for an event and a command that come together', async () => {
    h = harness({ list: listed({}, onEvent()) })
    await h.host.sync()
    await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]))
    emitAndPump(h, 'document_changed', 1)
    const reply = call(h, `${ID}.greet`)
    expect((await reply).result).toEqual({ greeting: 'Hello, Hew!' })
    expect(h.realms).toHaveLength(1)
    expect(pluginConns(h)).toHaveLength(1)
  })

  it('a command start also closes the listener, subscribing the realm to its events', async () => {
    const seen: string[] = []
    h = harness({ list: listed({}, onEvent()), main: recording(seen) })
    await h.host.sync()
    await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]))
    await call(h, `${ID}.greet`)
    expect(h.host.listening()).toEqual([])
    expect(pluginConns(h)).toHaveLength(1)
    emitAndPump(h, 'document_changed', 5)
    await vi.waitFor(() => expect(seen).toEqual(['document_changed:5']))
  })

  it('listens again after an idle unload, and leaves at least the least time between two event starts', async () => {
    const seen: string[] = []
    h = harness({ list: listed({}, onEvent()), main: recording(seen), timings: { pingMs: 10, idleMs: 40, drainMs: 100, eventStartMs: 400 } })
    await h.host.sync()
    await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]))
    const first = Date.now()
    emitAndPump(h, 'document_changed', 1)
    // It starts, and (quiet) unloads again within a few pings.
    await vi.waitFor(() => expect(sent(h, 0, 'unload')).toEqual([{ t: 'unload', reason: 'idle' }]))
    await vi.waitFor(() => expect(h.host.states()[ID]).toBe('enabled'))
    await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]))
    expect(pluginConns(h)).toHaveLength(1)
    emitAndPump(h, 'document_changed', 2)
    await flush(20)
    expect(h.realms).toHaveLength(1)
    await vi.waitFor(() => expect(h.realms).toHaveLength(2), { timeout: 2_000 })
    expect(Date.now() - first).toBeGreaterThanOrEqual(390)
    expect((h.realms[1].toRealm[0] as { activation: string }).activation).toBe('event')
    await vi.waitFor(() => expect(eventsSent(h, 1)).toEqual(['document_changed:2']))
  })

  it('waits twice as long after a start that failed', async () => {
    const reads: number[] = []
    let fail = true
    h = harness({
      list: listed({}, onEvent()),
      timings: { eventStartMs: 100, eventStartMaxMs: 1_000, startMs: 30 },
      readFile: async (id) => {
        reads.push(Date.now())
        // The document is held by a command that is not this plugin's, for
        // longer than the start may wait: the start fails, not as a crash.
        if (fail) h.scene.compounds.push({ kernel: 1n, forward: 999 })
        return new TextEncoder().encode(id).buffer as ArrayBuffer
      },
    })
    await h.host.sync()
    await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]))
    emitAndPump(h, 'document_changed', 1)
    await vi.waitFor(() => expect(h.logs.some((l) => l.includes('did not start'))).toBe(true))
    expect(h.host.states()[ID]).toBe('enabled')
    fail = false
    h.scene.compounds.splice(0)
    setBusy(null)
    await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]), { timeout: 2_000 })
    emitAndPump(h, 'document_changed', 2)
    await vi.waitFor(() => expect(reads).toHaveLength(2), { timeout: 2_000 })
    expect(reads[1] - reads[0]).toBeGreaterThanOrEqual(190)
    await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
  })

  it('closes the listener when the plugin is turned off, stopped, blocked, uninstalled, or the window closes', async () => {
    const cases: [string, (h: Harness) => Promise<void>][] = [
      ['off', async (x) => { x.list = listed({ enabled: false }, onEvent()); await x.host.sync() }],
      ['uninstalled', async (x) => { x.list = []; await x.host.sync() }],
      ['blocked', async (x) => { x.list = listed({ needs_consent: true }, onEvent()); await x.host.sync() }],
      ['window closed', async (x) => x.host.dispose()],
    ]
    for (const [name, end] of cases) {
      h = harness({ list: listed({}, onEvent()) })
      await h.host.sync()
      await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]))
      await end(h)
      await flush(40)
      expect(pluginConns(h), name).toEqual([])
      expect(h.host.listening(), name).toEqual([])
      h.host.dispose()
    }
    // Stopped: the plugin crashed, and no listener comes back for it.
    h = harness({ list: listed({}, onEvent()) })
    await h.host.sync()
    await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]))
    emitAndPump(h, 'document_changed', 1)
    await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
    h.realms[0].send({ t: 'error', message: 'boom' })
    await flush(40)
    expect(h.host.states()[ID]).toBe('crashed')
    expect(pluginConns(h)).toEqual([])
    expect(h.host.listening()).toEqual([])
  })

  it('a grant widened while the listener opens applies once it is ready: it listens for what the wider grant receives', async () => {
    const events = onEvent(['document_changed', 'selection_changed'])
    h = harness({ list: listed({ optional_granted: [] }, events), timings: { startMs: 30 } })
    // The document is held: the listener's opening calls wait.
    h.scene.compounds.push({ kernel: 1n, forward: 999 })
    await h.host.sync()
    await flush(20)
    expect(h.host.listening()).toEqual([])
    h.list = listed({ optional_granted: ['view'] }, events)
    await h.host.sync()
    h.scene.compounds.splice(0)
    await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]))
    const [conn] = pluginConns(h)
    await vi.waitFor(() => expect(h.scene.conns.get(conn)?.subscriptions.has('selection_changed')).toBe(true))
    expect(h.scene.conns.get(conn)?.permissions).toEqual(['model.read', 'view'])
    emitAndPump(h, 'selection_changed', 1)
    await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
  })

  it('a listener whose opening grant receives none of its events, widened while it opened, listens instead of closing', async () => {
    const events = onEvent(['selection_changed'])
    h = harness({ list: listed({ optional_granted: [] }, events), timings: { startMs: 30 } })
    h.scene.compounds.push({ kernel: 1n, forward: 999 })
    await h.host.sync()
    await flush(20)
    h.list = listed({ optional_granted: ['view'] }, events)
    await h.host.sync()
    h.scene.compounds.splice(0)
    await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]))
    expect(h.scene.conns.get(pluginConns(h)[0])?.subscriptions.has('selection_changed')).toBe(true)
  })

  it('a start that takes over from a listener whose widened subscribe is still waiting subscribes the realm to what the wider grant receives', async () => {
    const events = onEvent(['document_changed', 'selection_changed'])
    h = harness({ list: listed({ optional_granted: [] }, events), timings: { startMs: 30 } })
    await h.host.sync()
    await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]))
    const [listenerConn] = pluginConns(h)
    // The listener's widened subscribe is answered busy until let go.
    let holdWiden = true
    const real = h.scene.api_dispatch.getMockImplementation()!
    h.scene.api_dispatch.mockImplementation((conn: number, frame: string) => {
      const m = JSON.parse(frame) as { id?: unknown; method?: string }
      if (holdWiden && conn === listenerConn && m.method === 'hew.event.subscribe')
        return JSON.stringify({ jsonrpc: '2.0', id: m.id, error: { code: -32000, message: 'busy', data: { refusal: 'document_busy' } } })
      return real(conn, frame)
    })
    h.list = listed({ optional_granted: ['view'] }, events)
    await h.host.sync()
    await flush(20)
    expect(h.scene.conns.get(listenerConn)?.subscriptions.has('selection_changed')).toBe(false)
    emitAndPump(h, 'document_changed', 1)
    await vi.waitFor(() => expect(h.host.states()[ID]).toBe('activated'))
    holdWiden = false
    await new Promise((r) => setTimeout(r, 80))
    const [realmConn] = pluginConns(h)
    expect(realmConn).not.toBe(listenerConn)
    expect(h.scene.conns.get(realmConn)?.subscriptions.has('selection_changed')).toBe(true)
  })

  /** Answers `document_busy` to the listener's calls of `method` while
   *  `hold()` says so. The listener's connection is the plugin's only one
   *  when the first call is made. */
  function holdListener(method: string, hold: () => boolean): { conn: () => number | null } {
    let listenerConn: number | null = null
    const real = h.scene.api_dispatch.getMockImplementation()!
    h.scene.api_dispatch.mockImplementation((conn: number, frame: string) => {
      const m = JSON.parse(frame) as { id?: unknown; method?: string }
      if (listenerConn === null) listenerConn = pluginConns(h)[0] ?? null
      if (hold() && conn === listenerConn && m.method === method)
        return JSON.stringify({ jsonrpc: '2.0', id: m.id, error: { code: -32000, message: 'busy', data: { refusal: 'document_busy' } } })
      return real(conn, frame)
    })
    return { conn: () => listenerConn }
  }

  /** The `hew.event.subscribe` calls made on `conn`, by their events. */
  const subscribes = (conn: number): string[][] =>
    h.scene.api_dispatch.mock.calls
      .filter(([c, f]) => c === conn && (JSON.parse(f) as { method: string }).method === 'hew.event.subscribe')
      .map(([, f]) => (JSON.parse(f) as { params: { events: string[] } }).params.events)

  it('a start that takes over from a listener still opening subscribes the realm to its starting events, and each event comes once', async () => {
    h = harness({ list: listed({}, onEvent()), timings: { startMs: 30 } })
    let holding = true
    const held = holdListener('hew.meta.hello', () => holding)
    await h.host.sync()
    await flush(20)
    const listenerConn = held.conn()
    expect(listenerConn).not.toBeNull()
    expect(h.host.listening()).toEqual([])
    await call(h, `${ID}.greet`)
    holding = false
    await new Promise((r) => setTimeout(r, 80))
    const conns = pluginConns(h)
    expect(conns).toHaveLength(1)
    const [realmConn] = conns
    expect(realmConn).not.toBe(listenerConn)
    expect(subscribes(realmConn)).toEqual([['document_changed']])
    expect(h.scene.conns.get(realmConn)?.subscriptions.has('document_changed')).toBe(true)
    expect(h.host.listening()).toEqual([])
    emitAndPump(h, 'document_changed', 7)
    await vi.waitFor(() => expect(eventsSent(h, 0)).toEqual(['document_changed:7']))
    await flush(40)
    expect(eventsSent(h, 0)).toEqual(['document_changed:7'])
  })

  it('a start that takes over from a listener that receives none yet, its widened subscribe waiting, subscribes the realm to every starting event', async () => {
    const events = onEvent(['selection_changed'])
    h = harness({ list: listed({ optional_granted: [] }, events), timings: { startMs: 30 } })
    // The opening grant receives none: the listener is ready with no names
    // once the widened grant arrives while it opens, and its widened
    // subscribe is answered busy until let go.
    h.scene.compounds.push({ kernel: 1n, forward: 999 })
    let holding = false
    const held = holdListener('hew.event.subscribe', () => holding)
    await h.host.sync()
    await flush(20)
    h.list = listed({ optional_granted: ['view'] }, events)
    holding = true
    await h.host.sync()
    h.scene.compounds.splice(0)
    await flush(40)
    const listenerConn = held.conn()!
    expect(h.scene.conns.get(listenerConn)?.subscriptions.has('selection_changed') ?? false).toBe(false)
    await call(h, `${ID}.greet`)
    holding = false
    await new Promise((r) => setTimeout(r, 80))
    const [realmConn] = pluginConns(h)
    expect(pluginConns(h)).toHaveLength(1)
    expect(realmConn).not.toBe(listenerConn)
    expect(subscribes(realmConn)).toEqual([['selection_changed']])
    expect(h.scene.conns.get(realmConn)?.subscriptions.has('selection_changed')).toBe(true)
    emitAndPump(h, 'selection_changed', 3)
    await vi.waitFor(() => expect(eventsSent(h, 0)).toEqual(['selection_changed:3']))
    await flush(40)
    expect(eventsSent(h, 0)).toEqual(['selection_changed:3'])
  })

  it('drops the events held for a start that never came when the listener closes: a later start does not get them', async () => {
    const seen: string[] = []
    h = harness({ list: listed({}, onEvent()), main: recording(seen), timings: { pingMs: 10, idleMs: 40, drainMs: 100, eventStartMs: 60_000 } })
    await h.host.sync()
    await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]))
    emitAndPump(h, 'document_changed', 1)
    await vi.waitFor(() => expect(sent(h, 0, 'unload')).toHaveLength(1))
    await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]))
    // Within the least time between two event starts: held, the start waits.
    emitAndPump(h, 'document_changed', 2)
    await flush(20)
    expect(h.realms).toHaveLength(1)
    // Blocked, then let run again: the waiting start is gone, and so is
    // what it held.
    h.list = listed({ needs_consent: true }, onEvent())
    await h.host.sync()
    h.list = listed({}, onEvent())
    await h.host.sync()
    await call(h, `${ID}.greet`)
    await flush(40)
    expect(h.realms).toHaveLength(2)
    expect(eventsSent(h, 1)).toEqual([])
  })

  it('on the desktop, waits for the document id with one wait per plugin however often it syncs, and a teardown ends it', async () => {
    vi.useFakeTimers()
    setPumpDocumentId(null)
    try {
      h = harness({ platform: 'desktop', list: listed({}, onEvent()) })
      for (let i = 0; i < 5; i++) await h.host.sync()
      expect(vi.getTimerCount()).toBe(1)
      await vi.advanceTimersByTimeAsync(1_000)
      for (let i = 0; i < 3; i++) await h.host.sync()
      expect(vi.getTimerCount()).toBe(1)
      // Turned off: the wait ends at its next tick, and opens nothing.
      h.list = listed({ enabled: false }, onEvent())
      await h.host.sync()
      setPumpDocumentId('doc_7')
      await vi.advanceTimersByTimeAsync(300)
      expect(vi.getTimerCount()).toBe(0)
      expect(pluginConns(h)).toEqual([])
      // On again, with an id: one listener.
      h.list = listed({}, onEvent())
      await h.host.sync()
      await vi.advanceTimersByTimeAsync(10)
      expect(h.host.listening()).toEqual([ID])
      expect(pluginConns(h)).toHaveLength(1)
    } finally {
      setPumpDocumentId(null)
      vi.useRealTimers()
    }
  })

  it('on Shop, builds every grant as a viewer one', async () => {
    h = harness({ platform: 'shop', list: listed({}, onEvent()) })
    await h.host.sync()
    await vi.waitFor(() => expect(h.host.listening()).toEqual([ID]))
    await call(h, `${ID}.greet`)
    expect(h.viewerCalls.length).toBeGreaterThanOrEqual(2)
    expect(h.viewerCalls.every((v) => v)).toBe(true)
    expect(h.scene.conns.get(pluginConns(h)[0])?.permissions).toEqual(['model.read'])
  })
})

// ------------------------------------------------- per-document storage

const D1 = '11111111-1111-4111-8111-111111111111'
const D2 = '22222222-2222-4222-8222-222222222222'

describe('the plugin host, storage for the document', () => {
  let h: Harness
  beforeEach((ctx) => {
    requiresPreview(ctx, 'plugins')
    setBusy(null)
  })
  afterEach(() => h?.host.dispose())

  /** A harness whose plugin hands its `hew` out, started. */
  async function started(opts: Partial<HarnessOpts> = {}): Promise<{ hew: Hew; docEvents: (string | null)[] }> {
    let hew!: Hew
    const docEvents: (string | null)[] = []
    h = harness({
      main: (api) => {
        hew = api
        api.commands.handle('greet', () => 'hi')
        api.lifecycle.on('document', (id) => docEvents.push(id))
      },
      ...opts,
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    return { hew, docEvents }
  }
  const lifecycle = async (kind: 'opened' | 'saved' | 'closed', id: string | null): Promise<void> => {
    if (kind !== 'closed') h.scene.docId = id ?? undefined
    h.scene.lifecycle(kind, id)
    pumpEvents(h.scene as unknown as Scene)
    await flush(60)
  }

  it('keeps an unsaved document in memory and writes it under the id the first save gives, before the plugin is told', async () => {
    const { hew, docEvents } = await started()
    expect(hew.plugin?.documentId).toBeNull()
    expect((h.realms[0].toRealm[0] as { document_id: unknown }).document_id).toBeNull()
    await hew.storage.document.set('a', 1)
    await hew.storage.document.set('b', { x: 2 })
    await expect(hew.storage.document.get('a')).resolves.toBe(1)
    await expect(hew.storage.document.list()).resolves.toEqual(['a', 'b'])
    expect(h.storage.docSet).not.toHaveBeenCalled()
    // What a key under the new id already had is overwritten.
    h.docs.set(`${ID}/${D1}`, new Map([['a', '"old"'], ['c', '3']]))
    let atTold: string[] = []
    hew.lifecycle.on('document', () => (atTold = [...(h.docs.get(`${ID}/${D1}`)?.keys() ?? [])].sort()))
    await lifecycle('saved', D1)
    await vi.waitFor(() => expect(docEvents).toEqual([D1]))
    expect(atTold).toEqual(['a', 'b', 'c'])
    expect(h.docs.get(`${ID}/${D1}`)).toEqual(new Map([['a', '1'], ['b', '{"x":2}'], ['c', '3']]))
    expect(hew.plugin?.documentId).toBe(D1)
    await expect(hew.storage.document.list()).resolves.toEqual(['a', 'b', 'c'])
    // The plugin's own storage never saw any of it.
    expect(h.stored.size).toBe(0)
  })

  it("follows the document: another document's data is not visible, and comes back when it is reopened", async () => {
    const { hew } = await started({ documentId: D1 })
    expect(hew.plugin?.documentId).toBe(D1)
    await hew.storage.document.set('k', 'one')
    await lifecycle('opened', D2)
    expect(hew.plugin?.documentId).toBe(D2)
    await expect(hew.storage.document.get('k')).resolves.toBeUndefined()
    await hew.storage.document.set('k', 'two')
    await lifecycle('opened', D1)
    await expect(hew.storage.document.get('k')).resolves.toBe('one')
    await lifecycle('opened', D2)
    await expect(hew.storage.document.get('k')).resolves.toBe('two')
  })

  it('lands a write that is on its way when the document changes under the old id', async () => {
    const { hew } = await started({ documentId: D1, timings: { storageDebounceMs: 200 } })
    const write = hew.storage.document.set('k', 'mine')
    await flush(20)
    await lifecycle('opened', D2)
    await write
    expect(h.docs.get(`${ID}/${D1}`)?.get('k')).toBe('"mine"')
    expect(h.docs.get(`${ID}/${D2}`)?.get('k')).toBeUndefined()
  })

  it('refuses document_changed, and does nothing with, a call made before another document opened and handled after', async () => {
    const s = holdingSched()
    const { hew } = await started({ documentId: D1, sched: s.sched })
    expect((h.realms[0].toRealm[0] as { document_epoch: unknown }).document_epoch).toBe(0)
    s.hold()
    const write = hew.storage.document.set('k', 'for D1')
    const read = hew.storage.document.get('k')
    const listing = hew.storage.document.list()
    await flush(20)
    await lifecycle('opened', D2)
    expect(sent(h, 0, 'document').at(-1)).toEqual({ t: 'document', document_id: D2, epoch: 1 })
    s.release()
    await expect(write).rejects.toMatchObject({ name: 'document_changed' })
    await expect(read).rejects.toMatchObject({ name: 'document_changed' })
    await expect(listing).rejects.toMatchObject({ name: 'document_changed' })
    await flush(40)
    expect(h.docs.get(`${ID}/${D2}`)?.get('k')).toBeUndefined()
    expect(h.docs.get(`${ID}/${D1}`)?.get('k')).toBeUndefined()
    expect(h.host.states()[ID]).toBe('activated')
    // A call made now is for the new document, and lands there.
    await hew.storage.document.set('k', 'for D2')
    expect(h.docs.get(`${ID}/${D2}`)?.get('k')).toBe('"for D2"')
    // The plugin's own storage knows no documents: never refused so.
    await hew.storage.set('mine', 1)
    expect(h.stored.get('mine')).toBe('1')
  })

  it('keeps the epoch at the first save: a call made before the save gave the document its id lands under that id', async () => {
    const s = holdingSched()
    const { hew } = await started({ sched: s.sched })
    s.hold()
    const write = hew.storage.document.set('k', 'mine')
    await flush(20)
    await lifecycle('saved', D1)
    s.release()
    await write
    await vi.waitFor(() => expect(h.docs.get(`${ID}/${D1}`)?.get('k')).toBe('"mine"'))
    expect(sent(h, 0, 'document').at(-1)).toEqual({ t: 'document', document_id: D1, epoch: 0 })
    // A later save under another id is another document for storage.
    await lifecycle('saved', D2)
    expect(sent(h, 0, 'document').at(-1)).toEqual({ t: 'document', document_id: D2, epoch: 1 })
  })

  it('a document closed with none opened yet (a .skp import) binds storage to no document: writes on their way land under the old id, later calls never reach it', async () => {
    const s = holdingSched()
    const { hew, docEvents } = await started({ documentId: D1, sched: s.sched, timings: { storageDebounceMs: 200 } })
    await hew.storage.document.set('k', 'one')
    const write = hew.storage.document.set('w', 'on its way')
    await flush(20)
    s.hold()
    const stale = hew.storage.document.get('k')
    await flush(20)
    await lifecycle('closed', null)
    await write
    expect(h.docs.get(`${ID}/${D1}`)?.get('w')).toBe('"on its way"')
    expect(sent(h, 0, 'document').at(-1)).toEqual({ t: 'document', document_id: null, epoch: 1 })
    s.release()
    // Made before the close, handled after: refused, nothing read.
    await expect(stale).rejects.toMatchObject({ name: 'document_changed' })
    await vi.waitFor(() => expect(docEvents).toEqual([null]))
    expect(hew.plugin?.documentId).toBeNull()
    // Made after: the closed document's data is not seen and not written.
    await expect(hew.storage.document.get('k')).resolves.toBeUndefined()
    await hew.storage.document.set('k', 'importing')
    await flush(40)
    expect(h.docs.get(`${ID}/${D1}`)?.get('k')).toBe('"one"')
    // The import ends: the throwaway session is gone with it.
    await lifecycle('opened', D2)
    expect(sent(h, 0, 'document').at(-1)).toEqual({ t: 'document', document_id: D2, epoch: 2 })
    await expect(hew.storage.document.get('k')).resolves.toBeUndefined()
    expect(h.docs.get(`${ID}/${D2}`)?.get('k')).toBeUndefined()
  })

  it('a save after a close with none opened never carries what the throwaway session held to the saved id', async () => {
    const { hew, docEvents } = await started({ documentId: D1 })
    await hew.storage.document.set('k', 'one')
    await lifecycle('closed', null)
    await vi.waitFor(() => expect(docEvents).toEqual([null]))
    // Written in the gap: held by the throwaway session only.
    await hew.storage.document.set('gap', 'importing')
    await expect(hew.storage.document.get('gap')).resolves.toBe('importing')
    await lifecycle('saved', D2)
    await vi.waitFor(() => expect(docEvents).toEqual([null, D2]))
    // Another document for storage: the epoch moves on.
    expect(sent(h, 0, 'document').at(-1)).toEqual({ t: 'document', document_id: D2, epoch: 2 })
    await flush(40)
    expect(h.docs.get(`${ID}/${D2}`)?.has('gap') ?? false).toBe(false)
    await expect(hew.storage.document.list()).resolves.toEqual([])
    expect(h.docs.get(`${ID}/${D1}`)?.has('gap') ?? false).toBe(false)
    expect(h.docs.get(`${ID}/${D1}`)?.get('k')).toBe('"one"')
  })

  it('the first save of a new document opened after a close still carries its data to the id the save gives', async () => {
    const { hew } = await started({ documentId: D1 })
    await lifecycle('closed', null)
    await hew.storage.document.set('gap', 'dropped')
    // A new, never-saved document: not the throwaway session.
    await lifecycle('opened', null)
    await hew.storage.document.set('k', 'new')
    await lifecycle('saved', D2)
    await vi.waitFor(() => expect(h.docs.get(`${ID}/${D2}`)?.get('k')).toBe('"new"'))
    expect(h.docs.get(`${ID}/${D2}`)?.has('gap')).toBe(false)
    // The first save keeps the epoch the open set.
    expect(sent(h, 0, 'document').at(-1)).toEqual({ t: 'document', document_id: D2, epoch: 2 })
  })

  it('an ordinary open (closed, then opened, in one drain) tells the realm once, of the new document', async () => {
    const { hew, docEvents } = await started({ documentId: D1 })
    await hew.storage.document.set('k', 'one')
    const told = sent(h, 0, 'document').length
    h.scene.docId = D2
    h.scene.lifecycle('closed', null)
    h.scene.lifecycle('opened', D2)
    pumpEvents(h.scene as unknown as Scene)
    await flush(60)
    expect(sent(h, 0, 'document').slice(told)).toEqual([{ t: 'document', document_id: D2, epoch: 2 }])
    expect(docEvents).toEqual([D2])
    await expect(hew.storage.document.get('k')).resolves.toBeUndefined()
    await hew.storage.document.set('k', 'two')
    expect(h.docs.get(`${ID}/${D2}`)?.get('k')).toBe('"two"')
    expect(h.docs.get(`${ID}/${D1}`)?.get('k')).toBe('"one"')
  })

  it('a document opened with no id (a format with no place for one) starts empty', async () => {
    const { hew } = await started()
    await hew.storage.document.set('k', 1)
    await lifecycle('opened', null)
    await expect(hew.storage.document.list()).resolves.toEqual([])
    await expect(hew.storage.document.get('k')).resolves.toBeUndefined()
  })

  it('refuses an unsaved document past 5 MiB storage_quota', async () => {
    const { hew } = await started()
    const big = 'x'.repeat(999_000)
    for (let i = 0; i < 5; i++) await hew.storage.document.set(`k${i}`, big)
    await expect(hew.storage.document.set('k5', big)).rejects.toMatchObject({ name: 'storage_quota' })
    await hew.storage.document.delete('k0')
    await expect(hew.storage.document.set('k5', big)).resolves.toBeUndefined()
  })

  it('keeps carried data that could not be written, reads it, and writes it with the next write', async () => {
    let failing = true
    const { hew } = await started({ docSetFails: () => failing })
    await hew.storage.document.set('carried', 1)
    await lifecycle('saved', D1)
    await vi.waitFor(() => expect(h.logs.some((l) => l.includes('tried again'))).toBe(true))
    expect(h.docs.get(`${ID}/${D1}`)?.get('carried')).toBeUndefined()
    await expect(hew.storage.document.get('carried')).resolves.toBe(1)
    await expect(hew.storage.document.list()).resolves.toEqual(['carried'])
    failing = false
    await hew.storage.document.set('next', 2)
    expect(h.docs.get(`${ID}/${D1}`)).toEqual(new Map([['carried', '1'], ['next', '2']]))
  })

  it('lists document keys in code-point order before the first save, after it, and while carried data waits', async () => {
    // U+FF01 sorts before U+1F600 by code point; UTF-16 order puts the
    // emoji's surrogates (U+D83D) first.
    const order = ['\uFF01', '\u{1F600}']
    let failing = true
    const { hew } = await started({ docSetFails: () => failing })
    await hew.storage.document.set('\u{1F600}', 1)
    await hew.storage.document.set('\uFF01', 2)
    await expect(hew.storage.document.list()).resolves.toEqual(order)
    await lifecycle('saved', D1)
    await vi.waitFor(() => expect(h.logs.some((l) => l.includes('tried again'))).toBe(true))
    // Carried, not yet written: the list merges it in, in the same order.
    await expect(hew.storage.document.list()).resolves.toEqual(order)
    failing = false
    await hew.storage.document.set('\uFF01', 3)
    expect(h.docs.get(`${ID}/${D1}`)?.size).toBe(2)
    await expect(hew.storage.document.list()).resolves.toEqual(order)
  })

  it('keeps two windows of one plugin in two documents apart, and hands a notice only to the window with that document', async () => {
    const docs = new Map<string, Map<string, string>>()
    const a = await started({ documentId: D1, docs })
    const first = h
    const b = await started({ documentId: D2, docs })
    const second = h
    try {
      await a.hew.storage.document.set('k', 'a')
      await b.hew.storage.document.set('k', 'b')
      await expect(a.hew.storage.document.get('k')).resolves.toBe('a')
      await expect(b.hew.storage.document.get('k')).resolves.toBe('b')
      const seenA: string[][] = []
      const seenB: string[][] = []
      const pluginA: string[][] = []
      a.hew.storage.document.onChange((k) => seenA.push(k))
      b.hew.storage.document.onChange((k) => seenB.push(k))
      a.hew.storage.onChange((k) => pluginA.push(k))
      for (const x of [first, second]) x.storageChanged(ID, ['k'], 'document', D1)
      await flush(20)
      expect(seenA).toEqual([['k']])
      expect(seenB).toEqual([])
      // A document notice is never taken for the plugin's own storage.
      expect(pluginA).toEqual([])
    } finally {
      first.host.dispose()
    }
  })

  it('refuses a value with a lone surrogate storage_value_invalid in either scope, before any store sees it, and stops nothing', async () => {
    const { hew } = await started({ documentId: D1 })
    void hew
    const reply = (id: number): unknown => h.realms[0].toRealm.find((m) => m.t === 'storage_reply' && m.id === id)
    h.realms[0].send({ t: 'storage', id: 901, op: 'set', scope: 'plugin', key: 'p', value: '"\uD800"' })
    h.realms[0].send({ t: 'storage', id: 902, op: 'set', scope: 'document', epoch: 0, key: 'd', value: '"\uDC00x"' })
    await vi.waitFor(() => expect(reply(902)).toBeDefined())
    expect(reply(901)).toEqual({ t: 'storage_reply', id: 901, scope: 'plugin', ok: false, error: 'storage_value_invalid' })
    expect(reply(902)).toEqual({ t: 'storage_reply', id: 902, scope: 'document', ok: false, error: 'storage_value_invalid' })
    await flush(40)
    expect(h.storage.set).not.toHaveBeenCalled()
    expect(h.storage.docSet).not.toHaveBeenCalled()
    expect(h.stored.has('p')).toBe(false)
    expect(h.docs.get(`${ID}/${D1}`)?.has('d') ?? false).toBe(false)
    expect(h.host.states()[ID]).toBe('activated')
    // A well-formed pair is a value like any other.
    await hew.storage.set('pair', '\uD83D\uDE00')
    expect(h.stored.get('pair')).toBe('"\uD83D\uDE00"')
  })

  it('the validator takes a storage message with no scope as the plugin\'s own', async () => {
    const { hew } = await started({ documentId: D1 })
    void hew
    h.realms[0].send({ t: 'storage', id: 999, op: 'set', key: 'raw', value: '1' })
    await vi.waitFor(() => expect(h.stored.get('raw')).toBe('1'))
    expect(h.docs.get(`${ID}/${D1}`)?.get('raw')).toBeUndefined()
    h.realms[0].send({ t: 'storage', id: 1000, op: 'get', scope: 'elsewhere', key: 'raw' })
    await flush(20)
    expect(h.host.states()[ID]).toBe('crashed')
  })
})

// ------------------------------------------------- what the host reports

describe('the plugin host, reporting states and console', () => {
  let h: Harness
  beforeEach((ctx) => {
    requiresPreview(ctx, 'plugins')
    setBusy(null)
  })
  afterEach(() => h?.host.dispose())

  it('reports each state change once, with why', async () => {
    const reports: [string, string, string | null][] = []
    h = harness({ report: (id, state, reason) => reports.push([id, state, reason]), timings: { pingMs: 10, idleMs: 40, drainMs: 100 } })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    await vi.waitFor(() => expect(reports.map(([, s]) => s)).toContain('idle'))
    await call(h, `${ID}.greet`)
    h.realms[1].send({ t: 'error', message: 'boom' })
    await flush(20)
    const states = reports.map(([, s]) => s)
    expect(states.slice(0, 4)).toEqual(['enabled', 'activating', 'activated', 'idle'])
    expect(states).toContain('crashed')
    expect(reports.find(([, s]) => s === 'idle')![2]).toBe('idle')
    // The report carries the kind of stop as one word. What the plugin said
    // stays in the log and never rides along.
    expect(reports.find(([, s]) => s === 'crashed')![2]).toBe('error')
    // No two reports in a row are the same.
    for (let i = 1; i < reports.length; i++) expect(reports[i]).not.toEqual(reports[i - 1])
  })

  it('reports blocked with its reason', async () => {
    const reports: [string, string, string | null][] = []
    h = harness({ report: (id, state, reason) => reports.push([id, state, reason]), list: listed({ needs_consent: true }) })
    await h.host.sync()
    expect(reports.at(-1)).toEqual([ID, 'blocked', 'needs_review'])
  })

  it('hands console lines on in one batch a second, and the rest when the realm stops', async () => {
    const batches: { level: string; text: string }[][] = []
    h = harness({ console: (_id, lines) => batches.push(lines.map(({ level, text }) => ({ level, text }))) })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    for (let i = 0; i < 3; i++) h.realms[0].send({ t: 'console', level: 'info', args: [`line ${i}`], at: 0 })
    await flush(20)
    expect(batches).toEqual([])
    await vi.waitFor(() => expect(batches).toHaveLength(1), { timeout: 2_000 })
    expect(batches[0]).toEqual([0, 1, 2].map((i) => ({ level: 'info', text: `line ${i}` })))
    h.realms[0].send({ t: 'console', level: 'warn', args: ['last'], at: 0 })
    await flush(20)
    h.host.dispose()
    expect(batches.at(-1)).toEqual([{ level: 'warn', text: 'last' }])
  })

  it('a refused report of a stop is not taken as reported: it is tried once more after a while, and not again', async () => {
    const reports: string[] = []
    h = harness({
      timings: { reportRetryMs: 50 },
      report: (_id, state) => {
        reports.push(state)
        return state === 'crashed' ? Promise.reject(new PluginError('report_too_often', 'too often')) : Promise.resolve()
      },
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    h.realms[0].send({ t: 'error', message: 'boom' })
    await flush(20)
    const crashed = (): number => reports.filter((s) => s === 'crashed').length
    expect(crashed()).toBe(1)
    await vi.waitFor(() => expect(crashed()).toBe(2))
    await new Promise((r) => setTimeout(r, 200))
    expect(crashed()).toBe(2)
  })

  it('a report of a stop that the shell refuses and that is followed by another state is not tried again', async () => {
    const reports: string[] = []
    let refuse = true
    h = harness({
      timings: { reportRetryMs: 50 },
      list: listed({ needs_consent: true }),
      report: (_id, state) => {
        reports.push(state)
        return refuse && state === 'blocked' ? Promise.reject(new PluginError('report_too_often', 'too often')) : Promise.resolve()
      },
    })
    await h.host.sync()
    await flush(20)
    refuse = false
    h.list = listed()
    await h.host.sync()
    await new Promise((r) => setTimeout(r, 150))
    expect(reports.slice(-2)).toEqual(['blocked', 'enabled'])
    expect(reports.filter((s) => s === 'blocked')).toHaveLength(1)
  })

  it('a late refusal of an earlier report leaves the mark of a later accepted report of the same state', async () => {
    const reports: string[] = []
    let rejectFirst!: (e: unknown) => void
    h = harness({
      timings: { reportRetryMs: 30 },
      list: listed({ needs_consent: true }),
      report: (_id, state) => {
        reports.push(state)
        if (state === 'blocked' && rejectFirst === undefined) return new Promise((_r, reject) => (rejectFirst = reject))
        return Promise.resolve()
      },
    })
    await h.host.sync()
    h.list = listed()
    await h.host.sync()
    h.list = listed({ needs_consent: true })
    await h.host.sync()
    await flush(20)
    expect(reports).toEqual(['enabled', 'blocked', 'enabled', 'blocked'])
    rejectFirst(new PluginError('report_too_often', 'too often'))
    await new Promise((r) => setTimeout(r, 120))
    await h.host.sync()
    await flush(20)
    expect(reports).toEqual(['enabled', 'blocked', 'enabled', 'blocked'])
  })

  it('a report or a console hand-on that throws stops nothing', async () => {
    h = harness({
      report: () => {
        throw new Error('the shell is gone')
      },
      console: () => {
        throw new Error('the shell is gone')
      },
    })
    await h.host.sync()
    expect((await call(h, `${ID}.greet`)).result).toEqual({ greeting: 'Hello, Hew!' })
    h.realms[0].send({ t: 'console', level: 'info', args: ['x'], at: 0 })
    await new Promise((r) => setTimeout(r, 1_100))
    expect(h.host.states()[ID]).toBe('activated')
    expect((await call(h, `${ID}.greet`)).result).toEqual({ greeting: 'Hello, Hew!' })
  })
})

describe('the plugin host, a permission request and the realm that made it', () => {
  let h: Harness
  beforeEach((ctx) => {
    requiresPreview(ctx, 'plugins')
    setBusy(null)
  })
  afterEach(() => h?.host.dispose())

  it('a request left open by a realm that was reloaded does not keep the next realm waiting', async () => {
    let ask!: () => Promise<string>
    const decide = vi.fn(() => new Promise<'granted' | 'declined'>(() => {}))
    h = harness({
      list: listed({ optional_granted: [] }, { optionalPermissions: ['view'] }),
      grants: { decide },
      main: (hew) => {
        hew.commands.handle('greet', () => 'hi')
        ask = () =>
          hew.api.permissions.request({ permissions: ['view'] }).then(
            () => 'granted',
            (e: { refusal?: { refusal?: string } }) => e.refusal?.refusal ?? 'refused',
          )
      },
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    h.host.noteUserActivation(ID)
    void ask()
    await vi.waitFor(() => expect(decide).toHaveBeenCalledTimes(1))
    await h.host.reload(ID)
    await call(h, `${ID}.greet`)
    expect(h.realms).toHaveLength(2)
    h.host.noteUserActivation(ID)
    void ask()
    await vi.waitFor(() => expect(decide).toHaveBeenCalledTimes(2))
  })

  it('an act noted while the plugin does not run counts for nothing', async () => {
    let ask!: () => Promise<string>
    const decide = vi.fn(async () => 'granted' as const)
    h = harness({
      list: listed({ optional_granted: [] }, { optionalPermissions: ['view'] }),
      grants: { decide },
      main: (hew) => {
        hew.commands.handle('greet', () => 'hi')
        ask = () =>
          hew.api.permissions.request({ permissions: ['view'] }).then(
            () => 'granted',
            (e: { refusal?: { refusal?: string } }) => e.refusal?.refusal ?? 'refused',
          )
      },
    })
    await h.host.sync()
    h.host.noteUserActivation(ID)
    await call(h, `${ID}.greet`)
    await expect(ask()).resolves.toBe('no_user_activation')
    expect(decide).not.toHaveBeenCalled()
  })
})

describe('the plugin host, when the person cannot be asked', () => {
  let h: Harness
  beforeEach((ctx) => {
    requiresPreview(ctx, 'plugins')
    setBusy(null)
  })
  afterEach(() => h?.host.dispose())

  it('answers request_pending while another question is open, and request_declined, with no minute of refusals, when the asking fails otherwise', async () => {
    let refusal = 'confirmation_open'
    const decide = vi.fn(async () => {
      throw new PluginError(refusal, 'no')
    })
    let ask!: () => Promise<string>
    h = harness({
      list: listed({ optional_granted: [] }, { optionalPermissions: ['view'] }),
      grants: { decide },
      main: (hew) => {
        hew.commands.handle('greet', () => 'hi')
        ask = () =>
          hew.api.permissions.request({ permissions: ['view'] }).then(
            () => 'granted',
            (e: { refusal?: { refusal?: string } }) => e.refusal?.refusal ?? 'refused',
          )
      },
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    h.host.noteUserActivation(ID)
    await expect(ask()).resolves.toBe('request_pending')
    for (const name of ['plugin_changed', 'not_installed', 'unknown_permission', 'unsupported']) {
      refusal = name
      h.host.noteUserActivation(ID)
      await expect(ask(), name).resolves.toBe('request_declined')
    }
    // None of those was a no from the person: the decider was asked each time.
    expect(decide).toHaveBeenCalledTimes(5)
  })
})

describe("the plugin host, Shop's document sequence", () => {
  let h: Harness
  beforeEach((ctx) => {
    requiresPreview(ctx, 'plugins')
    setBusy(null)
  })
  afterEach(() => h?.host.dispose())

  it('takes an opened for another document, with no closed before it, as the end of the first: its writes land under the old id', async () => {
    let hew!: Hew
    h = harness({
      platform: 'shop',
      documentId: D1,
      timings: { storageDebounceMs: 200 },
      main: (api) => {
        hew = api
        api.commands.handle('greet', () => 'hi')
      },
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    const write = hew.storage.document.set('k', 'first')
    await flush(20)
    // What Shop notes when it loads the next file into the same Scene.
    h.scene.docId = D2
    h.scene.lifecycle('opened', D2)
    pumpEvents(h.scene as unknown as Scene)
    await write
    expect(h.docs.get(`${ID}/${D1}`)?.get('k')).toBe('"first"')
    expect(h.docs.get(`${ID}/${D2}`)?.get('k')).toBeUndefined()
    await vi.waitFor(() => expect(hew.plugin?.documentId).toBe(D2))
    await expect(hew.storage.document.get('k')).resolves.toBeUndefined()
  })

  it('a second file opened in place (closed and opened, each pumped on its own, in one task) tells the realm once, and lands what was on its way under the old id', async () => {
    let hew!: Hew
    h = harness({
      platform: 'shop',
      documentId: D1,
      timings: { storageDebounceMs: 200 },
      main: (api) => {
        hew = api
        api.commands.handle('greet', () => 'hi')
      },
    })
    await h.host.sync()
    await call(h, `${ID}.greet`)
    const write = hew.storage.document.set('k', 'first')
    await flush(20)
    const told = h.realms[0].toRealm.filter((m) => m.t === 'document').length
    // What Shop's noteOpened does when a model is already open.
    h.scene.lifecycle('closed', null)
    pumpEvents(h.scene as unknown as Scene)
    h.scene.docId = D2
    h.scene.lifecycle('opened', D2)
    pumpEvents(h.scene as unknown as Scene)
    await write
    await flush(40)
    expect(h.docs.get(`${ID}/${D1}`)?.get('k')).toBe('"first"')
    expect(h.realms[0].toRealm.filter((m) => m.t === 'document').slice(told)).toEqual([{ t: 'document', document_id: D2, epoch: 2 }])
  })
})
