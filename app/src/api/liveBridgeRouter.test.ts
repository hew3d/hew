// @vitest-environment jsdom
/**
 * The live bridge with the forward router consulted: what it does when the
 * registry turns the `plugin-forward` coupling on and a plugin host filled
 * the slot. The bridge keeps owning the compound, the busy state and the
 * completion; the router only answers. (With the coupling off, which is
 * every production build, `liveBridge.test.ts` shows the router is never
 * asked.)
 */

import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'

vi.hoisted(() => {
  Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
})

type Handler = (event: { payload: unknown }) => void
const listeners = new Map<string, Handler>()
const listenMock = vi.fn((event: string, cb: Handler) => {
  listeners.set(event, cb)
  return Promise.resolve(() => {})
})
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: () => ({ listen: listenMock }) }))
const emitMock = vi.fn((_event: string, _payload: unknown) => Promise.resolve())
vi.mock('@tauri-apps/api/event', () => ({ emit: emitMock }))
vi.mock('../preview', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../preview')>()),
  behaviorEnabled: (id: string) => id === 'plugin-forward',
}))

import { installLiveBridge, type LiveBridgeDeps } from './liveBridge'
import { getBusy, setBusy } from './busyState'
import { setForwardRouter, type ForwardReplyJson, type ForwardRouter } from './forwardRouter'
import { abortCompound, beginCompound, type OpenCompound } from './hostCompounds'
import type { ForwardRequestJson } from './dispatchResult'
import type { Scene } from '../wasm/loader'
import type { ViewportApi } from '../viewport/Viewport'

const fire = (event: string, payload: unknown): void => listeners.get(event)?.({ payload })

function forward(mutating: boolean): ForwardRequestJson {
  return {
    token: 4,
    plugin: 'com.example.lib',
    local: 'cmd',
    method: 'com.example.lib.cmd',
    request_id: 9,
    params: {},
    mutating,
    label: 'Do it',
    timeout_ms: 50_000,
    concurrency: 'parallel',
    caller: { identity: 'live:7', plugin: null, app: null, forward: null },
  }
}

describe('the live bridge with a plugin host serving forwards', () => {
  let scene: Record<string, ReturnType<typeof vi.fn>>
  let deps: LiveBridgeDeps
  let uninstall: () => void
  let refreshScene: ReturnType<typeof vi.fn>
  let answer: (reply: ForwardReplyJson) => void
  let signal: AbortSignal | null
  let serve: Mock<ForwardRouter['serve']>

  beforeEach(async () => {
    listeners.clear()
    listenMock.mockClear()
    emitMock.mockClear()
    setBusy(null)
    let busy = false
    scene = {
      api_connection_open: vi.fn(() => 7),
      api_connection_close: vi.fn(),
      api_dispatch: vi.fn(),
      api_complete_forward: vi.fn((_w: number, _t: bigint, reply: string) =>
        JSON.stringify((JSON.parse(reply) as { ok?: unknown }).ok !== undefined ? { jsonrpc: '2.0', id: 9, result: {} } : { jsonrpc: '2.0', id: 9, error: { code: -32000 } }),
      ),
      set_display_units: vi.fn(),
      api_method_mutates: vi.fn(() => true),
      take_pending_view_directives: vi.fn(() => '[]'),
      busy: vi.fn(() => busy),
      compound_begin: vi.fn(() => {
        busy = true
        return 11n
      }),
      compound_end: vi.fn(() => {
        busy = false
        return '{}'
      }),
      compound_abort: vi.fn(() => {
        busy = false
      }),
      drain_events: vi.fn(() => '[]'),
      set_host_document_id: vi.fn(),
      set_ui_gesture_armed: vi.fn(),
    }
    refreshScene = vi.fn()
    deps = {
      getScene: () => scene as unknown as Scene,
      getViewportApi: () => ({ refreshScene, hasArmedGesture: () => false }) as unknown as ViewportApi,
      reconcile: vi.fn(),
      reseedViewState: vi.fn(),
      reconcileVisibility: vi.fn(),
    }
    signal = null
    serve = vi.fn<ForwardRouter['serve']>((_fwd: ForwardRequestJson, s: AbortSignal) => {
      signal = s
      return new Promise<ForwardReplyJson>((resolve) => (answer = resolve))
    })
    setForwardRouter({ serve })
    uninstall = installLiveBridge(deps)
    await vi.waitFor(() => expect(listenMock.mock.calls.length).toBeGreaterThanOrEqual(5))
    fire('hew://api-connection-open', { connId: 3 })
  })
  afterEach(() => {
    uninstall()
    setForwardRouter(null)
  })

  const send = (mutating: boolean): void => {
    scene.api_dispatch.mockReturnValueOnce(JSON.stringify({ pending: { token: 4, forward: forward(mutating) } }))
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","id":9,"method":"com.example.lib.cmd","params":{}}' })
  }
  const replies = (): unknown[] => emitMock.mock.calls.filter(([e]) => e === 'hew://api-reply').map(([, p]) => p)

  it('holds the document while the plugin works, then ends the compound on ok', async () => {
    send(true)
    expect(serve).toHaveBeenCalledTimes(1)
    expect(scene.compound_begin).toHaveBeenCalledWith(7, 4n)
    expect(getBusy()).toMatchObject({ plugin: 'com.example.lib', label: 'Do it' })
    expect(replies()).toEqual([])
    answer({ ok: { n: 1 } })
    await vi.waitFor(() => expect(replies()).toHaveLength(1))
    expect(scene.compound_end).toHaveBeenCalledWith(11n)
    expect(scene.compound_abort).not.toHaveBeenCalled()
    expect(scene.api_complete_forward).toHaveBeenCalledWith(7, 4n, '{"ok":{"n":1}}')
    expect(refreshScene).toHaveBeenCalled()
    expect(getBusy()).toBeNull()
  })

  it('aborts the compound when the plugin refuses', async () => {
    send(true)
    answer({ refusal: { name: 'nope', explanation: 'No.' } })
    await vi.waitFor(() => expect(replies()).toHaveLength(1))
    expect(scene.compound_abort).toHaveBeenCalledWith(11n)
    expect(scene.compound_end).not.toHaveBeenCalled()
    expect(scene.api_complete_forward).toHaveBeenCalledWith(7, 4n, '{"refusal":{"name":"nope","explanation":"No."}}')
    expect(getBusy()).toBeNull()
  })

  it('on Cancel answers cancelled at once, tells the host, and ignores what the plugin says later', async () => {
    send(true)
    getBusy()!.cancel()
    expect(signal!.aborted).toBe(true)
    expect(scene.compound_abort).toHaveBeenCalledTimes(1)
    expect(scene.api_complete_forward).toHaveBeenCalledWith(7, 4n, '{"failed":"cancelled"}')
    answer({ ok: {} })
    await Promise.resolve()
    await Promise.resolve()
    expect(scene.api_complete_forward).toHaveBeenCalledTimes(1)
    expect(scene.compound_end).not.toHaveBeenCalled()
  })

  it("on the shell's timeout answers timeout and tells the host", () => {
    send(true)
    fire('hew://api-abort', { connId: 3 })
    expect(signal!.aborted).toBe(true)
    expect(scene.api_complete_forward).toHaveBeenCalledWith(7, 4n, '{"failed":"timeout"}')
  })

  it('serves a read-only forward without holding the document', async () => {
    send(false)
    expect(scene.compound_begin).not.toHaveBeenCalled()
    expect(getBusy()).toBeNull()
    answer({ ok: 'hi' })
    await vi.waitFor(() => expect(replies()).toHaveLength(1))
    expect(scene.api_complete_forward).toHaveBeenCalledWith(7, 4n, '{"ok":"hi"}')
  })

  it('stops serving a read-only forward when its connection closes', async () => {
    send(false)
    fire('hew://api-connection-close', { connId: 3 })
    expect(signal!.aborted).toBe(true)
    answer({ ok: 'late' })
    await Promise.resolve()
    await Promise.resolve()
    expect(scene.api_complete_forward).not.toHaveBeenCalled()
  })

  it("ends a compound a plugin opened inside the bridge's before ending its own, and frees the document", async () => {
    // A kernel that closes only the innermost compound, as the real one does.
    const stack: bigint[] = []
    let next = 11n
    scene.busy.mockImplementation(() => stack.length > 0)
    scene.compound_begin.mockImplementation(() => {
      stack.push(next)
      return next++
    })
    scene.compound_end.mockImplementation((k: bigint) => {
      if (stack.at(-1) !== k) throw new Error('CompoundTokenMismatch')
      stack.pop()
      return '{}'
    })
    scene.compound_abort.mockImplementation((k: bigint) => {
      if (stack.at(-1) !== k) throw new Error('CompoundTokenMismatch')
      stack.pop()
    })
    send(true)
    // The plugin's handler calls another mutating command and does not wait.
    let inner: OpenCompound | null = null
    const unwind = vi.fn(() => abortCompound(scene as unknown as Scene, inner!))
    inner = beginCompound(scene as unknown as Scene, 8, 5, unwind)
    expect(stack).toEqual([11n, 12n])
    answer({ ok: { n: 1 } })
    await vi.waitFor(() => expect(replies()).toHaveLength(1))
    expect(unwind).toHaveBeenCalledTimes(1)
    expect(scene.compound_abort).toHaveBeenCalledWith(12n)
    expect(scene.compound_end).toHaveBeenCalledWith(11n)
    expect(scene.api_complete_forward).toHaveBeenCalledWith(7, 4n, '{"ok":{"n":1}}')
    expect(stack).toEqual([])
    expect(getBusy()).toBeNull()
  })

  it('redraws a change that committed while its socket closed', async () => {
    send(true)
    fire('hew://api-connection-close', { connId: 3 })
    // The kernel no longer knows the connection.
    scene.api_complete_forward.mockImplementation(() => {
      throw new Error('FORWARD: unknown API connection')
    })
    answer({ ok: { n: 1 } })
    await vi.waitFor(() => expect(scene.compound_end).toHaveBeenCalledWith(11n))
    await vi.waitFor(() => expect(refreshScene).toHaveBeenCalled())
    expect(deps.reconcileVisibility).toHaveBeenCalled()
    expect(getBusy()).toBeNull()
  })

  it('answers crashed when the router itself fails', async () => {
    serve.mockImplementationOnce(() => Promise.reject(new Error('boom')))
    send(true)
    await vi.waitFor(() => expect(replies()).toHaveLength(1))
    expect(scene.api_complete_forward).toHaveBeenCalledWith(7, 4n, '{"failed":"crashed"}')
    expect(scene.compound_abort).toHaveBeenCalled()
  })
})
