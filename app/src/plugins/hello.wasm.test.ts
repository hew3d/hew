// @vitest-environment jsdom
/**
 * The hello example end to end through the REAL compiled kernel: its stored
 * manifest is parsed by the kernel, its grant built by `plugin_grant_json`,
 * its commands registered in a real `Scene`; a first-party connection calls
 * `com.example.hello.greet`; the host serves the forward to the plugin's own
 * `dist/background.js`, run by the real SDK runtime; the plugin reads the
 * model over its own restricted connection and uses its storage.
 *
 * Only the frame and the Worker are stood in for (the browser specs run
 * those): the background script runs here with `hew` passed in.
 */

import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { requiresPreview } from '../test/requiresPreview'
import { previewBuild } from '../preview/build'
import { createRuntime, HewRefusal } from '../../../sdk/plugin/src/core'
import { getBusy, setBusy } from '../api/busyState'
import type { LiveBridgeDeps } from '../api/liveBridge'
import type { Scene } from '../wasm/loader'
import { createPluginHost, type HostEnv, type PluginHost, type PortLike } from './hostCore'
import type { PluginStorageBackend } from './storageBackend'
import type { Installed, PluginManifest } from './types'
import { FakeDir } from '../test/opfsFake'
import { pluginFile } from './testKit'
import { makeWebPluginStore, type PluginWasm } from './webPluginStore'

const ROOT = join(process.cwd(), '..')
const HELLO = join(ROOT, 'plugins/examples/hello')

type Kernel = typeof import('../wasm/pkg/wasm_api.js')
let kernel: Kernel

beforeAll(async () => {
  if (!previewBuild) return
  kernel = await import('../wasm/pkg/wasm_api.js')
  await kernel.default({ module_or_path: readFileSync(join(process.cwd(), 'src/wasm/pkg/wasm_api_bg.wasm')) })
})

function makeBox(scene: Scene): void {
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
  const c: Array<[number, number]> = [[0, 0], [1, 0], [1, 1], [0, 1]]
  for (let i = 0; i < 4; i++) scene.sketch_add_segment(sketch, c[i][0], c[i][1], 0, c[(i + 1) % 4][0], c[(i + 1) % 4][1], 0)
  scene.extrude_region(sketch, Array.from(scene.sketch_regions(sketch))[0], 1)
}

describe('the hello plugin, through the real kernel', () => {
  beforeEach((ctx) => requiresPreview(ctx, 'plugins'))

  it('answers greet with the object count, reading through its own grant, and counts in its storage', async () => {
    const manifest = JSON.parse(
      (kernel as unknown as { plugin_manifest_parse(t: string): string }).plugin_manifest_parse(readFileSync(join(HELLO, 'hew-plugin.json'), 'utf8')),
    ) as PluginManifest
    expect(manifest.id).toBe('com.example.hello')
    const main = readFileSync(join(HELLO, 'dist/background.js'), 'utf8')

    const scene = new kernel.Scene() as unknown as Scene
    makeBox(scene)
    const installed: Installed = {
      schema: 1,
      id: 'com.example.hello',
      version: '0.1.0',
      source: { kind: 'file', name: 'hello.hewplugin' },
      installed_at_ms: 0,
      grant: ['model.read'],
      optional_granted: [],
      enabled: true,
      startup_allowed: false,
      data_version: 0,
    }
    const stored = new Map<string, string>()
    const storage: PluginStorageBackend = {
      get: async (_id, key) => stored.get(key) ?? null,
      set: async (_id, key, value) => void stored.set(key, value),
      delete: async (_id, key) => void stored.delete(key),
      list: async () => [...stored.keys()],
      docGet: async () => null,
      docSet: async () => {},
      docDelete: async () => {},
      docList: async () => [],
    }
    const bridge: LiveBridgeDeps = { getScene: () => scene, getViewportApi: () => null, reconcile: vi.fn() }
    const env: HostEnv = {
      scene: () => scene,
      bridge,
      platform: 'web',
      store: {
        list: async () => [{ kind: 'installed', installed, manifest, previous_version: null }],
        readFile: async () => new TextEncoder().encode(main).buffer as ArrayBuffer,
        onChanged: () => () => {},
        onStorageChanged: () => () => {},
      },
      storage,
      grantJson: (id, doc, names) => (kernel as unknown as { plugin_grant_json(a: string, b: string, c: string, d: boolean): string }).plugin_grant_json(id, doc, names, false),
      async openRealm(code) {
        let hostPort: PortLike
        const rt = createRuntime({
          post: (m) => queueMicrotask(() => hostPort.onmessage?.({ data: structuredClone(m), ports: [] } as unknown as MessageEvent)),
          console: { log: () => {}, info: () => {}, warn: () => {}, error: () => {} },
          now: () => Date.now(),
        })
        // What the realm Worker runs after the runtime: the plugin's main.
        new Function('hew', 'HewRefusal', new TextDecoder().decode(code))(rt.hew, HewRefusal)
        hostPort = {
          onmessage: null,
          postMessage: (m: unknown) => {
            const copy = structuredClone(m)
            queueMicrotask(() => rt.receive(copy))
          },
          close: () => {},
        }
        return { control: { onmessage: null, postMessage: () => {}, close: () => {} }, port: hostPort, remove: () => {} }
      },
      toast: () => {},
      log: () => {},
      locale: () => 'en',
      theme: () => 'dark',
    }
    const host = createPluginHost(env, { pingMs: 60_000, startMs: 10_000, storageDebounceMs: 5, sliceMs: 8 })
    try {
      await host.sync()
      const conn = scene.api_connection_open()
      await host.dispatch(conn, '{"jsonrpc":"2.0","id":1,"method":"hew.meta.hello","params":{"protocol":1}}')
      await host.dispatch(conn, '{"jsonrpc":"2.0","id":2,"method":"hew.doc.attach","params":{}}')
      const greet = async (id: number) =>
        JSON.parse((await host.dispatch(conn, JSON.stringify({ jsonrpc: '2.0', id, method: 'com.example.hello.greet', params: { name: 'Kurt' } })))!) as {
          result?: unknown
          error?: unknown
        }
      expect(await greet(3)).toEqual({ jsonrpc: '2.0', id: 3, result: { greeting: 'Hello, Kurt!', objects: 1, times: 1 } })
      expect(await greet(4)).toMatchObject({ result: { times: 2 } })
      expect(stored.get('times')).toBe('2')
      expect(host.states()).toEqual({ 'com.example.hello': 'activated' })

      // A param the command does not declare is refused by the kernel before
      // the plugin sees it.
      const bad = JSON.parse(
        (await host.dispatch(conn, '{"jsonrpc":"2.0","id":5,"method":"com.example.hello.greet","params":{"evil":1}}'))!,
      ) as { error?: { code: number } }
      expect(bad.error?.code).toBe(-32602)
    } finally {
      host.dispose()
    }
  })

  it("refuses a write the plugin's grant does not hold", async () => {
    const scene = new kernel.Scene() as unknown as Scene
    const grant = (kernel as unknown as { plugin_grant_json(a: string, b: string, c: string, d: boolean): string }).plugin_grant_json(
      'com.example.hello',
      'doc_1',
      '["model.read"]',
      false,
    )
    const conn = scene.api_plugin_connection_open(grant)
    scene.api_dispatch(conn, '{"jsonrpc":"2.0","id":1,"method":"hew.meta.hello","params":{"protocol":1}}')
    scene.api_dispatch(conn, '{"jsonrpc":"2.0","id":2,"method":"hew.doc.attach","params":{}}')
    const reply = JSON.parse(
      JSON.parse(scene.api_dispatch(conn, '{"jsonrpc":"2.0","id":3,"method":"hew.tag.create","params":{"path":"x"}}')!).reply as string,
    ) as { error?: { code: number } }
    expect(reply.error?.code).toBe(-32001)
  })

  it('refuses the shell-answered plugin commands to a plugin: the kernel answers, and nothing goes further', async () => {
    // A plugin's realm talks only to the kernel of the page it runs in,
    // over a connection opened with its grant; the shell's router serves
    // only frames from the live API socket. So `hew.plugin.*` from a plugin
    // meets the kernel's permission check and stops there.
    const scene = new kernel.Scene() as unknown as Scene
    const grant = (kernel as unknown as { plugin_grant_json(a: string, b: string, c: string, d: boolean): string }).plugin_grant_json(
      'com.example.hello',
      'doc_1',
      '["model.read","model.write"]',
      false,
    )
    const conn = scene.api_plugin_connection_open(grant)
    scene.api_dispatch(conn, '{"jsonrpc":"2.0","id":1,"method":"hew.meta.hello","params":{"protocol":1}}')
    scene.api_dispatch(conn, '{"jsonrpc":"2.0","id":2,"method":"hew.doc.attach","params":{}}')
    for (const [id, method, params] of [
      [3, 'hew.plugin.dev', { path: '/home/me/plugin' }],
      [4, 'hew.plugin.list', {}],
    ] as const) {
      const out = JSON.parse(scene.api_dispatch(conn, JSON.stringify({ jsonrpc: '2.0', id, method, params }))!) as Record<string, unknown>
      // A reply from the kernel itself: no forward, no host request.
      expect(Object.keys(out)).toEqual(['reply'])
      const reply = JSON.parse(out.reply as string) as { error?: { code: number; data?: { refusal?: string } } }
      expect(reply.error?.code, method).toBe(-32001)
    }
  })

  describe('what the caller of a command that does not finish is told', () => {
    let host: PluginHost | null = null
    afterEach(() => {
      host?.dispose()
      host = null
      setBusy(null)
    })

    /** A plugin whose `wait` (mutating, 1 s) answers only `how` says. */
    async function waiting(how: 'never' | 'throw') {
      const id = 'com.example.waits'
      const manifest = JSON.parse(
        (kernel as unknown as { plugin_manifest_parse(t: string): string }).plugin_manifest_parse(
          JSON.stringify({
            manifest: 1,
            id,
            name: 'Waits',
            version: '0.1.0',
            description: 'Takes its time.',
            api: 1,
            main: 'dist/background.js',
            permissions: ['model.read', 'model.write'],
            activation: ['on_command'],
            contributes: { commands: [{ id: 'wait', class: 'mutating', summary: 'Waits.', timeout_ms: 1_000 }] },
          }),
        ),
      ) as PluginManifest
      const main =
        how === 'never'
          ? "hew.commands.handle('wait', () => new Promise(() => {}))"
          : "hew.commands.handle('wait', () => { throw new Error('on purpose') })"
      const scene = new kernel.Scene() as unknown as Scene
      const installed: Installed = {
        schema: 1,
        id,
        version: '0.1.0',
        source: { kind: 'file', name: 'waits.hewplugin' },
        installed_at_ms: 0,
        grant: ['model.read', 'model.write'],
        optional_granted: [],
        enabled: true,
        startup_allowed: false,
        data_version: 0,
      }
      const env: HostEnv = {
        scene: () => scene,
        bridge: { getScene: () => scene, getViewportApi: () => null, reconcile: vi.fn() },
        platform: 'web',
        store: {
          list: async () => [{ kind: 'installed', installed, manifest, previous_version: null }],
          readFile: async () => new TextEncoder().encode(main).buffer as ArrayBuffer,
          onChanged: () => () => {},
          onStorageChanged: () => () => {},
        },
        storage: {
          get: async () => null,
          set: async () => {},
          delete: async () => {},
          list: async () => [],
          docGet: async () => null,
          docSet: async () => {},
          docDelete: async () => {},
          docList: async () => [],
        },
        grantJson: (i, doc, names) => (kernel as unknown as { plugin_grant_json(a: string, b: string, c: string, d: boolean): string }).plugin_grant_json(i, doc, names, false),
        async openRealm(code) {
          let hostPort: PortLike
          const rt = createRuntime({
            post: (m) => queueMicrotask(() => hostPort.onmessage?.({ data: structuredClone(m), ports: [] } as unknown as MessageEvent)),
            console: { log: () => {}, info: () => {}, warn: () => {}, error: () => {} },
            now: () => Date.now(),
          })
          new Function('hew', 'HewRefusal', new TextDecoder().decode(code))(rt.hew, HewRefusal)
          hostPort = {
            onmessage: null,
            postMessage: (m: unknown) => {
              const copy = structuredClone(m)
              queueMicrotask(() => rt.receive(copy))
            },
            close: () => {},
          }
          return { control: { onmessage: null, postMessage: () => {}, close: () => {} }, port: hostPort, remove: () => {} }
        },
        toast: () => {},
        log: () => {},
        locale: () => 'en',
        theme: () => 'dark',
      }
      host = createPluginHost(env, { pingMs: 60_000, startMs: 10_000, storageDebounceMs: 5, sliceMs: 8 })
      await host.sync()
      const conn = scene.api_connection_open()
      await host.dispatch(conn, '{"jsonrpc":"2.0","id":1,"method":"hew.meta.hello","params":{"protocol":1}}')
      await host.dispatch(conn, '{"jsonrpc":"2.0","id":2,"method":"hew.doc.attach","params":{}}')
      const h = host
      const call = async () => JSON.parse((await h.dispatch(conn, `{"jsonrpc":"2.0","id":3,"method":"${id}.wait","params":{}}`))!) as Record<string, unknown>
      return { id, scene, call, method: `${id}.wait` }
    }

    it('cancelled: refused plugin_command_cancelled, and the document is free', async () => {
      const w = await waiting('never')
      const reply = w.call()
      await vi.waitFor(() => expect(getBusy()).not.toBeNull())
      getBusy()!.cancel()
      expect(await reply).toEqual({
        jsonrpc: '2.0',
        id: 3,
        error: {
          code: -32000,
          message: 'refused',
          data: { refusal: 'plugin_command_cancelled', failed_index: 0, failed_method: w.method, detail: {}, explanation: 'The user cancelled this command while it was running.' },
        },
      })
      expect(w.scene.busy()).toBe(false)
    })

    it('out of time: -32003 with reason timeout', async () => {
      const w = await waiting('never')
      expect(await w.call()).toEqual({
        jsonrpc: '2.0',
        id: 3,
        error: { code: -32003, message: 'the plugin did not answer in time', data: { plugin: w.id, method: w.method, reason: 'timeout' } },
      })
      expect(w.scene.busy()).toBe(false)
    })

    it('stopped: -32003 with reason crashed', async () => {
      const w = await waiting('throw')
      expect(await w.call()).toEqual({
        jsonrpc: '2.0',
        id: 3,
        error: { code: -32003, message: 'the plugin stopped while handling this command', data: { plugin: w.id, method: w.method, reason: 'crashed' } },
      })
      expect(w.scene.busy()).toBe(false)
    })
  })

  describe('a grant widened while the plugin runs, and a start on an event', () => {
    let host: PluginHost | null = null
    afterEach(() => {
      host?.dispose()
      host = null
      setBusy(null)
    })

    /** A host over a real Scene for one plugin, whose record `list` reads
     *  each time. */
    function realHost(man: Record<string, unknown>, main: string, record: { installed: Installed }, grants?: HostEnv['grants']) {
      const manifest = JSON.parse(
        (kernel as unknown as { plugin_manifest_parse(t: string): string }).plugin_manifest_parse(
          JSON.stringify({ manifest: 1, name: 'Real', version: '0.1.0', api: 1, main: 'dist/background.js', ...man }),
        ),
      ) as PluginManifest
      const scene = new kernel.Scene() as unknown as Scene
      const env: HostEnv = {
        scene: () => scene,
        bridge: { getScene: () => scene, getViewportApi: () => null, reconcile: vi.fn() },
        platform: 'web',
        store: {
          list: async () => [{ kind: 'installed', installed: record.installed, manifest, previous_version: null }],
          readFile: async () => new TextEncoder().encode(main).buffer as ArrayBuffer,
          onChanged: () => () => {},
          onStorageChanged: () => () => {},
        },
        storage: {
          get: async () => null,
          set: async () => {},
          delete: async () => {},
          list: async () => [],
          docGet: async () => null,
          docSet: async () => {},
          docDelete: async () => {},
          docList: async () => [],
        },
        grantJson: (i, doc, names, viewer) =>
          (kernel as unknown as { plugin_grant_json(a: string, b: string, c: string, d: boolean): string }).plugin_grant_json(i, doc, names, viewer),
        ...(grants === undefined ? {} : { grants }),
        async openRealm(code) {
          let hostPort: PortLike
          const rt = createRuntime({
            post: (m) => queueMicrotask(() => hostPort.onmessage?.({ data: structuredClone(m), ports: [] } as unknown as MessageEvent)),
            console: { log: () => {}, info: () => {}, warn: () => {}, error: () => {} },
            now: () => Date.now(),
          })
          new Function('hew', 'HewRefusal', new TextDecoder().decode(code))(rt.hew, HewRefusal)
          hostPort = {
            onmessage: null,
            postMessage: (m: unknown) => {
              const copy = structuredClone(m)
              queueMicrotask(() => rt.receive(copy))
            },
            close: () => {},
          }
          return { control: { onmessage: null, postMessage: () => {}, close: () => {} }, port: hostPort, remove: () => {} }
        },
        toast: () => {},
        log: () => {},
        locale: () => 'en',
        theme: () => 'dark',
      }
      host = createPluginHost(env, { pingMs: 60_000, startMs: 10_000, storageDebounceMs: 5, sliceMs: 8 })
      return { scene, host }
    }

    const record = (id: string, grant: string[]): { installed: Installed } => ({
      installed: {
        schema: 1,
        id,
        version: '0.1.0',
        source: { kind: 'file', name: 'real.hewplugin' },
        installed_at_ms: 0,
        grant,
        optional_granted: [],
        enabled: true,
        startup_allowed: false,
        data_version: 0,
      },
    })

    it('a write refused inside a command runs after the person grants it, on the same connection, in the same command', async () => {
      const id = 'com.example.widen'
      const rec = record(id, ['model.read'])
      const main = `hew.commands.handle('tag', async (p) => {
        if (!p.ask) return { first: 'running', granted: [], grant: [] }
        let first = 'ok'
        const rect = (x) => hew.api.sketch.drawRect({ plane: { ground: true }, corner_a: [x, 0, 0], corner_b: [x + 1, 1, 0] })
        try { await rect(0) } catch (e) { first = e.code }
        const asked = await hew.api.permissions.request({ permissions: ['model.write'] })
        await rect(3)
        return { first, granted: asked.granted, grant: hew.plugin.grant.permissions }
      })`
      const { scene, host: h } = realHost(
        { id, permissions: ['model.read'], optionalPermissions: ['model.write'], activation: ['on_command'], contributes: { commands: [{ id: 'tag', class: 'mutating', summary: 'Tags.', params: { type: 'object', properties: { ask: { type: 'boolean' } }, additionalProperties: false }, result: { type: 'object', properties: { first: {}, granted: {}, grant: {} }, additionalProperties: false } }] } },
        main,
        rec,
        {
          async decide(_id, add) {
            rec.installed = { ...rec.installed, optional_granted: [...rec.installed.optional_granted, ...add] }
            return 'granted'
          },
        },
      )
      await h.sync()
      const conn = scene.api_connection_open()
      await h.dispatch(conn, '{"jsonrpc":"2.0","id":1,"method":"hew.meta.hello","params":{"protocol":1}}')
      await h.dispatch(conn, '{"jsonrpc":"2.0","id":2,"method":"hew.doc.attach","params":{}}')
      const undoBefore = (JSON.parse((await h.dispatch(conn, '{"jsonrpc":"2.0","id":3,"method":"hew.history.status","params":{}}'))!) as { result: { undo_depth?: number } }).result
      // Started first: the person's act counts only for a running plugin.
      await h.dispatch(conn, `{"jsonrpc":"2.0","id":6,"method":"${id}.tag","params":{}}`)
      h.noteUserActivation(id)
      const reply = JSON.parse((await h.dispatch(conn, `{"jsonrpc":"2.0","id":4,"method":"${id}.tag","params":{"ask":true}}`))!) as { result?: unknown; error?: unknown }
      expect(reply.error).toBeUndefined()
      expect(reply.result).toEqual({ first: -32001, granted: ['model.write'], grant: ['model.read', 'model.write'] })
      expect(scene.busy()).toBe(false)
      const undoAfter = (JSON.parse((await h.dispatch(conn, '{"jsonrpc":"2.0","id":5,"method":"hew.history.status","params":{}}'))!) as { result: { undo_depth?: number } }).result
      expect(undoAfter).not.toEqual(undoBefore)
      expect(h.states()[id]).toBe('activated')
    })

    it('a listener on the real kernel starts the plugin on its event and hands it over', async () => {
      const id = 'com.example.listens'
      const rec = record(id, ['model.read'])
      const main = `globalThis.__heard = []
        hew.events.on('document_changed', (p) => globalThis.__heard.push(p.kind))
        hew.commands.handle('noop', () => null)`
      const { scene, host: h } = realHost(
        { id, permissions: ['model.read'], activation: ['on_command', 'on_event'], contributes: { commands: [{ id: 'noop', class: 'read_only', summary: 'Nothing.' }], events: ['document_changed'] } },
        main,
        rec,
      )
      await h.sync()
      await vi.waitFor(() => expect(h.listening()).toEqual([id]))
      const conn = scene.api_connection_open()
      await h.dispatch(conn, '{"jsonrpc":"2.0","id":1,"method":"hew.meta.hello","params":{"protocol":1}}')
      await h.dispatch(conn, '{"jsonrpc":"2.0","id":2,"method":"hew.doc.attach","params":{}}')
      await h.dispatch(conn, '{"jsonrpc":"2.0","id":3,"method":"hew.sketch.draw_rect","params":{"plane":{"ground":true},"corner_a":[0,0,0],"corner_b":[1,1,0]}}')
      await vi.waitFor(() => expect(h.states()[id]).toBe('activated'))
      await vi.waitFor(() => expect((globalThis as { __heard?: unknown[] }).__heard).toEqual(['commit']))
      expect(h.listening()).toEqual([])
    })
  })

  it("the web store's grant lines come from the real kernel", async () => {
    const root = new FakeDir('opfs-root')
    const store = makeWebPluginStore({
      wasm: async () => kernel as unknown as PluginWasm,
      opfsRoot: async () => root as unknown as FileSystemDirectoryHandle,
      channel: null,
      storage: null,
      // One holder at a time is all one test needs.
      locks: { request: (_name, callback) => callback() },
      hewVersion: () => '1.5.0',
    })
    const staged = await store.stageFile(pluginFile({ permissions: ['model.read'], optionalPermissions: ['network:api.example.com', 'view'] }))
    expect(await store.commit(staged.token, true)).not.toBeNull()
    const lines = await store.grantLines('com.example.one', ['model.read'], ['network:api.example.com'])
    expect(lines.added.map((l) => l.name)).toEqual(['network:api.example.com'])
    expect(lines.added[0].text.length).toBeGreaterThan(0)
    expect(lines.warnings).toEqual(['Can send what it reads from your model to api.example.com.'])
    await expect(store.grantLines('com.example.one', ['model.read'], ['files'])).rejects.toMatchObject({ refusal: 'unknown_permission' })
  })
})
