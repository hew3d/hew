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
      grantJson: (id, doc, names) => (kernel as unknown as { plugin_grant_json(a: string, b: string, c: string): string }).plugin_grant_json(id, doc, names),
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
    const grant = (kernel as unknown as { plugin_grant_json(a: string, b: string, c: string): string }).plugin_grant_json(
      'com.example.hello',
      'doc_1',
      '["model.read"]',
    )
    const conn = scene.api_plugin_connection_open(grant)
    scene.api_dispatch(conn, '{"jsonrpc":"2.0","id":1,"method":"hew.meta.hello","params":{"protocol":1}}')
    scene.api_dispatch(conn, '{"jsonrpc":"2.0","id":2,"method":"hew.doc.attach","params":{}}')
    const reply = JSON.parse(
      JSON.parse(scene.api_dispatch(conn, '{"jsonrpc":"2.0","id":3,"method":"hew.tag.create","params":{"path":"x"}}')!).reply as string,
    ) as { error?: { code: number } }
    expect(reply.error?.code).toBe(-32001)
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
        storage: { get: async () => null, set: async () => {}, delete: async () => {}, list: async () => [] },
        grantJson: (i, doc, names) => (kernel as unknown as { plugin_grant_json(a: string, b: string, c: string): string }).plugin_grant_json(i, doc, names),
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
})
