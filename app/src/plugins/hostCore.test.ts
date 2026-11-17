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
import type { HostToRealm } from '../../../sdk/plugin/src/protocol'
import { getBusy, setBusy } from '../api/busyState'
import { pumpEvents, setPumpDocumentId } from '../api/eventPump'
import type { LiveBridgeDeps } from '../api/liveBridge'
import type { ForwardRequestJson } from '../api/dispatchResult'
import type { Scene } from '../wasm/loader'
import { createPluginHost, MAX_FORWARD_TIMEOUT_MS, MAX_PENDING_BUS, type HostScheduler, forwardTimeoutMs, packagePath, readForwardReply, type HostEnv, type PluginHost, type PortLike, type RealmChannel } from './hostCore'
import type { PluginStorageBackend } from './storageBackend'
import type { PluginStoreAccess } from './storeAccess'
import type { Installed, Listed, PluginManifest } from './types'
import { PluginError } from './pluginError'

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
  const conns = new Map<number, { plugin: string | null; permissions?: string[] }>()
  const registered = new Map<string, Decl[]>()
  const pending = new Map<string, { requestId: unknown; mutating: boolean; plugin: string; method: string }>()
  const compounds: { kernel: bigint; forward: number }[] = []
  const events: { conn: number; frame: string }[] = []
  let generation = 1
  const s = {
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
    drain_events: vi.fn(() => {
      const out = events.splice(0).map((e) => ({ conn: e.conn, frames: [e.frame] }))
      return JSON.stringify(out)
    }),
    api_connection_open: vi.fn(() => {
      const id = nextConn++
      conns.set(id, { plugin: null })
      return id
    }),
    api_plugin_connection_open: vi.fn((grant: string) => {
      const id = nextConn++
      const g = JSON.parse(grant) as { plugin: string; permissions: string[] }
      conns.set(id, { plugin: g.plugin, permissions: g.permissions })
      return id
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
      if (req.method === 'hew.event.subscribe') return reply({ result: { subscribed: (req.params as { events: string[] }).events, denied: [] } })
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
  storageChanged(id: string, keys: string[]): void
  stored: Map<string, string>
  storage: PluginStorageBackend
  toasts: string[]
  logs: string[]
  setMain(main: Main): void
  grantCalls: [string, string, string][]
  bridge: LiveBridgeDeps
}

const flush = async (n = 20): Promise<void> => {
  for (let i = 0; i < n; i++) await Promise.resolve()
}

interface HarnessOpts {
  list?: Listed[]
  main?: Main
  /** Each plugin's own background, by id; `main` for the rest. */
  mains?: Record<string, Main>
  timings?: Partial<Parameters<typeof createPluginHost>[1]>
  sched?: HostScheduler
  platform?: 'desktop' | 'web'
  /** Stands in for the store's read of a package file. */
  readFile?: (id: string, path: string) => Promise<ArrayBuffer>
  /** Runs before a realm frame answers. */
  beforeOpen?: () => Promise<void>
}

function harness(opts: HarnessOpts = {}): Harness {
  const scene = fakeScene()
  let main: Main = opts.main ?? ((hew) => hew.commands.handle('greet', (p) => ({ greeting: `Hello, ${(p as { name?: string }).name ?? 'Hew'}!` })))
  let onChanged: (() => void) | null = null
  let onStorage: ((id: string, keys: string[]) => void) | null = null
  const stored = new Map<string, string>()
  const toasts: string[] = []
  const logs: string[] = []
  const grantCalls: [string, string, string][] = []
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
  }
  const storage: PluginStorageBackend = {
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
    grantJson: (id, doc, names) => {
      grantCalls.push([id, doc, names])
      return JSON.stringify({ ceiling: 'core', permissions: JSON.parse(names), plugin: id, app: null, document: doc })
    },
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
    storageChanged: (id: string, keys: string[]) => onStorage?.(id, keys),
    stored,
    storage,
    toasts,
    logs,
    setMain: (m: Main) => (main = m),
    grantCalls,
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
