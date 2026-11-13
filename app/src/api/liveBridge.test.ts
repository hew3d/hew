// @vitest-environment jsdom
//
// This suite needs a real `window` (isTauri reads `'__TAURI_INTERNALS__' in
// window`) — `vitest.config.ts` only routes `.test.tsx` files to jsdom by
// default, so this `.test.ts` file opts in explicitly (text/fontSources.test.ts
// is the existing precedent for this pragma).

/**
 * `liveBridge.ts` — the webview half of `--live` (docs/agents/HEW_API.md §11.2).
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

import { describe, expect, it, vi, beforeEach, type Mock } from 'vitest'

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

import { dispatchFrameNow, installLiveBridge, shouldRefreshAfterDispatch, type LiveBridgeDeps } from './liveBridge'
import type { Scene } from '../wasm/loader'
import type { ViewportApi } from '../viewport/Viewport'
import { getLengthUnit, setLengthUnit, subscribe as subscribeLengthUnit } from '../settings/units'
import { tagged } from '../test/taggedDispatch'
import { getBusy, setBusy } from './busyState'

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
    api_complete_forward: ReturnType<typeof vi.fn>
    set_display_units: ReturnType<typeof vi.fn>
    api_method_mutates: ReturnType<typeof vi.fn>
    take_pending_view_directives: ReturnType<typeof vi.fn>
    busy: ReturnType<typeof vi.fn>
    compound_begin: ReturnType<typeof vi.fn>
    compound_abort: ReturnType<typeof vi.fn>
    drain_events: ReturnType<typeof vi.fn>
    set_host_document_id: ReturnType<typeof vi.fn>
    set_ui_gesture_armed: ReturnType<typeof vi.fn>
  }
  let uninstall: (() => void) | null = null
  let refreshScene: ReturnType<typeof vi.fn>
  // Typed concretely rather than as `ReturnType<typeof vi.fn>`: since Vitest 4
  // a bare `vi.fn()` is `Mock<Constructable | Procedure>`, which does not
  // satisfy `LiveBridgeDeps.reconcile`'s plain `() => void`.
  let reconcile: Mock<() => void>
  let reconcileVisibility: Mock<() => void>
  let reseedViewState: Mock<() => void>
  let viewportApi: {
    refreshScene: ReturnType<typeof vi.fn>
    setCamera: ReturnType<typeof vi.fn>
    setStandardView: ReturnType<typeof vi.fn>
    zoomExtents: ReturnType<typeof vi.fn>
    isolate: ReturnType<typeof vi.fn>
    clearIsolation: ReturnType<typeof vi.fn>
    zoomToWorldBounds: ReturnType<typeof vi.fn>
    hasArmedGesture: ReturnType<typeof vi.fn>
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
      api_complete_forward: vi.fn(),
      set_display_units: vi.fn(),
      // Stands in for the real registry lookup (crates/api): the shapes
      // this suite dispatches are all mutating except the query/meta/attr
      // reads and the hew.view.* host effects (mutates_document = false)
      // it explicitly exercises.
      api_method_mutates: vi.fn(
        (m: string) =>
          !m.startsWith('hew.query.') &&
          !m.startsWith('hew.meta.') &&
          !m.startsWith('hew.view.') &&
          !m.startsWith('hew.selection.') &&
          m !== 'hew.attr.get',
      ),
      // Stands in for `Scene::take_pending_view_directives` (crates/wasm-api):
      // `[]` unless a test explicitly arms it with directives, mirroring the
      // real accessor's "nothing to hand out" default.
      take_pending_view_directives: vi.fn(() => '[]'),
      // Stands in for the kernel's compound stack (`Scene.busy` is true
      // between `compound_begin` and its abort).
      busy: vi.fn(() => false),
      compound_begin: vi.fn(() => {
        scene.busy.mockReturnValue(true)
        return 11n
      }),
      compound_abort: vi.fn(() => {
        scene.busy.mockReturnValue(false)
      }),
      // Stands in for `Scene::drain_events`: nothing owed unless a test
      // arms it.
      drain_events: vi.fn(() => '[]'),
      set_host_document_id: vi.fn(),
      set_ui_gesture_armed: vi.fn(),
    }
    // A bridge a previous test installed would keep its event sink.
    uninstall?.()
    uninstall = null
    setBusy(null)
    refreshScene = vi.fn()
    viewportApi = {
      refreshScene,
      setCamera: vi.fn(),
      setStandardView: vi.fn(),
      zoomExtents: vi.fn(),
      isolate: vi.fn(),
      clearIsolation: vi.fn(),
      zoomToWorldBounds: vi.fn(),
      hasArmedGesture: vi.fn(() => false),
    }
    reconcile = vi.fn<() => void>()
    reconcileVisibility = vi.fn<() => void>()
    reseedViewState = vi.fn<() => void>()
    deps = {
      getScene: () => scene as unknown as Scene,
      getViewportApi: () => viewportApi as unknown as ViewportApi,
      reconcile,
      reconcileVisibility,
      reseedViewState,
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

    scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":0,"result":{}}'))
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","id":0,"method":"hew.meta.hello"}' })
    expect(scene.api_dispatch).toHaveBeenCalledWith(7, expect.stringContaining('hew.meta.hello'))
  })

  it('tells the scene the app\'s display units before each dispatch, for a .skp export to open in', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    setLengthUnit('cm')
    scene.api_dispatch.mockImplementation(() => {
      expect(scene.set_display_units).toHaveBeenLastCalledWith('cm')
      return tagged('{"jsonrpc":"2.0","id":0,"result":{}}')
    })
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","id":0,"method":"hew.meta.hello"}' })
    expect(scene.api_dispatch).toHaveBeenCalledTimes(1)
    setLengthUnit('m')
  })

  describe('hew.doc.revert (the shell routes it to the window as a "revert" operation)', () => {
    it('runs the window\'s revert and answers success over hew://api-op-reply', async () => {
      const runOp = vi.fn(async () => ({ ok: true }))
      installLiveBridge({ ...deps, runOp })
      await vi.waitFor(() => {
        if (listenMock.mock.calls.length < 4) throw new Error('listeners not yet registered')
      })
      fire('hew://api-op', { opId: 11, op: 'revert', args: {} })
      await vi.waitFor(() => {
        expect(emitMock).toHaveBeenCalledWith('hew://api-op-reply', { opId: 11, ok: true })
      })
      expect(runOp).toHaveBeenCalledWith('revert', {})
    })

    it('hands the window\'s typed refusal back to the shell', async () => {
      const runOp = vi.fn(async () => ({
        ok: false,
        refusal: 'nothing_to_revert',
        message: 'There is nothing to revert: the document has no changes since the last save.',
      }))
      installLiveBridge({ ...deps, runOp })
      await vi.waitFor(() => {
        if (listenMock.mock.calls.length < 4) throw new Error('listeners not yet registered')
      })
      fire('hew://api-op', { opId: 12, op: 'revert', args: {} })
      await vi.waitFor(() => {
        expect(emitMock).toHaveBeenCalledWith('hew://api-op-reply', {
          opId: 12,
          ok: false,
          refusal: 'nothing_to_revert',
          message: 'There is nothing to revert: the document has no changes since the last save.',
        })
      })
    })

    it('a window with no operation runner refuses host_capability_missing', async () => {
      installLiveBridge(deps)
      await vi.waitFor(() => {
        if (listenMock.mock.calls.length < 4) throw new Error('listeners not yet registered')
      })
      fire('hew://api-op', { opId: 13, op: 'revert', args: {} })
      await vi.waitFor(() => {
        expect(emitMock).toHaveBeenCalledWith(
          'hew://api-op-reply',
          expect.objectContaining({ opId: 13, ok: false, refusal: 'host_capability_missing' }),
        )
      })
    })
  })

  it('a lifecycle frame is offered to the shell for connections bound elsewhere', async () => {
    uninstall = installLiveBridge(deps)
    await flush()
    scene.api_connection_open.mockReturnValue(78)
    fire('hew://api-connection-open', { connId: 4 })
    await vi.waitFor(() => {
      expect(listeners.has('hew://document-id')).toBe(true)
    })
    const opened = '{"jsonrpc":"2.0","method":"hew.event.document_opened","params":{"document":"doc_2"}}'
    // This window's own session (wasm 78) was sent it; the shell is told to
    // skip that connection and offer the frame to every other.
    scene.drain_events.mockReturnValueOnce(
      JSON.stringify([
        { conn: 78, frames: [opened] },
        { conn: null, frames: [opened] },
      ]),
    )
    scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":1,"result":{}}'))
    fire('hew://api-frame', {
      connId: 4,
      frame: '{"jsonrpc":"2.0","id":1,"method":"hew.sketch.draw_rect","params":{}}',
    })
    await vi.waitFor(() => {
      expect(emitMock).toHaveBeenCalledWith('hew://api-broadcast', { frame: opened, except: [4] })
    })
  })

  it('events for a socket connection are emitted after a dispatch', async () => {
    uninstall = installLiveBridge(deps)
    await flush()
    // An id no earlier test's bridge (each keeps its sink) ever mapped.
    scene.api_connection_open.mockReturnValue(77)
    fire('hew://api-connection-open', { connId: 3 })
    await vi.waitFor(() => {
      expect(listeners.has('hew://document-id')).toBe(true)
    })
    fire('hew://document-id', { id: 'doc_4' })
    expect(scene.set_host_document_id).toHaveBeenCalledWith('doc_4')
    const event = '{"jsonrpc":"2.0","method":"hew.event.document_changed","params":{}}'
    // Wasm connection 77 is this socket's; 9 is someone else's.
    scene.drain_events.mockReturnValueOnce(
      JSON.stringify([
        { conn: 77, frames: [event] },
        { conn: 9, frames: ['{"jsonrpc":"2.0","method":"hew.event.units_changed","params":{}}'] },
      ]),
    )
    scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":1,"result":{}}'))
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":1,"method":"hew.sketch.draw_rect","params":{}}',
    })
    await vi.waitFor(() => {
      expect(emitMock).toHaveBeenCalledWith('hew://api-event', { connId: 3, frame: event })
    })
    const sent = emitMock.mock.calls.map((c) => (c as unknown as [string, unknown])[0])
    expect(sent.indexOf('hew://api-reply')).toBeLessThan(sent.indexOf('hew://api-event'))
    expect(sent.filter((e) => e === 'hew://api-event')).toHaveLength(1)
    expect(scene.drain_events).toHaveBeenCalledTimes(1)
    // The id told before the dispatch, too.
    expect(scene.set_host_document_id).toHaveBeenLastCalledWith('doc_4')
  })

  it('forwards the reply back over hew://api-reply', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":1,"result":{"tree":[]}}'))
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

  it('completes a plugin forward as unavailable and sends that reply', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    const forward = {
      token: 4,
      plugin: 'com.example.lib',
      local: 'mut',
      method: 'com.example.lib.mut',
      request_id: 9,
      params: {},
      mutating: true,
      label: null,
      timeout_ms: 50000,
      concurrency: 'parallel',
      caller: { identity: 'live:7', plugin: null, app: null, forward: null },
    }
    scene.api_dispatch.mockReturnValue(JSON.stringify({ pending: { token: 4, forward } }))
    const refused =
      '{"jsonrpc":"2.0","id":9,"error":{"code":-32000,"message":"refused","data":{"refusal":"plugin_unavailable"}}}'
    scene.api_complete_forward.mockReturnValue(refused)
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":9,"method":"com.example.lib.mut","params":{}}',
    })
    await vi.waitFor(() => {
      expect(emitMock).toHaveBeenCalledWith('hew://api-reply', { connId: 3, frame: refused })
    })
    expect(scene.api_complete_forward).toHaveBeenCalledWith(7, 4n, '{"failed":"unavailable"}')
    // A mutating command runs in a compound, which the unanswered forward
    // aborts: the abort tail re-reads the view state and redraws once.
    expect(scene.compound_begin).toHaveBeenCalledWith(7, 4n)
    expect(scene.compound_abort).toHaveBeenCalledWith(11n)
    expect(reseedViewState).toHaveBeenCalledTimes(1)
    expect(refreshScene).toHaveBeenCalledTimes(1)
    expect(getBusy()).toBeNull()
  })

  it('an aborted compound puts back the selection and isolation, unframed', async () => {
    const selectMany = vi.fn()
    deps.selectMany = selectMany
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    scene.api_dispatch.mockReturnValue(
      JSON.stringify({
        pending: {
          token: 4,
          forward: {
            token: 4,
            plugin: 'com.example.lib',
            local: 'mut',
            method: 'com.example.lib.mut',
            request_id: 9,
            params: {},
            mutating: true,
            label: null,
            timeout_ms: 50000,
            concurrency: 'parallel',
            caller: { identity: 'live:7', plugin: null, app: null, forward: null },
          },
        },
      }),
    )
    // What `Scene::compound_abort` leaves: the selection and isolation at begin.
    scene.compound_abort.mockImplementation(() => {
      scene.busy.mockReturnValue(false)
      scene.take_pending_view_directives.mockReturnValueOnce(
        JSON.stringify([
          { kind: 'selection', mode: 'replace', entries: [{ kind: 'object', id: '2' }] },
          { kind: 'isolate', frame: false, entries: [{ kind: 'group', id: '5' }] },
        ]),
      )
    })
    // Completing the forward clears what is pending, so the bridge takes the
    // restore before it.
    scene.api_complete_forward.mockImplementation(() => {
      scene.take_pending_view_directives.mockReturnValue('[]')
      return '{"jsonrpc":"2.0","id":9,"error":{"code":-32000,"message":"refused"}}'
    })
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":9,"method":"com.example.lib.mut","params":{}}',
    })
    await vi.waitFor(() => expect(scene.compound_abort).toHaveBeenCalled())
    expect(selectMany).toHaveBeenCalledWith([{ kind: 'object', id: 2n }], 'replace')
    expect(viewportApi.isolate).toHaveBeenCalledWith([{ kind: 'group', id: 5n }], { frame: false })
  })

  /** A mutating forward's answer for `queuedFrames`' tests. */
  const mutatingForward = {
    token: 4,
    plugin: 'com.example.lib',
    local: 'mut',
    method: 'com.example.lib.mut',
    request_id: 9,
    params: {},
    mutating: true,
    label: 'Add box',
    timeout_ms: 50000,
    concurrency: 'parallel',
    caller: { identity: 'live:7', plugin: null, app: null, forward: null },
  }

  it('holds the document while a mutating forward is in flight, then dispatches what queued in order', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    fire('hew://api-connection-open', { connId: 5 })
    scene.api_complete_forward.mockReturnValue('{"jsonrpc":"2.0","id":9,"error":{"code":-32000}}')
    scene.api_dispatch.mockReturnValueOnce(JSON.stringify({ pending: { token: 4, forward: mutatingForward } }))
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","id":9,"method":"com.example.lib.mut"}' })
    // In flight: busy, with the command's label for the notice.
    expect(getBusy()).toMatchObject({ plugin: 'com.example.lib', label: 'Add box' })
    // Two frames arrive meanwhile: queued, not dispatched.
    fire('hew://api-frame', { connId: 5, frame: '{"jsonrpc":"2.0","id":1,"method":"hew.query.scene"}' })
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","id":2,"method":"hew.query.scene"}' })
    expect(scene.api_dispatch).toHaveBeenCalledTimes(1)
    await vi.waitFor(() => {
      expect(emitMock).toHaveBeenCalledWith('hew://api-queued', { connId: 5 })
      expect(emitMock).toHaveBeenCalledWith('hew://api-queued', { connId: 3 })
    })
    scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":1,"result":{}}'))
    // The placeholder answer ends the command; the queue drains in order,
    // each frame announced to the shell right before it runs.
    await vi.waitFor(() => {
      expect(scene.api_dispatch).toHaveBeenCalledTimes(3)
    })
    expect(scene.compound_abort).toHaveBeenCalledTimes(1)
    expect(getBusy()).toBeNull()
    expect(scene.api_dispatch.mock.calls[1][1]).toContain('"id":1')
    expect(scene.api_dispatch.mock.calls[2][1]).toContain('"id":2')
    await vi.waitFor(() => {
      expect(emitMock).toHaveBeenCalledWith('hew://api-started', { connId: 3 })
    })
    const events = emitMock.mock.calls.map((c) => (c as unknown as [string, { connId: number }])).map(
      ([event, payload]) => `${event}:${payload.connId}`,
    )
    const replyAt = events.indexOf('hew://api-reply:3')
    const started5 = events.indexOf('hew://api-started:5')
    const started3 = events.indexOf('hew://api-started:3')
    expect(replyAt).toBeGreaterThanOrEqual(0)
    expect(started5).toBeGreaterThan(replyAt)
    expect(started3).toBeGreaterThan(started5)
  })

  it('hew://api-abort aborts the compound held for that connection and answers the forward once', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    scene.api_complete_forward.mockReturnValue('{"jsonrpc":"2.0","id":9,"error":{"code":-32003}}')
    scene.api_dispatch.mockReturnValueOnce(JSON.stringify({ pending: { token: 4, forward: mutatingForward } }))
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","id":9,"method":"com.example.lib.mut"}' })
    // Another connection's timeout is not this one's.
    fire('hew://api-abort', { connId: 8 })
    expect(scene.compound_abort).not.toHaveBeenCalled()
    fire('hew://api-abort', { connId: 3 })
    expect(scene.compound_abort).toHaveBeenCalledWith(11n)
    expect(scene.api_complete_forward).toHaveBeenCalledWith(7, 4n, '{"failed":"timeout"}')
    expect(getBusy()).toBeNull()
    // The placeholder answer that would have come next finds it settled.
    await Promise.resolve()
    await Promise.resolve()
    expect(scene.api_complete_forward).toHaveBeenCalledTimes(1)
    expect(scene.compound_abort).toHaveBeenCalledTimes(1)
  })

  it('Cancel on the busy notice aborts the command and answers it cancelled, once', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    scene.api_complete_forward.mockReturnValue('{"jsonrpc":"2.0","id":9,"error":{"code":-32000}}')
    scene.api_dispatch.mockReturnValueOnce(JSON.stringify({ pending: { token: 4, forward: mutatingForward } }))
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","id":9,"method":"com.example.lib.mut"}' })
    const busy = getBusy()
    busy?.cancel()
    busy?.cancel()
    expect(scene.api_complete_forward).toHaveBeenCalledTimes(1)
    expect(scene.api_complete_forward).toHaveBeenCalledWith(7, 4n, '{"failed":"cancelled"}')
    expect(scene.compound_abort).toHaveBeenCalledTimes(1)
    expect(reseedViewState).toHaveBeenCalledTimes(1)
    expect(getBusy()).toBeNull()
  })

  it('each plugin command that finishes restarts the clock of every frame still waiting', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    fire('hew://api-connection-open', { connId: 5 })
    scene.busy.mockReturnValue(true)
    setBusy({ plugin: 'org.hew3d.harness', label: 'test', startedAt: 0, cancel: () => {} })
    // Connection 3 waits with a plugin command, 5 with a read behind it.
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","id":9,"method":"com.example.lib.mut"}' })
    fire('hew://api-frame', { connId: 5, frame: '{"jsonrpc":"2.0","id":1,"method":"hew.query.scene"}' })
    scene.busy.mockReturnValue(false)
    scene.api_complete_forward.mockReturnValue('{"jsonrpc":"2.0","id":9,"error":{"code":-32000}}')
    scene.api_dispatch.mockReturnValueOnce(JSON.stringify({ pending: { token: 4, forward: mutatingForward } }))
    scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":1,"result":{}}'))
    setBusy(null)
    // The harness command ended: 3's command starts (busy again) and 5,
    // still waiting, gets a fresh clock; when 3's command ends, 5 runs,
    // announced once more.
    await vi.waitFor(() => {
      expect(scene.api_dispatch).toHaveBeenCalledTimes(2)
      expect(emitMock).toHaveBeenCalledWith('hew://api-reply', expect.objectContaining({ connId: 5 }))
    })
    const events = emitMock.mock.calls
      .map((c) => c as unknown as [string, { connId: number }])
      .filter(([event]) => event !== 'hew://api-queued')
      .map(([event, payload]) => `${event}:${payload.connId}`)
    expect(events).toEqual([
      'hew://api-started:3',
      'hew://api-started:5',
      'hew://api-reply:3',
      'hew://api-started:5',
      'hew://api-reply:5',
    ])
  })

  it('a frame queued on a connection that closes before the flush is never dispatched', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    fire('hew://api-connection-open', { connId: 5 })
    scene.busy.mockReturnValue(true)
    setBusy({ plugin: 'org.hew3d.harness', label: 'test', startedAt: 0, cancel: () => {} })
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":1,"method":"hew.attr.set","params":{"target":"document","ns":"a","key":"k","value":1}}',
    })
    fire('hew://api-frame', { connId: 5, frame: '{"jsonrpc":"2.0","id":2,"method":"hew.query.scene"}' })
    // The shell's clock ran out on connection 3: it closes.
    fire('hew://api-connection-close', { connId: 3 })
    scene.busy.mockReturnValue(false)
    scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":2,"result":{}}'))
    setBusy(null)
    expect(scene.api_dispatch).toHaveBeenCalledTimes(1)
    expect(scene.api_dispatch.mock.calls[0][1]).toContain('hew.query.scene')
  })

  it('a frame queued behind a compound someone else holds runs once the document is free', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    scene.busy.mockReturnValue(true)
    setBusy({ plugin: 'org.hew3d.harness', label: 'test', startedAt: 0, cancel: () => {} })
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","id":1,"method":"hew.query.scene"}' })
    expect(scene.api_dispatch).not.toHaveBeenCalled()
    scene.busy.mockReturnValue(false)
    scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":1,"result":{}}'))
    setBusy(null)
    expect(scene.api_dispatch).toHaveBeenCalledTimes(1)
    await vi.waitFor(() => {
      expect(emitMock).toHaveBeenCalledWith('hew://api-started', { connId: 3 })
    })
  })

  it('refreshes after a forward by the plugin command\'s own flag, not the envelope\'s', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    const forwardOf = (mutating: boolean) => ({
      token: 1,
      plugin: 'com.example.lib',
      local: mutating ? 'mut' : 'sum',
      method: mutating ? 'com.example.lib.mut' : 'com.example.lib.sum',
      request_id: 5,
      params: {},
      mutating,
      label: null,
      timeout_ms: 50000,
      concurrency: 'parallel',
      caller: { identity: 'live:7', plugin: null, app: null, forward: null },
    })
    const ok = '{"jsonrpc":"2.0","id":5,"result":{"results":[{}],"label":"x"}}'
    scene.api_complete_forward.mockReturnValue(ok)
    // A read-only plugin command inside a sole-command transaction: the
    // envelope's method mutates, the command does not.
    scene.api_dispatch.mockReturnValue(JSON.stringify({ pending: { token: 1, forward: forwardOf(false) } }))
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":5,"method":"hew.doc.transact","params":{"commands":[{"method":"com.example.lib.sum"}]}}',
    })
    await vi.waitFor(() => {
      expect(emitMock).toHaveBeenCalledWith('hew://api-reply', { connId: 3, frame: ok })
    })
    expect(refreshScene).not.toHaveBeenCalled()

    scene.api_dispatch.mockReturnValue(JSON.stringify({ pending: { token: 1, forward: forwardOf(true) } }))
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":5,"method":"hew.doc.transact","params":{"commands":[{"method":"com.example.lib.mut"}]}}',
    })
    await vi.waitFor(() => {
      expect(refreshScene).toHaveBeenCalledTimes(1)
    })
  })

  it('answers an internal fault if a forward cannot be completed', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    scene.api_dispatch.mockReturnValue(JSON.stringify({ pending: { token: 1, forward: { request_id: 'q' } } }))
    scene.api_complete_forward.mockImplementation(() => {
      throw new Error('FORWARD: unknown forward token')
    })
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","id":"q","method":"com.example.lib.sum"}' })
    await vi.waitFor(() => {
      expect(emitMock).toHaveBeenCalledWith('hew://api-reply', {
        connId: 3,
        frame: '{"jsonrpc":"2.0","id":"q","error":{"code":-32003,"message":"the forward could not be completed"}}',
      })
    })
    expect(error).toHaveBeenCalled()
    error.mockRestore()
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
    scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":0,"result":{}}'))
    fire('hew://api-frame', { connId: 3, frame: '{"jsonrpc":"2.0","id":0,"method":"hew.meta.hello"}' })

    expect(scene.api_connection_open).toHaveBeenCalledTimes(1)
    expect(scene.api_dispatch).toHaveBeenCalledWith(7, expect.stringContaining('hew.meta.hello'))

    // A second frame for the same connId must not lazy-open a second time.
    scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":1,"result":{}}'))
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
    scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":2,"result":{"sketch":"skt_1"}}'))
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":2,"method":"hew.sketch.draw_rect","params":{}}',
    })
    await vi.waitFor(() => {
      expect(refreshScene).toHaveBeenCalledTimes(1)
    })
    expect(reconcile).not.toHaveBeenCalled()
  })

  it('re-applies tag visibility after a successful mutating dispatch, after the refresh', async () => {
    // hew.tag.assign can give a node an already-hidden tag; the viewport
    // refresh only re-applies the hidden set it already had.
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":2,"result":{}}'))
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":2,"method":"hew.tag.assign","params":{}}',
    })
    await vi.waitFor(() => {
      expect(reconcileVisibility).toHaveBeenCalledTimes(1)
    })
    expect(refreshScene.mock.invocationCallOrder[0]).toBeLessThan(
      reconcileVisibility.mock.invocationCallOrder[0],
    )
  })

  it('marks the document unsaved when a Library material item lands in the palette', async () => {
    const markLibraryMaterialsDirty = vi.fn<() => void>()
    installLiveBridge({ ...deps, markLibraryMaterialsDirty })
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    scene.api_dispatch.mockReturnValue(
      tagged('{"jsonrpc":"2.0","id":2,"result":{"label":"hew.library.insert","results":[{"roots":[],"materials":["mat_4"],"materials_added":1}]}}'),
    )
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":2,"method":"hew.library.insert","params":{}}',
    })
    await vi.waitFor(() => {
      expect(markLibraryMaterialsDirty).toHaveBeenCalledTimes(1)
    })
  })

  it('does not mark unsaved for an ordinary insert that places geometry', async () => {
    const markLibraryMaterialsDirty = vi.fn<() => void>()
    installLiveBridge({ ...deps, markLibraryMaterialsDirty })
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    scene.api_dispatch.mockReturnValue(
      tagged('{"jsonrpc":"2.0","id":2,"result":{"label":"hew.library.insert","results":[{"roots":["ins_9"],"materials_added":0}]}}'),
    )
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":2,"method":"hew.library.insert","params":{}}',
    })
    await vi.waitFor(() => {
      expect(refreshScene).toHaveBeenCalledTimes(1)
    })
    expect(markLibraryMaterialsDirty).not.toHaveBeenCalled()
  })

  it('falls back to reconcile() when no Viewport is mounted', async () => {
    deps.getViewportApi = () => null
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":2,"result":{}}'))
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
    scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":2,"result":{"tree":[]}}'))
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":2,"method":"hew.query.scene","params":{}}',
    })
    await flush()
    expect(refreshScene).not.toHaveBeenCalled()
    expect(reconcileVisibility).not.toHaveBeenCalled()
  })

  it('does not refresh after a dry run, however the reply reads', async () => {
    // A rehearsal answers success for a mutating envelope; only the tag
    // says nothing changed.
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    scene.api_dispatch.mockReturnValue(
      JSON.stringify({
        reply: '{"jsonrpc":"2.0","id":2,"result":{"label":"Box","results":[{}],"dry_run":true}}',
        dry_run: true,
      }),
    )
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":2,"method":"hew.doc.transact","params":{"dry_run":true,"commands":[]}}',
    })
    await vi.waitFor(() => {
      expect(emitMock).toHaveBeenCalled()
    })
    expect(refreshScene).not.toHaveBeenCalled()
    expect(reconcile).not.toHaveBeenCalled()
    expect(reconcileVisibility).not.toHaveBeenCalled()
  })

  it('does not refresh after a refused mutating command', async () => {
    installLiveBridge(deps)
    await flush()
    fire('hew://api-connection-open', { connId: 3 })
    scene.api_dispatch.mockReturnValue(
      tagged('{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"refused"}}'),
    )
    fire('hew://api-frame', {
      connId: 3,
      frame: '{"jsonrpc":"2.0","id":2,"method":"hew.solid.push_pull","params":{}}',
    })
    await flush()
    expect(refreshScene).not.toHaveBeenCalled()
    expect(reconcileVisibility).not.toHaveBeenCalled()
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
  // `Scene::take_pending_view_directives` to hand this bridge right after
  // dispatch. These prove the bridge actually invokes the SAME viewport
  // calls the Camera menu/toolbar and View menu already use.
  describe('hew.view.* directives', () => {
    it('applies an explicit camera through Viewport.setCamera', async () => {
      installLiveBridge(deps)
      await flush()
      fire('hew://api-connection-open', { connId: 3 })
      scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":2,"result":{}}'))
      scene.take_pending_view_directives.mockReturnValue(
        JSON.stringify([{
          kind: 'camera',
          camera: { eye: [1, 2, 3], target: [0, 0, 0], up: [0, 0, 1], fov_deg: 50 },
        }]),
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
      scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":2,"result":{}}'))
      scene.take_pending_view_directives.mockReturnValue(
        JSON.stringify([{ kind: 'camera', camera: { eye: [5, 0, 0], target: [0, 0, 0] } }]),
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
      scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":2,"result":{}}'))
      scene.take_pending_view_directives.mockReturnValue(
        JSON.stringify([{ kind: 'camera', view: 'top' }]),
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
      scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":2,"result":{}}'))
      scene.take_pending_view_directives.mockReturnValue(JSON.stringify([{ kind: 'zoom_extents' }]))
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
      scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":2,"result":{}}'))
      scene.take_pending_view_directives.mockReturnValue(JSON.stringify([{ kind: 'zoom_extents' }]))
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
      scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":2,"result":{"tree":[]}}'))
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
        scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":2,"result":{}}'))
        scene.take_pending_view_directives.mockReturnValue(
          JSON.stringify([{ kind: 'units', format: next }]),
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

  // Selection and view state (docs/agents/HEW_API.md §7): every directive
  // a dispatch left is applied, in order, through the app's own handlers.
  describe('selection and view-state directives', () => {
    const dispatchWith = async (directives: unknown[], method = 'hew.doc.transact'): Promise<void> => {
      installLiveBridge(deps)
      await flush()
      fire('hew://api-connection-open', { connId: 3 })
      scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":2,"result":{}}'))
      scene.take_pending_view_directives.mockReturnValue(JSON.stringify(directives))
      fire('hew://api-frame', {
        connId: 3,
        frame: `{"jsonrpc":"2.0","id":2,"method":"${method}","params":{}}`,
      })
    }

    it('selection: nodes and sketches replace the selection through selectMany', async () => {
      const selectMany = vi.fn()
      deps.selectMany = selectMany
      await dispatchWith(
        [
          {
            kind: 'selection',
            mode: 'replace',
            entries: [
              { kind: 'object', id: '4294967297' },
              { kind: 'group', id: '3' },
              { kind: 'instance', id: '9007199254740993' },
              { kind: 'sketch', id: '5' },
            ],
          },
        ],
        'hew.selection.set',
      )
      expect(selectMany).toHaveBeenCalledWith(
        [
          { kind: 'object', id: 4294967297n },
          { kind: 'group', id: 3n },
          // Past 2^53: the decimal string keeps every bit.
          { kind: 'instance', id: 9007199254740993n },
          { kind: 'sketch', id: 5n },
        ],
        'replace',
      )
      expect(refreshScene).not.toHaveBeenCalled()
    })

    it('selection: one guide, one annotation, or nothing go to their own handlers', async () => {
      const selectGuide = vi.fn()
      const selectAnnotation = vi.fn()
      const selectNone = vi.fn()
      const selectMany = vi.fn()
      Object.assign(deps, { selectGuide, selectAnnotation, selectNone, selectMany })
      await dispatchWith(
        [
          { kind: 'selection', mode: 'replace', entries: [{ kind: 'guide', id: '12' }] },
          { kind: 'selection', mode: 'replace', entries: [{ kind: 'annotation', id: '13' }] },
          { kind: 'selection', mode: 'replace', entries: [] },
        ],
        'hew.selection.set',
      )
      expect(selectGuide).toHaveBeenCalledWith(12n)
      expect(selectAnnotation).toHaveBeenCalledWith(13n)
      expect(selectNone).toHaveBeenCalledTimes(1)
      expect(selectMany).not.toHaveBeenCalled()
    })

    it('hidden_changed re-reads the hidden state before reconciling tags', async () => {
      const order: string[] = []
      reseedViewState.mockImplementation(() => order.push('reseed'))
      reconcileVisibility.mockImplementation(() => order.push('reconcile'))
      await dispatchWith([{ kind: 'hidden_changed' }], 'hew.view.hide')
      expect(order).toEqual(['reseed', 'reconcile'])
      // View state, not a document mutation: no refresh.
      expect(refreshScene).not.toHaveBeenCalled()
    })

    it('isolate frames the targets through ViewportApi.isolate', async () => {
      await dispatchWith(
        [
          {
            kind: 'isolate',
            frame: true,
            entries: [
              { kind: 'object', id: '7' },
              { kind: 'member', object: '8', instance: '9' },
            ],
          },
        ],
        'hew.view.isolate',
      )
      expect(viewportApi.isolate).toHaveBeenCalledWith(
        [
          { kind: 'object', id: 7n },
          { kind: 'member', object: 8n, instance: 9n },
        ],
        { frame: true },
      )
    })

    it('isolate goes through the app\'s own handler when one is wired', async () => {
      const isolate = vi.fn()
      deps.isolate = isolate
      await dispatchWith(
        [{ kind: 'isolate', frame: true, entries: [{ kind: 'instance', id: '4' }] }],
        'hew.view.isolate',
      )
      expect(isolate).toHaveBeenCalledWith([{ kind: 'instance', id: 4n }], { frame: true })
      expect(viewportApi.isolate).not.toHaveBeenCalled()
    })

    it('clear_isolation and zoom_to reach the viewport', async () => {
      await dispatchWith(
        [
          { kind: 'clear_isolation' },
          { kind: 'zoom_to', min: [-1, -2, -3], max: [4, 5, 6], padding: 0.5 },
        ],
        'hew.doc.transact',
      )
      expect(viewportApi.clearIsolation).toHaveBeenCalledTimes(1)
      expect(viewportApi.zoomToWorldBounds).toHaveBeenCalledWith([-1, -2, -3], [4, 5, 6])
    })

    it('enter_context enters each path instance outermost first, then the leaf', async () => {
      const enterNode = vi.fn()
      deps.enterNode = enterNode
      await dispatchWith(
        [
          {
            kind: 'enter_context',
            path: [
              { kind: 'instance', id: '7' },
              { kind: 'instance', id: '9' },
            ],
            leaf: { kind: 'object', id: '3' },
          },
        ],
        'hew.view.enter',
      )
      expect(enterNode.mock.calls).toEqual([
        [{ kind: 'instance', id: 7n }],
        [{ kind: 'instance', id: 9n }, { keepFallback: true }],
        [{ kind: 'object', id: 3n }, { keepFallback: true }],
      ])
    })

    it('exit_context pops to one crumb fewer, and exit_all_contexts to the top', async () => {
      const setPathDepth = vi.fn()
      deps.setPathDepth = setPathDepth
      deps.contextDepth = () => 3
      await dispatchWith([{ kind: 'exit_context' }, { kind: 'exit_all_contexts' }], 'hew.view.exit')
      expect(setPathDepth.mock.calls).toEqual([[2], [0]])
    })

    it('exit_context with nothing open pops nothing', async () => {
      const setPathDepth = vi.fn()
      deps.setPathDepth = setPathDepth
      deps.contextDepth = () => 0
      await dispatchWith([{ kind: 'exit_context' }], 'hew.view.exit')
      expect(setPathDepth).not.toHaveBeenCalled()
    })

    it('tells the scene whether a gesture is armed right before the dispatch', async () => {
      viewportApi.hasArmedGesture.mockReturnValue(true)
      await dispatchWith([], 'hew.view.enter')
      expect(scene.set_ui_gesture_armed).toHaveBeenLastCalledWith(true)
      expect(scene.set_ui_gesture_armed.mock.invocationCallOrder[0]).toBeLessThan(
        scene.api_dispatch.mock.invocationCallOrder[0],
      )
    })

    it('tells the scene a gesture is not armed once it has ended', async () => {
      viewportApi.hasArmedGesture.mockReturnValue(false)
      await dispatchWith([], 'hew.view.enter')
      expect(scene.set_ui_gesture_armed).toHaveBeenLastCalledWith(false)
    })

    it('applies several directives in the order the dispatch left them', async () => {
      const order: string[] = []
      viewportApi.isolate.mockImplementation(() => order.push('isolate'))
      viewportApi.zoomToWorldBounds.mockImplementation(() => order.push('zoom_to'))
      viewportApi.clearIsolation.mockImplementation(() => order.push('clear'))
      reseedViewState.mockImplementation(() => order.push('reseed'))
      deps.selectNone = () => order.push('select')
      await dispatchWith([
        { kind: 'hidden_changed' },
        { kind: 'isolate', entries: [{ kind: 'object', id: '1' }] },
        { kind: 'selection', mode: 'replace', entries: [] },
        { kind: 'zoom_to', min: [0, 0, 0], max: [1, 1, 1], padding: 0.1 },
        { kind: 'clear_isolation' },
      ])
      expect(order).toEqual(['reseed', 'isolate', 'select', 'zoom_to', 'clear'])
    })

    it('a viewport directive with no mounted Viewport is a silent no-op', async () => {
      deps.getViewportApi = () => null
      await expect(
        dispatchWith([
          { kind: 'isolate', entries: [{ kind: 'object', id: '1' }] },
          { kind: 'clear_isolation' },
          { kind: 'zoom_to', min: [0, 0, 0], max: [1, 1, 1], padding: 0.1 },
        ]),
      ).resolves.toBeUndefined()
    })
  })

  // hew.scenes.apply: unlike hew.view.*, this is a KERNEL-served,
  // mutates_document = true command — it already refreshes the viewport
  // through the normal mutation path above. The ActivateScene directive
  // is a SEPARATE, additive signal so the app can drive its own
  // Scene-activation UI (camera tween, panel/outliner sync) through
  // `LiveBridgeDeps.activateScene` — see that dep's own doc comment in
  // liveBridge.ts.
  describe('hew.scenes.apply directive', () => {
    it('calls deps.activateScene with the Scene sid', async () => {
      const activateScene = vi.fn()
      deps.activateScene = activateScene
      installLiveBridge(deps)
      await flush()
      fire('hew://api-connection-open', { connId: 3 })
      scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":2,"result":{}}'))
      scene.take_pending_view_directives.mockReturnValue(
        JSON.stringify([{ kind: 'activate_scene', sid: 7 }]),
      )
      fire('hew://api-frame', {
        connId: 3,
        frame: '{"jsonrpc":"2.0","id":2,"method":"hew.scenes.apply","params":{"id":"scene_7"}}',
      })
      expect(activateScene).toHaveBeenCalledWith(7)
      // Never a viewport call — activation is not a camera directive.
      expect(viewportApi.setCamera).not.toHaveBeenCalled()
      // The activation applies the Scene's hidden nodes and tags itself; a
      // tag reconcile first would push against the pre-activation hides.
      expect(reconcileVisibility).not.toHaveBeenCalled()
    })

    it('is a silent no-op when activateScene is not wired', async () => {
      installLiveBridge(deps)
      await flush()
      fire('hew://api-connection-open', { connId: 3 })
      scene.api_dispatch.mockReturnValue(tagged('{"jsonrpc":"2.0","id":2,"result":{}}'))
      scene.take_pending_view_directives.mockReturnValue(
        JSON.stringify([{ kind: 'activate_scene', sid: 7 }]),
      )
      expect(() =>
        fire('hew://api-frame', {
          connId: 3,
          frame: '{"jsonrpc":"2.0","id":2,"method":"hew.scenes.apply","params":{"id":"scene_7"}}',
        }),
      ).not.toThrow()
      // No activation to own visibility here, so tags are still reconciled.
      expect(reconcileVisibility).toHaveBeenCalledTimes(1)
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

describe('dispatchFrameNow', () => {
  it('dispatches, applies the directives in order, and answers the reply', () => {
    const selectMany = vi.fn()
    const reseedViewState = vi.fn()
    const scene = {
      set_display_units: vi.fn(),
      set_ui_gesture_armed: vi.fn(),
      api_dispatch: vi.fn(() => tagged('{"jsonrpc":"2.0","id":4,"result":{"hidden":[]}}')),
      api_method_mutates: vi.fn(() => false),
      take_pending_view_directives: vi.fn(() =>
        JSON.stringify([
          { kind: 'hidden_changed' },
          { kind: 'selection', mode: 'replace', entries: [{ kind: 'object', id: '2' }] },
        ]),
      ),
      drain_events: vi.fn(() => '[]'),
    }
    const reply = dispatchFrameNow(
      {
        getScene: () => scene as unknown as Scene,
        getViewportApi: () => null,
        reconcile: vi.fn(),
        reseedViewState,
        selectMany,
      },
      scene as unknown as Scene,
      5,
      '{"jsonrpc":"2.0","id":4,"method":"hew.view.hide","params":{"ids":[]}}',
    )
    expect(reply).toBe('{"jsonrpc":"2.0","id":4,"result":{"hidden":[]}}')
    expect(scene.api_dispatch).toHaveBeenCalledWith(5, expect.stringContaining('hew.view.hide'))
    expect(reseedViewState).toHaveBeenCalledTimes(1)
    expect(selectMany).toHaveBeenCalledWith([{ kind: 'object', id: 2n }], 'replace')
    expect(scene.take_pending_view_directives).toHaveBeenCalledTimes(1)
  })

  it('does not refresh after a dry run', () => {
    const reconcile = vi.fn()
    const reconcileVisibility = vi.fn()
    const scene = {
      set_display_units: vi.fn(),
      set_ui_gesture_armed: vi.fn(),
      api_dispatch: vi.fn(() =>
        JSON.stringify({
          reply: '{"jsonrpc":"2.0","id":4,"result":{"label":"Box","results":[{}],"dry_run":true}}',
          dry_run: true,
        }),
      ),
      api_method_mutates: vi.fn(() => true),
      take_pending_view_directives: vi.fn(() => '[]'),
      drain_events: vi.fn(() => '[]'),
    }
    const reply = dispatchFrameNow(
      {
        getScene: () => scene as unknown as Scene,
        getViewportApi: () => null,
        reconcile,
        reconcileVisibility,
      },
      scene as unknown as Scene,
      5,
      '{"jsonrpc":"2.0","id":4,"method":"hew.doc.transact","params":{"dry_run":true,"commands":[]}}',
    )
    expect(reply).toContain('"dry_run":true')
    expect(reconcile).not.toHaveBeenCalled()
    expect(reconcileVisibility).not.toHaveBeenCalled()
  })
})
