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
 *   JS -> Rust  'hew://api-queued'           { connId }
 *   JS -> Rust  'hew://api-started'          { connId }
 *   Rust -> JS  'hew://api-abort'            { connId }
 *   JS -> Rust  'hew://api-event'            { connId, frame }
 *   Rust -> JS  'hew://document-id'          { id }
 *
 * While a plugin command holds the document (`Scene.busy()`, a compound
 * is open; docs/agents/PLUGINS.md "Compounds"), a frame is queued instead
 * of dispatched (`api-queued`), and dispatched in arrival order once the
 * document is free again, each announced by `api-started` so the shell
 * restarts its reply clock then. Every plugin command that finishes while
 * frames wait restarts their clocks too (`api-started` again), so a client
 * behind a chain of commands is timed out only by one command that runs
 * longer than the clock, never by the length of the queue. `api-abort` is the shell's own timeout
 * giving up on a connection: the compound opened for its forward, if any,
 * is aborted, so a closed socket never leaves the document held.
 *
 * Events (docs/agents/HEW_API.md §4.5): after every dispatch, forward
 * completion, and abort the bridge pumps `Scene.drain_events`
 * (`eventPump.ts`); each frame owed to a socket connection goes back as
 * `api-event`, which the shell writes on that connection between replies.
 * `document-id` tells this window the id the shell gave its document, which
 * every event names.
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
import type { NodeRef, SelectMode } from '../panels/treeModel'
import type { IsolationTarget } from '../viewport/isolation'
import { getLengthUnit, setLengthUnit, type LengthFormat } from '../settings/units'
import { parseDispatch } from './dispatchResult'
import { getBusy, setBusy, subscribeBusy } from './busyState'
import { pumpEvents, registerBroadcastSink, registerEventSink, setPumpDocumentId } from './eventPump'

/** What a webview-side live operation answers (`hew://api-op-reply`'s body
 * beside its `opId`). `refusal` names a typed API refusal the shell turns
 * into the client's reply. `afterReply` runs once the answer is on its way —
 * for an operation whose success destroys this very window. */
export interface LiveOpOutcome {
  ok: boolean
  refusal?: string
  message?: string
  afterReply?: () => Promise<void>
}

/** Runs one shell-requested operation that needs this window's real
 * document state: `open` (`{path}`: load a file into this fresh window) and
 * `close` (`{discard}`: close this window, refusing `unsaved_changes`
 * unless discarding). Wire: Rust -> JS `hew://api-op` `{opId, op, args}`,
 * JS -> Rust `hew://api-op-reply` `{opId, ...LiveOpOutcome}`. */
export type LiveOpRunner = (op: string, args: Record<string, unknown>) => Promise<LiveOpOutcome>

export interface LiveBridgeDeps {
  getScene: () => Scene | null
  /** Writes the viewport's current camera into the document (the same step
   * Save does), so a `hew.doc.save` that hands bytes back carries "what the
   * user is looking at" — which is what a client-side snapshot of this
   * document then renders from when the request names no camera. Optional. */
  pushCameraState?: (scene: Scene) => void
  /** The webview half of the shell's window-management operations
   * (`hew.doc.open`/`close`) — optional; absent answers every op as
   * unsupported. */
  runOp?: LiveOpRunner
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
  /**
   * Recompute the hidden set (hidden tags from the document's registry,
   * plus hidden nodes and groups) and push it to the renderer and the
   * kernel's pick/snap exclusion, only if it actually changed. A live
   * command can assign a hidden tag, hide or show a tag, rename or delete
   * one, reparent a node into a hidden group, or undo any of those — and
   * the viewport refresh alone only re-applies the hidden set it already
   * had. Optional (a host without visibility state omits it); App.tsx
   * wires it to its `reconcileVisibility`.
   */
  reconcileVisibility?: () => void
  /**
   * Marks the document unsaved because materials came into its palette
   * from a Library material item (`hew.library.insert`). The palette rides
   * the document but records no undo entry, so the saved-mark check after
   * a mutation can't see it — the same non-undoable reason the Library's
   * own "Add to palette" stamps. Optional; App.tsx wires it.
   */
  markLibraryMaterialsDirty?: () => void
  /**
   * Re-reads the hidden state the app mirrors from the document (hidden
   * nodes, the tag registry) and pushes it to the renderer — what a load
   * does. Run after a plugin command is aborted: the abort restores the
   * whole document, view state included, so the mirrors are stale.
   * Optional; App.tsx wires it to `reseedHiddenStateFromScene`.
   */
  reseedViewState?: () => void
  /**
   * The app's own selection handlers, which a bus `hew.selection.set` or
   * `clear` drives (`ViewDirective::Selection`): nodes and sketches through
   * `handleSelectMany(nodes, 'replace')`, one guide, one annotation, or
   * nothing. Optional (a host without a selection omits them; the directive
   * is then a no-op). App.tsx wires them to its handlers.
   */
  selectMany?: (nodes: NodeRef[], mode: SelectMode) => void
  selectGuide?: (id: bigint) => void
  selectAnnotation?: (id: bigint) => void
  selectNone?: () => void
  /**
   * Isolates through the app's own handler (`isolate` directives): the
   * viewport's isolation, owned by the bus rather than a panel. Optional;
   * without it the bridge calls `ViewportApi.isolate` directly.
   */
  isolate?: (targets: IsolationTarget[], opts: { frame: boolean }) => void
}

/** Whether a successful reply added a Library material item to the
 * palette: `hew.library.insert`'s result carries a `materials` list only
 * for a material item, bare or as one entry of a transaction's results. */
export function addedLibraryMaterials(reply: string): boolean {
  try {
    const result = (JSON.parse(reply) as { result?: { results?: unknown } }).result
    if (result === undefined || result === null) return false
    const entries = Array.isArray(result.results) ? result.results : [result]
    return entries.some(
      (e) => typeof e === 'object' && e !== null && Array.isArray((e as { materials?: unknown }).materials),
    )
  } catch {
    return false
  }
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

/** `crates/api/src/envelope.rs`'s `codes::INTERNAL_FAULT`. */
const INTERNAL_FAULT_CODE = -32003

/** Completes a forward with the `unavailable` reply and returns the final
 * frame. `Scene.api_complete_forward` refuses only a token it never handed
 * out, which this bridge cannot produce; should it ever, the client still
 * gets an answer instead of waiting out the shell's reply timeout. */
export function completeForward(
  scene: Scene,
  wasmId: number,
  token: number,
  requestId: number | string,
  failed: 'unavailable' | 'cancelled' | 'timeout' = 'unavailable',
): string {
  try {
    return scene.api_complete_forward(wasmId, BigInt(token), JSON.stringify({ failed }))
  } catch (err) {
    console.error('Scene.api_complete_forward refused a forward this bridge was handed', err)
    return JSON.stringify({
      jsonrpc: '2.0',
      id: requestId,
      // oxlint-disable-next-line lingui/no-unlocalized-strings -- JSON-RPC error text for the developer API, which stays English
      error: { code: INTERNAL_FAULT_CODE, message: 'the forward could not be completed' },
    })
  }
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
      // oxlint-disable-next-line lingui/no-unlocalized-strings -- JSON-RPC refusal text for the developer API, which stays English
      message: 'not ready: the live connection is still starting up',
    },
  })
}

/** Whether a reply frame is a success (no `error`); malformed reads as no. */
function replySucceeded(reply: string): boolean {
  try {
    return (JSON.parse(reply) as { error?: unknown }).error === undefined
  } catch {
    return false
  }
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
  return replySucceeded(reply)
}

/** A selection entry as a `selection` directive carries it: the kernel
 * handle as a decimal string (`crates/wasm-api/src/live.rs`'s
 * `UiSelectionEntry`). */
type DirectiveSelectionEntry = {
  kind: 'object' | 'group' | 'instance' | 'sketch' | 'guide' | 'annotation'
  id: string
}

/** An isolation target as an `isolate` directive carries it
 * (`UiIsolationEntry`). */
type DirectiveIsolationEntry =
  | { kind: 'object' | 'group' | 'instance'; id: string }
  | { kind: 'member'; object: string; instance: string }

/**
 * The wire shape of `crates/wasm-api/src/live.rs`'s `ViewDirective`,
 * JSON-encoded by `Scene::take_pending_view_directives` (an array, in the
 * order the dispatch's commands ran) — one WASM-internal
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
        /** The height the view spans at the target, meters (a parallel view frames it). */
        parallel_height?: number
      }
      view?: string
    }
  | { kind: 'zoom_extents' }
  | { kind: 'units'; format: LengthFormat }
  | { kind: 'activate_scene'; sid: number }
  | { kind: 'selection'; entries: DirectiveSelectionEntry[]; mode: 'replace' }
  | { kind: 'hidden_changed' }
  | { kind: 'isolate'; entries: DirectiveIsolationEntry[]; frame: boolean }
  | { kind: 'clear_isolation' }
  | { kind: 'zoom_to'; min: [number, number, number]; max: [number, number, number]; padding: number }

/** `hew.view.camera`'s own defaults for an explicit camera's optional
 * fields (docs/agents/HEW_API.md §7, `crates/api/src/host.rs`'s `SnapshotCamera`
 * doc comment: "identity up `[0,0,1]`, perspective, 35°") — resolved here
 * rather than in Rust because THIS is the "host" that builds the actual
 * camera for a live connection (`crates/wasm-api`'s `LiveHost` has no
 * renderer of its own to resolve them against). */
const DEFAULT_CAMERA_UP: [number, number, number] = [0, 0, 1]
const DEFAULT_CAMERA_PROJECTION = 'perspective' as const
const DEFAULT_CAMERA_FOV_DEG = 35

/** What applying a directive reaches: the viewport and the app's own
 * handlers (a subset of `LiveBridgeDeps`). */
export type DirectiveTargets = Pick<
  LiveBridgeDeps,
  | 'activateScene'
  | 'reseedViewState'
  | 'reconcileVisibility'
  | 'selectMany'
  | 'selectGuide'
  | 'selectAnnotation'
  | 'selectNone'
  | 'isolate'
> & { viewportApi: ViewportApi | null }

/** A `selection` directive's merged entries, applied wholesale through the
 * app's handlers: nodes and sketches replace the selection, a lone guide or
 * annotation selects it, and nothing clears it. */
function applySelection(entries: DirectiveSelectionEntry[], targets: DirectiveTargets): void {
  if (entries.length === 0) {
    targets.selectNone?.()
    return
  }
  const [first] = entries
  if (entries.length === 1 && first.kind === 'guide') {
    targets.selectGuide?.(BigInt(first.id))
    return
  }
  if (entries.length === 1 && first.kind === 'annotation') {
    targets.selectAnnotation?.(BigInt(first.id))
    return
  }
  const nodes: NodeRef[] = []
  for (const e of entries) {
    // The kernel never merges a guide or annotation with nodes; skip one
    // defensively rather than hand the node handler a kind it cannot hold.
    if (e.kind === 'guide' || e.kind === 'annotation') continue
    nodes.push({ kind: e.kind, id: BigInt(e.id) })
  }
  targets.selectMany?.(nodes, 'replace')
}

function isolationTargets(entries: DirectiveIsolationEntry[]): IsolationTarget[] {
  return entries.map((e) =>
    e.kind === 'member'
      ? { kind: 'member' as const, object: BigInt(e.object), instance: BigInt(e.instance) }
      : { kind: e.kind, id: BigInt(e.id) },
  )
}

/**
 * Applies one `ViewDirective` — the live-viewport/app half of a view
 * command that `LiveHost` (Rust) cannot perform itself. Routes through the
 * SAME calls the app's own UI already makes: `ViewportApi.setCamera`/
 * `setStandardView`/`zoomExtents`/`isolate`/`clearIsolation`/
 * `zoomToWorldBounds` (Camera menu, toolbar, View menu, Model Health),
 * `app/src/settings/units.ts`'s `setLengthUnit` (Settings window), the
 * app's selection handlers, and its hidden-state re-read — never a second
 * camera, units, selection, or visibility path.
 *
 * A viewport directive with no mounted `Viewport` (no window has rendered
 * one yet, an extremely narrow startup race in practice — the desktop
 * app's main window always mounts one) is a silent no-op: the command
 * already succeeded on the Rust side (there is no document mutation to
 * roll back), and there is no sensible fallback for "aim a viewport that
 * does not exist" the way `refreshAfterMutation` has one
 * (`deps.reconcile()`) for a stale document. `units` and the selection,
 * unlike the others, never touch the viewport at all.
 */
function applyViewDirective(directive: ViewDirective, targets: DirectiveTargets): void {
  const viewportApi = targets.viewportApi
  switch (directive.kind) {
    case 'camera':
      if (viewportApi === null) return
      if (directive.camera !== undefined) {
        const c = directive.camera
        if (c.parallel_height !== undefined && Number.isFinite(c.parallel_height) && c.parallel_height > 0) {
          // A camera that says what it spans at the target is posed whole,
          // the way a Scene's or a loaded document's camera is.
          viewportApi.tweenCameraState(
            {
              projection: c.projection ?? DEFAULT_CAMERA_PROJECTION,
              fovDeg: c.fov_deg ?? DEFAULT_CAMERA_FOV_DEG,
              parallelHeight: c.parallel_height,
              eye: c.eye,
              target: c.target,
              up: c.up ?? DEFAULT_CAMERA_UP,
            },
            0,
          )
        } else {
          viewportApi.setCamera(
            c.eye,
            c.target,
            c.up ?? DEFAULT_CAMERA_UP,
            c.fov_deg ?? DEFAULT_CAMERA_FOV_DEG,
            c.projection ?? DEFAULT_CAMERA_PROJECTION,
          )
        }
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
      targets.activateScene?.(directive.sid)
      return
    case 'selection':
      applySelection(directive.entries, targets)
      return
    case 'hidden_changed':
      // The kernel's user-hidden flags moved: re-read them into the app's
      // hidden set first, then reconcile tags against that fresh set.
      targets.reseedViewState?.()
      targets.reconcileVisibility?.()
      return
    case 'isolate':
      // An aborted plugin command putting an isolation back does not reframe.
      if (targets.isolate !== undefined) {
        targets.isolate(isolationTargets(directive.entries), { frame: directive.frame !== false })
      } else {
        viewportApi?.isolate(isolationTargets(directive.entries), { frame: directive.frame !== false })
      }
      return
    case 'clear_isolation':
      viewportApi?.clearIsolation()
      return
    case 'zoom_to':
      viewportApi?.zoomToWorldBounds(directive.min, directive.max)
      return
  }
}

/** Parses `take_pending_view_directives`'s JSON array. Malformed input
 * (never expected — the Rust side is the only producer) reads as none. */
export function parseViewDirectives(directivesJson: string): ViewDirective[] {
  try {
    const parsed = JSON.parse(directivesJson) as unknown
    return Array.isArray(parsed) ? (parsed as ViewDirective[]) : []
  } catch {
    return []
  }
}

/** Whether a dispatch's directives include a `hew.scenes.apply` activation. */
function hasSceneActivation(directives: ViewDirective[]): boolean {
  return directives.some((d) => d.kind === 'activate_scene')
}

/** Applies every directive in `directivesJson` (from
 * `Scene.take_pending_view_directives`) in order, tolerating malformed
 * input defensively by no-op rather than throwing into the Tauri event
 * handler; one directive's failure does not stop the rest. Exported for
 * direct unit testing alongside `shouldRefreshAfterDispatch`. */
export function applyPendingViewDirectives(directivesJson: string, targets: DirectiveTargets): void {
  for (const directive of parseViewDirectives(directivesJson)) {
    try {
      applyViewDirective(directive, targets)
    } catch (err) {
      console.error('a live view directive failed to apply', directive.kind, err)
    }
  }
}

/** The directive targets `deps` reaches. */
export function directiveTargets(deps: LiveBridgeDeps): DirectiveTargets {
  return {
    viewportApi: deps.getViewportApi(),
    activateScene: deps.activateScene,
    reseedViewState: deps.reseedViewState,
    reconcileVisibility: deps.reconcileVisibility,
    selectMany: deps.selectMany,
    selectGuide: deps.selectGuide,
    selectAnnotation: deps.selectAnnotation,
    selectNone: deps.selectNone,
    isolate: deps.isolate,
  }
}

/** Re-renders after a dispatch changed the document: the mounted viewport
 * refreshes its scene, else the app reconciles without one. */
function refreshAfterMutation(deps: LiveBridgeDeps): void {
  const api = deps.getViewportApi()
  if (api !== null) api.refreshScene()
  else deps.reconcile()
}

/**
 * The tail every answered dispatch runs, after its reply is on its way:
 * re-render a mutation, mark Library materials dirty, take and apply the
 * dispatch's view directives in order, and reconcile tag visibility. Shared
 * by the bridge and the test harness's `apiDispatch`, so both apply a
 * dispatch exactly the same way. The caller pumps events afterwards.
 */
export function afterDispatch(deps: LiveBridgeDeps, scene: Scene, reply: string, mutated: boolean): void {
  if (mutated) refreshAfterMutation(deps)
  if (mutated && addedLibraryMaterials(reply)) deps.markLibraryMaterialsDirty?.()
  // Every directive the dispatch left (`take_pending_view_directives`'s own
  // doc comment has the full story); `[]` for a refused one.
  const directivesJson = scene.take_pending_view_directives()
  // A Scene activation applies the Scene's hidden nodes AND tags itself;
  // reconciling tags first would push against the pre-activation node
  // hides, only to be overwritten a moment later.
  if (mutated && !(deps.activateScene !== undefined && hasSceneActivation(parseViewDirectives(directivesJson)))) {
    deps.reconcileVisibility?.()
  }
  applyPendingViewDirectives(directivesJson, directiveTargets(deps))
}

/**
 * Dispatches one frame on `wasmId` and runs the whole answered-dispatch
 * path, returning the reply frame (`undefined` for a notification). A
 * plugin command is answered `unavailable` at once, no compound opened:
 * this is the test harness's synchronous path
 * (`__hew_test.apiDispatch`), with no queue and no plugin host behind it.
 */
export function dispatchFrameNow(
  deps: LiveBridgeDeps,
  scene: Scene,
  wasmId: number,
  frame: string,
): string | undefined {
  scene.set_display_units(getLengthUnit())
  const result = parseDispatch(scene.api_dispatch(wasmId, frame))
  if (result.kind === 'none') return undefined
  const reply =
    result.kind === 'reply'
      ? result.frame
      : completeForward(scene, wasmId, result.token, result.forward.request_id)
  const mutated = result.kind === 'reply' && shouldRefreshAfterDispatch(scene, frame, reply)
  afterDispatch(deps, scene, reply, mutated)
  pumpEvents(scene)
  return reply
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
  let unlistenOp: (() => void) | undefined
  let unlistenAbort: (() => void) | undefined
  let unlistenDocumentId: (() => void) | undefined
  // The id the shell gave this window's document (`hew://document-id`),
  // told to the Scene before each dispatch: it may arrive before the Scene.
  let hostDocumentId: string | null = null
  let tauriEmit: ((event: string, payload?: unknown) => Promise<void>) | null = null

  // Frames that arrived while a plugin command held the document, in
  // arrival order (see the module comment).
  const queued: Array<{ connId: number; frame: string }> = []

  // The compound this bridge opened around a connection's in-flight
  // mutating forward, by shell connId: what `hew://api-abort` and Cancel
  // abort. `done` makes the abort and the forward's completion happen
  // exactly once whichever comes first.
  interface InFlight {
    kernelToken: bigint
    wasmId: number
    token: number
    requestId: number | string
    done: boolean
  }
  const inFlight = new Map<number, InFlight>()

  // The event module, imported once: every emit before it resolves waits on
  // the same import, so emits reach the shell in the order they were made
  // (a queued frame's `api-started` before its `api-reply`).
  let eventModule: Promise<typeof import('@tauri-apps/api/event')> | null = null
  const emitTo = (event: string, payload: unknown): void => {
    if (tauriEmit !== null) {
      tauriEmit(event, payload).catch(() => {
        /* the connection is gone — the shell's own reply-timeout covers it */
      })
      return
    }
    eventModule ??= import('@tauri-apps/api/event')
    eventModule
      .then(({ emit }) => {
        tauriEmit = emit
        return emit(event, payload)
      })
      .catch(() => {
        /* ignore — not in Tauri, or emission failed */
      })
  }

  const emitReply = (connId: number, frame: string): void => {
    emitTo('hew://api-reply', { connId, frame })
  }

  // A socket connection's events go to the shell, which writes them on the
  // connection's own writer, between its replies. A wasm connection this
  // bridge did not open (an in-page plugin's) is someone else's.
  const unregisterSink = registerEventSink((wasmConnId, frame) => {
    for (const [connId, wasmId] of wasmConnOf) {
      if (wasmId === wasmConnId) {
        emitTo('hew://api-event', { connId, frame })
        return
      }
    }
  })

  // A document lifecycle frame goes to every socket connection the shell
  // has subscribed to it, except those this window's own sessions already
  // sent it: a listener bound to another window hears of this document.
  const unregisterBroadcast = registerBroadcastSink((frame, delivered) => {
    const except: number[] = []
    for (const [connId, wasmId] of wasmConnOf) {
      if (delivered.has(wasmId)) except.push(connId)
    }
    emitTo('hew://api-broadcast', { frame, except })
  })

  const pump = (scene: Scene): void => {
    if (hostDocumentId !== null) scene.set_host_document_id(hostDocumentId)
    pumpEvents(scene)
  }

  const handleDocumentId = (id: string): void => {
    hostDocumentId = id
    setPumpDocumentId(id)
    deps.getScene()?.set_host_document_id(id)
  }

  const handleOp = (opId: number, op: string, args: Record<string, unknown>): void => {
    // Resolves once the reply has actually been handed to the shell, so an
    // operation that destroys this window afterwards cannot outrun it.
    const answer = async (outcome: Omit<LiveOpOutcome, 'afterReply'>): Promise<void> => {
      const payload = { opId, ...outcome }
      try {
        if (tauriEmit === null) {
          const { emit } = await import('@tauri-apps/api/event')
          tauriEmit = emit
        }
        await tauriEmit('hew://api-op-reply', payload)
      } catch {
        /* not in Tauri, or emission failed — the shell's own op timeout covers it */
      }
    }
    if (deps.runOp === undefined) {
      void answer({
        ok: false,
        refusal: 'host_capability_missing',
        // oxlint-disable-next-line lingui/no-unlocalized-strings -- API refusal text for developers, which stays English
        message: 'this window cannot run that operation',
      })
      return
    }
    deps
      .runOp(op, args)
      .then(async (outcome) => {
        const { afterReply, ...rest } = outcome
        await answer(rest)
        if (afterReply !== undefined) await afterReply()
      })
      .catch(async (err: unknown) => {
        await answer({ ok: false, message: err instanceof Error ? err.message : String(err) })
      })
  }

  // The tail every aborted compound runs: the document came back whole,
  // view state included, so the app's mirrors are re-read from it and the
  // viewport redrawn — a document-wide change.
  const abortTail = (scene: Scene, kernelToken: bigint): void => {
    try {
      scene.compound_abort(kernelToken)
    } catch (err) {
      console.error('Scene.compound_abort refused a compound this bridge opened', err)
    }
    deps.reseedViewState?.()
    deps.reconcileVisibility?.()
    refreshAfterMutation(deps)
    // The selection and isolation the command's frames changed come back
    // (the camera does not).
    applyPendingViewDirectives(scene.take_pending_view_directives(), directiveTargets(deps))
  }

  // Opens the compound for `connId`'s mutating forward and marks the
  // document busy; false (nothing held) when the scene refuses it.
  const beginInFlight = (
    scene: Scene,
    connId: number,
    wasmId: number,
    token: number,
    forward: { plugin: string; label: string | null; request_id: number | string },
  ): boolean => {
    let kernelToken: bigint
    try {
      kernelToken = scene.compound_begin(wasmId, BigInt(token))
    } catch (err) {
      console.error('Scene.compound_begin refused a forward this bridge was handed', err)
      return false
    }
    inFlight.set(connId, { kernelToken, wasmId, token, requestId: forward.request_id, done: false })
    setBusy({
      plugin: forward.plugin,
      label: forward.label ?? null,
      startedAt: performance.now(),
      cancel: () => settleInFlight(connId, 'cancelled'),
    })
    return true
  }

  // Ends `connId`'s in-flight forward without a plugin answer: aborts its
  // compound, answers the forward with `failed`, and frees the document.
  // Once, whichever of the placeholder answer, Cancel, and the shell's
  // timeout comes first.
  const settleInFlight = (connId: number, failed: 'unavailable' | 'cancelled' | 'timeout'): void => {
    const held = inFlight.get(connId)
    const scene = deps.getScene()
    if (held === undefined || held.done || scene === null) return
    held.done = true
    inFlight.delete(connId)
    abortTail(scene, held.kernelToken)
    emitReply(connId, completeForward(scene, held.wasmId, held.token, held.requestId, failed))
    // The abort's `document_changed` follows the reply.
    pump(scene)
    // Cleared after the reply is on its way, so what waited behind this
    // command is dispatched after it, never before.
    if (!scene.busy()) setBusy(null)
  }

  // Dispatches what waited behind a plugin command, in arrival order,
  // while the document stays free.
  let flushing = false
  const flushQueue = (): void => {
    if (flushing) return
    flushing = true
    try {
      drainQueue()
    } finally {
      flushing = false
    }
  }
  const drainQueue = (): void => {
    while (queued.length > 0) {
      const scene = deps.getScene()
      if (scene === null) return
      if (scene.busy()) {
        // A queued frame started another plugin command: the document made
        // progress, so what still waits gets a fresh clock.
        for (const connId of new Set(queued.map((q) => q.connId))) {
          emitTo('hew://api-started', { connId })
        }
        return
      }
      const next = queued.shift()
      if (next === undefined) return
      emitTo('hew://api-started', { connId: next.connId })
      handleFrame(next.connId, next.frame, true)
    }
  }
  // Whoever ends a compound (this bridge, the test harness, and later the
  // plugin host) clears the busy state; that is the moment to drain.
  const unsubscribeBusy = subscribeBusy(() => {
    if (getBusy() === null) flushQueue()
  })

  const handleOpen = (connId: number): void => {
    openConnIds.add(connId)
    // Idempotent: the shell re-announces a connection while it waits for a
    // freshly created window to come up (the first announcement can land
    // before this listener exists), so a repeat must not open a second wasm
    // connection and orphan the first.
    if (wasmConnOf.has(connId)) return
    const scene = deps.getScene()
    if (scene === null) return // not ready yet — handleFrame lazily opens on the first frame instead
    wasmConnOf.set(connId, scene.api_connection_open())
  }

  const handleClose = (connId: number): void => {
    openConnIds.delete(connId)
    // What it left waiting dies with it: its socket can take no reply.
    for (let i = queued.length - 1; i >= 0; i--) {
      if (queued[i].connId === connId) queued.splice(i, 1)
    }
    const wasmId = wasmConnOf.get(connId)
    wasmConnOf.delete(connId)
    if (wasmId === undefined) return
    deps.getScene()?.api_connection_close(wasmId)
  }

  function handleFrame(connId: number, frame: string, fromQueue = false): void {
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
    // A plugin command holds the document: wait, in order, behind it (and
    // behind anything already waiting). The bridge's own frames are never
    // stamped with a forward, so every one waits.
    if (scene.busy() || (!fromQueue && queued.length > 0)) {
      queued.push({ connId, frame })
      emitTo('hew://api-queued', { connId })
      if (!scene.busy()) flushQueue()
      return
    }
    // A save that hands bytes to a client (the render fallback's way of
    // reading this document) should see the user's current camera.
    try {
      if ((JSON.parse(frame) as { method?: unknown }).method === 'hew.doc.save') {
        deps.pushCameraState?.(scene)
      }
    } catch {
      /* an unparseable frame is answered by the dispatcher below */
    }
    // A `.skp` the API exports opens in the app's display units, as File
    // > Export writes it, so the dispatch sees the current preference.
    scene.set_display_units(getLengthUnit())
    if (hostDocumentId !== null) scene.set_host_document_id(hostDocumentId)
    const result = parseDispatch(scene.api_dispatch(wasmId, frame))
    if (result.kind === 'none') return // a notification: no reply, per §4.1
    let reply: string
    if (result.kind === 'reply') {
      reply = result.frame
    } else if (result.forward.mutating && beginInFlight(scene, connId, wasmId, result.token, result.forward)) {
      // A mutating plugin command runs inside a compound: one undo entry,
      // nothing else lands meanwhile, and an abort puts the document back.
      // No plugin host runs yet, so the forward is answered `unavailable`
      // (and the compound aborted) as soon as this frame's task is done;
      // the plugin realm routes it to the plugin's background instead once
      // that lands. Until then it is in flight: frames queue behind it,
      // and Cancel or the shell's timeout can end it.
      queueMicrotask(() => settleInFlight(connId, 'unavailable'))
      return
    } else {
      reply = completeForward(scene, wasmId, result.token, result.forward.request_id)
    }
    emitReply(connId, reply)
    // A forward is judged by the plugin command itself, not the envelope
    // that carried it: a read-only plugin command inside a sole-command
    // transaction changed nothing.
    const mutated =
      result.kind === 'pending'
        ? result.forward.mutating && replySucceeded(reply)
        : shouldRefreshAfterDispatch(scene, frame, reply)
    // Re-render, then the view effects the dispatch left (camera, units, a
    // Scene activation, the selection, hide and show, isolation, framing),
    // in the order its commands ran.
    afterDispatch(deps, scene, reply, mutated)
    // What the dispatch changed reaches subscribers after its reply.
    pump(scene)
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
        win.listen<{ opId: number; op: string; args: Record<string, unknown> }>(
          'hew://api-op',
          (event) => {
            handleOp(event.payload.opId, event.payload.op, event.payload.args ?? {})
          },
        ),
        // The shell's reply clock ran out for this connection: abort the
        // compound held for its forward, if any.
        win.listen<{ connId: number }>('hew://api-abort', (event) => {
          settleInFlight(event.payload.connId, 'timeout')
        }),
        win.listen<{ id: string }>('hew://document-id', (event) => {
          handleDocumentId(event.payload.id)
        }),
      ])
    })
    .then(([openFn, closeFn, frameFn, opFn, abortFn, documentIdFn]) => {
      if (cancelled) {
        openFn()
        closeFn()
        frameFn()
        opFn()
        abortFn()
        documentIdFn()
        return
      }
      unlistenOpen = openFn
      unlistenClose = closeFn
      unlistenFrame = frameFn
      unlistenOp = opFn
      unlistenAbort = abortFn
      unlistenDocumentId = documentIdFn
    })
    .catch(() => {
      /* not in Tauri (shouldn't happen — gated by isTauri above), or registration failed */
    })

  return () => {
    cancelled = true
    unsubscribeBusy()
    unregisterSink()
    unregisterBroadcast()
    unlistenOpen?.()
    unlistenClose?.()
    unlistenFrame?.()
    unlistenOp?.()
    unlistenAbort?.()
    unlistenDocumentId?.()
  }
}
