/**
 * The webview half of `--live` (docs/agents/HEW_API.md §11.2, §12): forwards
 * newline-delimited JSON-RPC frames the Tauri shell reads off its local
 * socket, dispatches each against the SAME live kernel `Document` the
 * viewport renders — via `Scene.api_dispatch`
 * (crates/wasm-api/src/live.rs) — and sends the reply back. This module
 * owns exactly the transport plumbing (Tauri events) and the
 * refresh-after-mutation contract; the protocol itself (hello, profile
 * enforcement, transactions, refusals) lives entirely in `crates/api` and
 * is opaque here — every frame/reply is treated as raw JSON-RPC text.
 *
 * Wire contract with the Rust shell
 * (`shells/tauri/src-tauri/src/live.rs`, whose own doc comment is the
 * Rust-facing half of this same contract):
 *
 *   Rust -> JS  'hew://api-connection-open'  { connId }
 *   Rust -> JS  'hew://api-connection-close' { connId }
 *   Rust -> JS  'hew://api-frame'            { connId, frame }
 *   JS -> Rust  'hew://api-reply'            { connId, frame }
 *
 * `connId` is the SHELL's id for one accepted socket connection — this
 * module keeps its own map to whatever id `Scene.api_connection_open()`
 * mints for it (`crates/wasm-api` self-assigns; there is no reason for
 * the two numbering spaces to be the same one, and tying them together
 * would mean changing an already-tested WASM surface for no functional
 * gain). Every event is window-scoped (the shell resolves one target
 * window per connection at accept time and `emit_to`s only that window —
 * see `live.rs`'s `spawn_accept_loop` doc comment), so this listens on
 * `getCurrentWebviewWindow()`, the same convention `App.tsx` already uses
 * for `menu-action`/`menu-open-path`.
 *
 * There is no module-level `Scene` singleton (`App.tsx` owns it in a
 * `useRef`, set once the WASM module loads) — this module is wired in
 * from `App.tsx` exactly like `test/harness.ts`'s `installTestHarness`,
 * with live accessor functions instead of captured values so it never
 * goes stale across re-renders.
 */

import { isTauri } from '../io/fileHost'
import type { Scene } from '../wasm/loader'
import type { StandardView, ViewportApi } from '../viewport/Viewport'
import { setLengthUnit, type LengthFormat } from '../settings/units'

export interface LiveBridgeDeps {
  getScene: () => Scene | null
  getViewportApi: () => ViewportApi | null
  /** Reconcile + re-render after a mutation when no `Viewport` is mounted
   * — mirrors `HarnessDeps.reconcile` (`test/harness.ts`), the app's own
   * document-changed path. */
  reconcile: () => void
  /**
   * Applies a Scene the live app just activated via `hew.scenes.apply`
   * (`ViewDirective::ActivateScene`, crates/wasm-api/src/live.rs) — the
   * kernel-side state (hidden nodes/tags, section plane) is already
   * written by the time this fires; `sid` names which Scene so this can
   * drive the same app-side activation path Scene Tray uses (camera
   * tween, panel/outliner sync).
   *
   * Optional (a host without a Scenes UI omits it — an `ActivateScene`
   * directive is then a no-op here, harmless since the kernel state it
   * names is already written); App.tsx wires it to
   * `useScenesController`'s `activate`.
   */
  activateScene?: (sid: number) => void
}

/** Whether a dispatch of `method` could have changed the document, per
 * the command registry (`Scene.api_method_mutates` → `crates/api`) — the
 * same fact `Scene::api_dispatch` uses to decide whether to re-sync and
 * record, so the viewport and the kernel can never disagree.
 *
 * Deliberately NOT a method-name convention. Matching on `hew.query.*` /
 * `hew.meta.*` happens to be right for today's registry, but a future
 * mutating command outside that shape would skip the refresh and leave
 * the user looking at a document that no longer exists — a silent stale
 * viewport, with no error anywhere. */
function methodMutates(scene: Scene, method: string): boolean {
  return scene.api_method_mutates(method)
}

/** `crates/api/src/envelope.rs`'s `codes::NOT_READY` — mirrored here (this
 * module has no `crates/api` binding, only raw JSON-RPC text) for the one
 * "not ready" case that is genuinely local to the webview: the WASM
 * `Scene` hasn't finished loading yet when a frame arrives, so there is
 * nothing at all to dispatch it through. */
const NOT_READY_CODE = -32004

/** Builds a `-32004 not ready` JSON-RPC error reply for `frame`, or
 * `undefined` if `frame` is a notification (no `id`) — per §4.1, a
 * notification never gets a reply, not even an error one. A frame that
 * fails to parse as JSON is treated as `id: null`, the same convention
 * `hew-cli mcp`'s own parse-error handling uses (there is no way to
 * correlate a truly malformed frame to a request, but it still deserves
 * an answer rather than silent drop). */
function notReadyReply(frame: string): string | undefined {
  let id: unknown = null
  try {
    const parsed = JSON.parse(frame) as { id?: unknown }
    if (!('id' in parsed)) return undefined // a notification: no reply, per §4.1
    id = parsed.id
  } catch {
    id = null
  }
  return JSON.stringify({
    jsonrpc: '2.0',
    id,
    error: {
      code: NOT_READY_CODE,
      message: 'not ready: the live connection is still starting up',
    },
  })
}

/**
 * Whether a dispatched `frame`/`reply` pair warrants a viewport refresh:
 * the method wasn't one of the read-only ones above, AND the reply
 * actually succeeded (a refusal leaves the document byte-identical,
 * docs/agents/HEW_API.md §14 — refreshing after one would be pure waste). Pure
 * and exported for direct unit testing; malformed input on either side
 * (should not happen — both come from this module's own dispatch call)
 * conservatively answers false rather than guess.
 */
export function shouldRefreshAfterDispatch(scene: Scene, frame: string, reply: string): boolean {
  let method: unknown
  try {
    method = (JSON.parse(frame) as { method?: unknown }).method
  } catch {
    return false
  }
  if (typeof method !== 'string' || !methodMutates(scene, method)) return false
  try {
    return (JSON.parse(reply) as { error?: unknown }).error === undefined
  } catch {
    return false
  }
}

/**
 * The wire shape of `crates/wasm-api/src/live.rs`'s `ViewDirective`,
 * JSON-encoded by `Scene::take_pending_view_directive` — one WASM-internal
 * side channel local to this process (never the JSON-RPC reply itself; see
 * that Rust type's doc comment for why) through which `hew.view.camera`/
 * `zoom_extents`/`units` hand their effect to this bridge, since none of
 * them is reachable from inside the WASM sandbox on its own (no DOM
 * access). `camera`/`view`'s fields mirror `hew.view.snapshot`/
 * `hew.view.camera`'s own wire params exactly (docs/agents/HEW_API.md §7) —
 * the same vocabulary, never a second dialect.
 */
type ViewDirective =
  | {
      kind: 'camera'
      camera?: {
        eye: [number, number, number]
        target: [number, number, number]
        up?: [number, number, number]
        projection?: 'perspective' | 'parallel'
        fov_deg?: number
      }
      view?: string
    }
  | { kind: 'zoom_extents' }
  | { kind: 'units'; format: LengthFormat }
  | { kind: 'activate_scene'; sid: number }

/** `hew.view.camera`'s own defaults for an explicit camera's optional
 * fields (docs/agents/HEW_API.md §7, `crates/api/src/host.rs`'s `SnapshotCamera`
 * doc comment: "identity up `[0,0,1]`, perspective, 35°") — resolved here
 * rather than in Rust because THIS is the "host" that builds the actual
 * camera for a live connection (`crates/wasm-api`'s `LiveHost` has no
 * renderer of its own to resolve them against). */
const DEFAULT_CAMERA_UP: [number, number, number] = [0, 0, 1]
const DEFAULT_CAMERA_PROJECTION = 'perspective' as const
const DEFAULT_CAMERA_FOV_DEG = 35

/**
 * Applies one `ViewDirective` — the live-viewport/app-settings half of
 * `hew.view.camera`/`zoom_extents`/`units` that `LiveHost` (Rust) cannot
 * perform itself. Routes through the SAME calls the app's own UI already
 * makes: `ViewportApi.setCamera`/`setStandardView`/`zoomExtents` (Camera
 * menu, toolbar, View menu) and `app/src/settings/units.ts`'s
 * `setLengthUnit` (Settings window) — never a second camera or
 * units-writing path.
 *
 * A `camera`/`zoom_extents` directive with no mounted `Viewport` (no
 * window has rendered one yet, an extremely narrow startup race in
 * practice — the desktop app's main window always mounts one) is a
 * silent no-op: `hew.view.camera` already succeeded on the Rust side
 * (there is no document mutation to roll back), and there is no sensible
 * fallback for "aim a viewport that does not exist" the way
 * `refreshAfterMutation` has one (`deps.reconcile()`) for a stale
 * document. `units`, unlike the other two, never touches the viewport at
 * all, so it is unaffected by this case.
 */
function applyViewDirective(
  directive: ViewDirective,
  viewportApi: ViewportApi | null,
  activateScene: ((sid: number) => void) | undefined,
): void {
  switch (directive.kind) {
    case 'camera':
      if (viewportApi === null) return
      if (directive.camera !== undefined) {
        const c = directive.camera
        viewportApi.setCamera(
          c.eye,
          c.target,
          c.up ?? DEFAULT_CAMERA_UP,
          c.fov_deg ?? DEFAULT_CAMERA_FOV_DEG,
          c.projection ?? DEFAULT_CAMERA_PROJECTION,
        )
      } else if (directive.view !== undefined) {
        viewportApi.setStandardView(directive.view as StandardView)
      }
      return
    case 'zoom_extents':
      viewportApi?.zoomExtents()
      return
    case 'units':
      setLengthUnit(directive.format)
      return
    case 'activate_scene':
      // Optional — see `LiveBridgeDeps.activateScene`'s doc.
      activateScene?.(directive.sid)
      return
  }
}

/** Parses and applies `directiveJson` (from `Scene.take_pending_view_directive`),
 * tolerating malformed input defensively (should never happen — the Rust
 * side is the only producer) by no-op rather than throwing into the Tauri
 * event handler. Exported for direct unit testing alongside
 * `shouldRefreshAfterDispatch`. */
export function applyPendingViewDirective(
  directiveJson: string,
  viewportApi: ViewportApi | null,
  activateScene?: (sid: number) => void,
): void {
  try {
    applyViewDirective(JSON.parse(directiveJson) as ViewDirective, viewportApi, activateScene)
  } catch {
    /* malformed directive JSON should never happen — defensive no-op */
  }
}

/**
 * Wires the live API bridge to this window's Tauri events. A no-op
 * outside Tauri (the web build never has a socket to speak to). Returns
 * an uninstall function (HMR / unmount), the same shape
 * `installTestHarness` returns.
 */
export function installLiveBridge(deps: LiveBridgeDeps): () => void {
  if (!isTauri) return () => {}

  // Shell connId -> the wasm-minted connection id for it. Entries are
  // added on 'hew://api-connection-open' (or, lazily, on the first
  // 'hew://api-frame' for a connId that raced ahead of its own wasm-side
  // open — see `handleFrame`) and removed on
  // 'hew://api-connection-close' (or if the frame handler ever finds the
  // Scene gone — a mid-session reload/unmount race).
  const wasmConnOf = new Map<number, number>()

  // Every shell connId the shell has told us about via
  // 'hew://api-connection-open', whether or not the Scene was ready to
  // open its wasm side at the time — the set `handleFrame` consults to
  // decide "lazily open" (a connId the shell genuinely accepted, just
  // racing the Scene's own startup) from "never opened at all" (a connId
  // that should never dispatch, shell bug or otherwise). Cleared on
  // 'hew://api-connection-close', same lifetime as `wasmConnOf`.
  const openConnIds = new Set<number>()

  let cancelled = false
  let unlistenOpen: (() => void) | undefined
  let unlistenClose: (() => void) | undefined
  let unlistenFrame: (() => void) | undefined
  let tauriEmit: ((event: string, payload?: unknown) => Promise<void>) | null = null

  const emitReply = (connId: number, frame: string): void => {
    const payload = { connId, frame }
    if (tauriEmit !== null) {
      tauriEmit('hew://api-reply', payload).catch(() => {
        /* the connection is gone — the shell's own reply-timeout covers it */
      })
      return
    }
    import('@tauri-apps/api/event')
      .then(({ emit }) => {
        tauriEmit = emit
        return emit('hew://api-reply', payload)
      })
      .catch(() => {
        /* ignore — not in Tauri, or emission failed */
      })
  }

  const refreshAfterMutation = (): void => {
    const api = deps.getViewportApi()
    if (api !== null) api.refreshScene()
    else deps.reconcile()
  }

  const handleOpen = (connId: number): void => {
    openConnIds.add(connId)
    const scene = deps.getScene()
    if (scene === null) return // not ready yet — handleFrame lazily opens on the first frame instead
    wasmConnOf.set(connId, scene.api_connection_open())
  }

  const handleClose = (connId: number): void => {
    openConnIds.delete(connId)
    const wasmId = wasmConnOf.get(connId)
    wasmConnOf.delete(connId)
    if (wasmId === undefined) return
    deps.getScene()?.api_connection_close(wasmId)
  }

  const handleFrame = (connId: number, frame: string): void => {
    if (!openConnIds.has(connId)) return // never accepted by the shell — never dispatched
    const scene = deps.getScene()
    if (scene === null) {
      // The Scene hasn't finished loading yet — a startup race between
      // the shell accepting a socket connection and the webview's WASM
      // module coming up. There used to be nothing this could do but
      // drop the frame silently, permanently: `handleOpen` never retried,
      // so a connection accepted this early was dead for its whole
      // lifetime, hello included. Answer honestly instead — the client
      // gets a typed "not ready" now rather than waiting out the shell's
      // own reply timeout for every single frame.
      const reply = notReadyReply(frame)
      if (reply !== undefined) emitReply(connId, reply)
      return
    }
    let wasmId = wasmConnOf.get(connId)
    if (wasmId === undefined) {
      // Lazy-open: `handleOpen` saw this connId before the Scene was
      // ready and could only record it in `openConnIds`, not open its
      // wasm side. Open it now, on the first frame that actually needs
      // it — idempotent, since `wasmConnOf` is checked first, so a connId
      // that DID open eagerly in `handleOpen` never opens twice.
      wasmId = scene.api_connection_open()
      wasmConnOf.set(connId, wasmId)
    }
    const reply = scene.api_dispatch(wasmId, frame)
    if (reply === undefined) return // a notification: no reply, per §4.1
    emitReply(connId, reply)
    if (shouldRefreshAfterDispatch(scene, frame, reply)) refreshAfterMutation()
    // `hew.view.camera`/`zoom_extents`/`units`: a non-mutating host effect
    // `Scene::api_dispatch` left for this bridge to actually perform
    // (`take_pending_view_directive`'s own doc comment has the full
    // story). `undefined` for every other dispatch, including a refused
    // view command.
    const directiveJson = scene.take_pending_view_directive()
    if (directiveJson !== undefined) {
      applyPendingViewDirective(directiveJson, deps.getViewportApi(), deps.activateScene)
    }
  }

  import('@tauri-apps/api/webviewWindow')
    .then(({ getCurrentWebviewWindow }) => {
      const win = getCurrentWebviewWindow()
      return Promise.all([
        win.listen<{ connId: number }>('hew://api-connection-open', (event) => {
          handleOpen(event.payload.connId)
        }),
        win.listen<{ connId: number }>('hew://api-connection-close', (event) => {
          handleClose(event.payload.connId)
        }),
        win.listen<{ connId: number; frame: string }>('hew://api-frame', (event) => {
          handleFrame(event.payload.connId, event.payload.frame)
        }),
      ])
    })
    .then(([openFn, closeFn, frameFn]) => {
      if (cancelled) {
        openFn()
        closeFn()
        frameFn()
        return
      }
      unlistenOpen = openFn
      unlistenClose = closeFn
      unlistenFrame = frameFn
    })
    .catch(() => {
      /* not in Tauri (shouldn't happen — gated by isTauri above), or registration failed */
    })

  return () => {
    cancelled = true
    unlistenOpen?.()
    unlistenClose?.()
    unlistenFrame?.()
  }
}
