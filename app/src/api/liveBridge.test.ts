// @vitest-environment jsdom
//
// This suite needs a real `window` (isTauri reads `'__TAURI_INTERNALS__' in
// window`) — `vitest.config.ts` only routes `.test.tsx` files to jsdom by
// default, so this `.test.ts` file opts in explicitly (text/fontSources.test.ts
// is the existing precedent for this pragma).

/**
 * `liveBridge.ts` — the webview half of `--live` (docs/HEW_API.md §11.2).
 *
 * `isTauri` (io/fileHost.ts) is a module-level constant resolved once from
 * `'__TAURI_INTERNALS__' in window` — the same isolation note
 * App.openInNewWindow.test.tsx documents applies here: this file forces it
 * true, in its own isolated module graph (Vitest's per-file isolation), so
 * the rest of the suite's `isTauri === false` assumption is untouched.
 *
 * A real WASM `Scene` and a real Tauri socket are out of reach in a jsdom
 * test; what's covered here is `installLiveBridge`'s own logic — listener
 * wiring, the shell-connId-to-wasm-connId map, notification (no-reply)
 * handling, and the refresh-after-mutation decision — against a minimal
 * mocked `Scene` and a fake `getCurrentWebviewWindow().listen` that lets
 * the test fire events directly.
 */

import { describe, expect, it, vi, beforeEach } from 'vitest'

vi.hoisted(() => {
  Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
})

// ---------------------------------------------------------------------------
// Mocks — vi.mock() is hoisted before imports, so these must appear first.
// ---------------------------------------------------------------------------

type Handler = (event: { payload: unknown }) => void

const listeners = new Map<string, Handler>()
const unlistenSpies = new Map<string, ReturnType<typeof vi.fn>>()

const listenMock = vi.fn((event: string, cb: Handler) => {
  listeners.set(event, cb)
  const unlisten = vi.fn()
  unlistenSpies.set(event, unlisten)
  return Promise.resolve(unlisten)
})

vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({ listen: listenMock }),
}))

const emitMock = vi.fn(() => Promise.resolve())
vi.mock('@tauri-apps/api/event', () => ({ emit: emitMock }))

import { installLiveBridge, shouldRefreshAfterDispatch, type LiveBridgeDeps } from './liveBridge'
import type { Scene } from '../wasm/loader'
import type { ViewportApi } from '../viewport/Viewport'
import { getLengthUnit, setLengthUnit, subscribe as subscribeLengthUnit } from '../settings/units'

function fire(event: string, payload: unknown): void {
  listeners.get(event)?.({ payload })
}

/** Waits for `installLiveBridge`'s dynamic `import('@tauri-apps/api/webviewWindow')`
 * chain to settle and register its three listeners — dynamic `import()` resolves
 * over more microtask turns than a fixed number of `await Promise.resolve()`s
 * reliably covers, so this polls instead. */
async function flush(): Promise<void> {
  await vi.waitFor(() => {
    if (listenMock.mock.calls.length < 3) throw new Error('listeners not yet registered')
  })
}

describe('installLiveBridge', () => {
  let scene: {
    api_connection_open: ReturnType<typeof vi.fn>
    api_connection_close: ReturnType<typeof vi.fn>
    api_dispatch: ReturnType<typeof vi.fn>
    api_method_mutates: ReturnType<typeof vi.fn>
    take_pending_view_directive: ReturnType<typeof vi.fn>
  }
  let refreshScene: ReturnType<typeof vi.fn>
  let reconcile: ReturnType<typeof vi.fn>
  let viewportApi: {
    refreshScene: ReturnType<typeof vi.fn>
    setCamera: ReturnType<typeof vi.fn>
    setStandardView: ReturnType<typeof vi.fn>
    zoomExtents: ReturnType<typeof vi.fn>
  }
  let deps: LiveBridgeDeps

  beforeEach(() => {
    listeners.clear()
    unlistenSpies.clear()
    listenMock.mockClear()
    emitMock.mockClear()
    scene = {
      api_connection_open: vi.fn(() => 7),
      api_connection_close: vi.fn(),
      api_dispatch: vi.fn(),
      // Stands in for the real registry lookup (crates/api): the shapes
      // this suite dispatches are all mutating except the query/meta/attr
      // reads and the hew.view.* host effects (mutates_document = false)
      // it explicitly exercises.
      api_method_mutates: vi.fn(
        (m: string) =>
          !m.startsWith('hew.query.') &&
          !m.startsWith('hew.meta.') &&
          !m.startsWith('hew.view.') &&
          m !== 'hew.attr.get',
      ),
      // Stands in for `Scene::take_pending_view_directive` (crates/wasm-api):
      // `undefined` unless a test explicitly arms it with a `hew.view.*`
      // directive, mirroring the real accessor's "nothing to hand out" default.
      take_pending_view_directive: vi.fn(() => undefined as string | undefined),
    }
    refreshScene = vi.fn()
    viewportApi = {
      refreshScene,
      setCamera: vi.fn(),
      setStandardView: vi.fn(),
      zoomExtents: vi.fn(),
    }
    reconcile = vi.fn()
    deps = {
      getScene: () => scene as unknown as Scene,
      getViewportApi: () => viewportApi as unknown as ViewportApi,
      reconcile,
    }
  })

  it('registers all three window-scoped listeners', async () => {
    installLiveBridge(deps)
    await flush()
    expect(listenMock).toHaveBeenCalledWith('hew://api-connection-open', expect.any(Function))
    expect(listenMock).toHaveBeenCalledWith('hew://api-connection-close', expect.any(Function))
    expect(listenMock).toHaveBeenCalledWith('hew://api-frame', expect.any(Function))
  })

  it('opens a wasm connection and maps it to the shell connId', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    expect(scene.api_connection_open).toHaveBeenCalledTimes(1)

    scene.api_dispatch.mockReturnValue('{"jsonrpc":"2.0","id":0,"result":{}}')
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","id":0,"method":"hew.meta.hello"}' })
    expect(scene.api_dispatch).toHaveBeenCalledWith(7, expect.stringContaining('hew.meta.hello'))
  })

  it('forwards the reply back over hew://api-reply', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    scene.api_dispatch.mockReturnValue('{"jsonrpc":"2.0","id":1,"result":{"tree":[]}}')
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":1,"method":"hew.query.scene"}',
    })
    await vi.waitFor(() => {
      expect(emitMock).toHaveBeenCalledWith('hew://api-reply', {
        connId: 3,
        frame: '{"jsonrpc":"2.0","id":1,"result":{"tree":[]}}',
      })
    })
  })

  it('a notification (dispatch returns undefined) sends no reply', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    scene.api_dispatch.mockReturnValue(undefined)
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","method":"hew.query.scene"}' })
    await flush()
    expect(emitMock).not.toHaveBeenCalled()
  })

  it('a frame for an unopened connId is never dispatched', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-frame', { connId: 99, frame: '{"jsonrpc":"2.0","id":0,"method":"hew.query.scene"}' })
    expect(scene.api_dispatch).not.toHaveBeenCalled()
  })

  it('lazily opens a wasm connection on the first frame after a Scene-not-ready startup race', async () => {
    // The shell accepts the socket and fires 'hew://api-connection-open'
    // before the WASM Scene has finished loading — the connId is real
    // (the shell did accept it), but `handleOpen` could not open the wasm
    // side yet. Once the Scene becomes available, the first frame for
    // that same connId must open it lazily instead of staying dead for
    // the connection's whole lifetime (the defect this regresses).
    let sceneReady = false
    deps.getScene = () => (sceneReady ? (scene as unknown as Scene) : null)
    installLiveBridge(deps)
    await flush()

    fire('hew://api-connection-open', { connId: 3 })
    expect(scene.api_connection_open).not.toHaveBeenCalled()

    sceneReady = true
    scene.api_dispatch.mockReturnValue('{"jsonrpc":"2.0","id":0,"result":{}}')
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","id":0,"method":"hew.meta.hello"}' })

    expect(scene.api_connection_open).toHaveBeenCalledTimes(1)
    expect(scene.api_dispatch).toHaveBeenCalledWith(7, expect.stringContaining('hew.meta.hello'))

    // A second frame for the same connId must not lazy-open a second time.
    scene.api_dispatch.mockReturnValue('{"jsonrpc":"2.0","id":1,"result":{}}')
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","id":1,"method":"hew.query.scene"}' })
    expect(scene.api_connection_open).toHaveBeenCalledTimes(1)
  })

  it('replies -32004 not ready when a frame arrives while the Scene is still unavailable', async () => {
    deps.getScene = () => null
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","id":5,"method":"hew.meta.hello"}' })

    await vi.waitFor(() => {
      expect(emitMock).toHaveBeenCalledTimes(1)
    })
    const [event, payload] = emitMock.mock.calls[0] as unknown as [
      string,
      { connId: number; frame: string },
    ]
    expect(event).toBe('hew://api-reply')
    expect(payload.connId).toBe(3)
    const reply = JSON.parse(payload.frame) as { id: unknown; error?: { code: number } }
    expect(reply.id).toBe(5)
    expect(reply.error?.code).toBe(-32004)
    expect(scene.api_dispatch).not.toHaveBeenCalled()
  })

  it('drops a notification with no reply when the Scene is still unavailable', async () => {
    deps.getScene = () => null
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","method":"hew.event.whatever"}' })
    await flush()
    expect(emitMock).not.toHaveBeenCalled()
  })

  it('closing a connection drops its mapping and calls scene.api_connection_close', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    fire('hew://api-connection-close', { connId: 3 })
    expect(scene.api_connection_close).toHaveBeenCalledWith(7)

    scene.api_dispatch.mockClear()
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","id":0,"method":"hew.query.scene"}' })
    expect(scene.api_dispatch).not.toHaveBeenCalled()
  })

  it('refreshes the viewport after a successful mutating dispatch', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    scene.api_dispatch.mockReturnValue('{"jsonrpc":"2.0","id":2,"result":{"sketch":"skt_1"}}')
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":2,"method":"hew.sketch.draw_rect","params":{}}',
    })
    await vi.waitFor(() => {
      expect(refreshScene).toHaveBeenCalledTimes(1)
    })
    expect(reconcile).not.toHaveBeenCalled()
  })

  it('falls back to reconcile() when no Viewport is mounted', async () => {
    deps.getViewportApi = () => null
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    scene.api_dispatch.mockReturnValue('{"jsonrpc":"2.0","id":2,"result":{}}')
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":2,"method":"hew.sketch.draw_rect","params":{}}',
    })
    await vi.waitFor(() => {
      expect(reconcile).toHaveBeenCalledTimes(1)
    })
  })

  it('does not refresh after a read-only query', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    scene.api_dispatch.mockReturnValue('{"jsonrpc":"2.0","id":2,"result":{"tree":[]}}')
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":2,"method":"hew.query.scene","params":{}}',
    })
    await flush()
    expect(refreshScene).not.toHaveBeenCalled()
  })

  it('does not refresh after a refused mutating command', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    scene.api_dispatch.mockReturnValue(
      '{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"refused"}}',
    )
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":2,"method":"hew.solid.push_pull","params":{}}',
    })
    await flush()
    expect(refreshScene).not.toHaveBeenCalled()
  })

  it('uninstall stops routing further events', async () => {
    const uninstall = installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    uninstall()
    for (const unlisten of unlistenSpies.values()) {
      expect(unlisten).toHaveBeenCalledTimes(1)
    }
  })

  // hew.view.camera / zoom_extents / units: `LiveHost` (Rust) cannot reach
  // the viewport or app settings itself, so it leaves a `ViewDirective` for
  // `Scene::take_pending_view_directive` to hand this bridge right after
  // dispatch. These prove the bridge actually invokes the SAME viewport
  // calls the Camera menu/toolbar and View menu already use.
  describe('hew.view.* directives', () => {
    it('applies an explicit camera through Viewport.setCamera', async () => {
      installLiveBridge(deps)
      await flush()
      fire('hew://api-connection-open', { connId: 3 })
      scene.api_dispatch.mockReturnValue('{"jsonrpc":"2.0","id":2,"result":{}}')
      scene.take_pending_view_directive.mockReturnValue(
        JSON.stringify({
          kind: 'camera',
          camera: { eye: [1, 2, 3], target: [0, 0, 0], up: [0, 0, 1], fov_deg: 50 },
        }),
      )
      fire('hew://api-frame', {
        connId: 3,
        frame: '{"jsonrpc":"2.0","id":2,"method":"hew.view.camera","params":{}}',
      })
      expect(viewportApi.setCamera).toHaveBeenCalledWith(
        [1, 2, 3],
        [0, 0, 0],
        [0, 0, 1],
        50,
        'perspective',
      )
      // A view/camera effect is not a document mutation — it must never
      // trigger the same refresh a real mutating command would.
      expect(refreshScene).not.toHaveBeenCalled()
      expect(reconcile).not.toHaveBeenCalled()
    })

    it('resolves defaults (up, fov_deg, projection) the same way hew.view.snapshot does', async () => {
      installLiveBridge(deps)
      await flush()
      fire('hew://api-connection-open', { connId: 3 })
      scene.api_dispatch.mockReturnValue('{"jsonrpc":"2.0","id":2,"result":{}}')
      scene.take_pending_view_directive.mockReturnValue(
        JSON.stringify({ kind: 'camera', camera: { eye: [5, 0, 0], target: [0, 0, 0] } }),
      )
      fire('hew://api-frame', {
        connId: 3,
        frame: '{"jsonrpc":"2.0","id":2,"method":"hew.view.camera","params":{}}',
      })
      expect(viewportApi.setCamera).toHaveBeenCalledWith(
        [5, 0, 0],
        [0, 0, 0],
        [0, 0, 1],
        35,
        'perspective',
      )
    })

    it('applies a standard view through Viewport.setStandardView', async () => {
      installLiveBridge(deps)
      await flush()
      fire('hew://api-connection-open', { connId: 3 })
      scene.api_dispatch.mockReturnValue('{"jsonrpc":"2.0","id":2,"result":{}}')
      scene.take_pending_view_directive.mockReturnValue(
        JSON.stringify({ kind: 'camera', view: 'top' }),
      )
      fire('hew://api-frame', {
        connId: 3,
        frame: '{"jsonrpc":"2.0","id":2,"method":"hew.view.camera","params":{}}',
      })
      expect(viewportApi.setStandardView).toHaveBeenCalledWith('top')
    })

    it('applies zoom_extents through Viewport.zoomExtents', async () => {
      installLiveBridge(deps)
      await flush()
      fire('hew://api-connection-open', { connId: 3 })
      scene.api_dispatch.mockReturnValue('{"jsonrpc":"2.0","id":2,"result":{}}')
      scene.take_pending_view_directive.mockReturnValue(JSON.stringify({ kind: 'zoom_extents' }))
      fire('hew://api-frame', {
        connId: 3,
        frame: '{"jsonrpc":"2.0","id":2,"method":"hew.view.zoom_extents","params":{}}',
      })
      expect(viewportApi.zoomExtents).toHaveBeenCalledTimes(1)
    })

    it('a camera/zoom_extents directive with no mounted Viewport is a silent no-op', async () => {
      deps.getViewportApi = () => null
      installLiveBridge(deps)
      await flush()
      fire('hew://api-connection-open', { connId: 3 })
      scene.api_dispatch.mockReturnValue('{"jsonrpc":"2.0","id":2,"result":{}}')
      scene.take_pending_view_directive.mockReturnValue(JSON.stringify({ kind: 'zoom_extents' }))
      // Must not throw despite no Viewport being mounted.
      expect(() =>
        fire('hew://api-frame', {
          connId: 3,
          frame: '{"jsonrpc":"2.0","id":2,"method":"hew.view.zoom_extents","params":{}}',
        }),
      ).not.toThrow()
    })

    it('does nothing when there is no pending directive', async () => {
      installLiveBridge(deps)
      await flush()
      fire('hew://api-connection-open', { connId: 3 })
      scene.api_dispatch.mockReturnValue('{"jsonrpc":"2.0","id":2,"result":{"tree":[]}}')
      fire('hew://api-frame', {
        connId: 3,
        frame: '{"jsonrpc":"2.0","id":2,"method":"hew.query.scene","params":{}}',
      })
      expect(viewportApi.setCamera).not.toHaveBeenCalled()
      expect(viewportApi.setStandardView).not.toHaveBeenCalled()
      expect(viewportApi.zoomExtents).not.toHaveBeenCalled()
    })

    // hew.view.units is not a viewport call at all — it must go through
    // `app/src/settings/units.ts`'s OWN setter (persistence + subscriber
    // notification + the cross-window Tauri broadcast), never a bare
    // `localStorage.setItem`. Deliberately NOT mocking `../settings/units`
    // here (unlike the camera/zoom_extents cases above, which mock
    // `ViewportApi`): the point is to prove the REAL setter ran, with its
    // real side effects, not merely that some function was called.
    it('applies a units format through settings/units.ts\'s real setLengthUnit', async () => {
      const before = getLengthUnit()
      const next = before === 'cm' ? 'mm' : 'cm'
      const notified = vi.fn()
      const unsubscribe = subscribeLengthUnit(notified)
      try {
        installLiveBridge(deps)
        await flush()
        fire('hew://api-connection-open', { connId: 3 })
        scene.api_dispatch.mockReturnValue('{"jsonrpc":"2.0","id":2,"result":{}}')
        scene.take_pending_view_directive.mockReturnValue(
          JSON.stringify({ kind: 'units', format: next }),
        )
        fire('hew://api-frame', {
          connId: 3,
          frame: '{"jsonrpc":"2.0","id":2,"method":"hew.view.units","params":{}}',
        })
        // The real setter's own effects — persistence, the singleton, and
        // subscriber notification — all fired. A direct `localStorage`
        // write would do none of the latter two.
        expect(getLengthUnit()).toBe(next)
        expect(localStorage.getItem('hew.settings.lengthUnit')).toBe(next)
        expect(notified).toHaveBeenCalledWith(next)
        // Never a viewport call.
        expect(viewportApi.setCamera).not.toHaveBeenCalled()
        expect(viewportApi.setStandardView).not.toHaveBeenCalled()
        expect(viewportApi.zoomExtents).not.toHaveBeenCalled()
      } finally {
        unsubscribe()
        setLengthUnit(before) // restore the module singleton for later tests
      }
    })
  })
})

describe('shouldRefreshAfterDispatch', () => {
  // The registry is the authority; this stub stands in for
  // `Scene.api_method_mutates` (crates/api's CommandClass).
  const stubScene = {
    api_method_mutates: (m: string) =>
      !m.startsWith('hew.query.') && !m.startsWith('hew.meta.') && m !== 'hew.attr.get',
  } as unknown as Parameters<typeof shouldRefreshAfterDispatch>[0]

  it('is true for a successful mutating command', () => {
    expect(
      shouldRefreshAfterDispatch(
        stubScene,
        '{"jsonrpc":"2.0","id":1,"method":"hew.solid.push_pull","params":{}}',
        '{"jsonrpc":"2.0","id":1,"result":{}}',
      ),
    ).toBe(true)
  })

  it('is false for hew.query.*', () => {
    expect(
      shouldRefreshAfterDispatch(
        stubScene,
        '{"jsonrpc":"2.0","id":1,"method":"hew.query.scene","params":{}}',
        '{"jsonrpc":"2.0","id":1,"result":{"tree":[]}}',
      ),
    ).toBe(false)
  })

  it('is false for hew.meta.*', () => {
    expect(
      shouldRefreshAfterDispatch(
        stubScene,
        '{"jsonrpc":"2.0","id":1,"method":"hew.meta.capabilities","params":{}}',
        '{"jsonrpc":"2.0","id":1,"result":{"commands":[]}}',
      ),
    ).toBe(false)
  })

  it('is false for hew.attr.get', () => {
    expect(
      shouldRefreshAfterDispatch(
        stubScene,
        '{"jsonrpc":"2.0","id":1,"method":"hew.attr.get","params":{}}',
        '{"jsonrpc":"2.0","id":1,"result":{}}',
      ),
    ).toBe(false)
  })

  it('is false when the reply is a refusal', () => {
    expect(
      shouldRefreshAfterDispatch(
        stubScene,
        '{"jsonrpc":"2.0","id":1,"method":"hew.solid.push_pull","params":{}}',
        '{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"refused"}}',
      ),
    ).toBe(false)
  })

  it('asks the registry, not the method name — a mutating command outside the naming convention still refreshes', () => {
    // The exact drift this fix exists to prevent: a future mutating
    // command whose name does not match the old hew.query./hew.meta.
    // heuristic must still refresh, or the user's viewport goes stale
    // with no error anywhere.
    const registryScene = {
      api_method_mutates: (m: string) => m === 'hew.future.reshape',
    } as unknown as Parameters<typeof shouldRefreshAfterDispatch>[0]
    expect(
      shouldRefreshAfterDispatch(
        registryScene,
        '{"jsonrpc":"2.0","id":1,"method":"hew.future.reshape","params":{}}',
        '{"jsonrpc":"2.0","id":1,"result":{}}',
      ),
    ).toBe(true)
  })

  it('is false for malformed frame or reply text', () => {
    expect(shouldRefreshAfterDispatch(stubScene, 'not json', '{"jsonrpc":"2.0","id":1,"result":{}}')).toBe(
      false,
    )
    expect(
      shouldRefreshAfterDispatch(
        stubScene,
        '{"jsonrpc":"2.0","id":1,"method":"hew.solid.push_pull"}',
        'not json',
      ),
    ).toBe(false)
  })
})
