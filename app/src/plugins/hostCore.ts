/* oxlint-disable lingui/no-unlocalized-strings -- protocol words, log lines and plugin copy; English only until the feature graduates. */
/**
 * The plugin host of one document window: it registers each enabled
 * plugin's commands, starts a plugin's realm when the plugin is first
 * needed (a forward to one of its commands, or Hew starting when the user
 * allowed that), serves forwards and the realm's own bus calls, keeps its
 * storage, and stops it when it misbehaves.
 *
 * Who owns a forward: the side that received the call owns its compound,
 * busy state and completion. A socket caller's forward is the live
 * bridge's, which asks `serve` (through the forward router) for the
 * plugin's answer. A call that comes from a realm, or from this window
 * itself (the test harness), is the host's own: `dispatch` opens and
 * closes the compound, sets the busy notice, completes the forward and
 * runs the dispatch tail.
 *
 * Every message from a realm passes `checkRealmMessage` and the realm's
 * `RateMeter`; a failure of either, a forward that runs out of time, two
 * unanswered pings, an uncaught error, or a frame that navigates stops the
 * realm (`crash`). Nothing a realm sends can throw into the host: every
 * handler is wrapped and turns an exception into a crash.
 *
 * A plugin the store says may not run now is `blocked`, never crashed: a
 * folder plugin while developer mode is off, a record waiting for the
 * person to review a change, or a folder whose code cannot be read when the
 * plugin starts. A blocked plugin keeps its commands registered, so a
 * caller hears `unavailable` rather than that the command does not exist;
 * it gets no crash record and no toast, and it runs again as soon as the
 * store says it may, without being turned off and on. A folder plugin's
 * files changing reloads it (`plugins-dev-changed`).
 *
 * A realm that has nothing open and has exchanged no message with the host
 * for `idleMs` is unloaded (`idle`) and starts again on its next command;
 * one that starts with Hew is never unloaded that way. A plugin whose
 * manifest lists events (`contributes.events`, with `on_event`) is started
 * by the first of them: while it does not run, the host keeps a listener
 * connection for it, subscribed to those events. A grant that only adds
 * optional permissions is applied to a running realm in place; any other
 * change of grant reloads it. A plugin asks for an optional permission
 * with `hew.permissions.request`, which this host answers.
 */

import type { ForwardRequestJson } from '../api/dispatchResult'
import { parseDispatch } from '../api/dispatchResult'
import { getBusy, setBusy, subscribeBusy } from '../api/busyState'
import { pumpDocumentId, pumpEvents, registerBroadcastSink, registerEventSink } from '../api/eventPump'
import type { ForwardReplyJson, ForwardRouter } from '../api/forwardRouter'
import {
  abortCompound,
  beginCompound,
  endCompound,
  innermostForward,
  isForwardOpen,
  subscribeCompounds,
  type OpenCompound,
} from '../api/hostCompounds'
import {
  afterAbort,
  afterDispatch,
  afterEnterDispatch,
  beforeDispatch,
  enterRefusedReply,
  frameMethod,
  refreshesAfter,
  refusedFrame,
  completeForwardWith,
  type LiveBridgeDeps,
} from '../api/liveBridge'
import type { Scene } from '../wasm/loader'
import type { ActivationReason, HostToRealm, RealmToHost, UnloadReason } from '../../../sdk/plugin/src/protocol'
import { toPluginError } from './pluginError'
import { cleanText, compareCodePoints, displayName } from './pluginText'
import { checkBootMessage, checkRealmMessage, LIMITS, RateMeter, wellFormed } from './realm/validate'
import type { PluginStorageBackend } from './storageBackend'
import type { GrantDecider, PluginStoreAccess } from './storeAccess'
import { needsConsent, type Installed, type Listed, type PluginManifest } from './types'

/** Where a plugin stands in this window. */
export type RealmState = 'enabled' | 'activating' | 'activated' | 'crashed' | 'blocked'

/** Why a plugin is `blocked`. The first two come from the store and last
 *  until it says otherwise; the last two come from a start that could not
 *  read the plugin's code, and the next start tries again. */
export type BlockReason = 'needs_developer_mode' | 'needs_review' | 'folder_missing' | 'main_missing'

const STORE_BLOCKS: ReadonlySet<BlockReason> = new Set(['needs_developer_mode', 'needs_review'])

/** What the store says keeps a plugin from running now, or null. */
export function storeBlock(installed: Installed, devMode: boolean): BlockReason | null {
  if (installed.source.kind === 'dir' && !devMode) return 'needs_developer_mode'
  if (needsConsent(installed)) return 'needs_review'
  return null
}

/** What a refused read of a plugin's main script means, or null when the
 *  refusal is not one that says the plugin may not run now. Only the
 *  shell's refusal names count, never text a plugin wrote. */
function readBlock(installed: Installed, e: unknown): BlockReason | null {
  const name = toPluginError(e).refusal
  if (name === 'developer_mode_off') return 'needs_developer_mode'
  if (name === 'dev_consent_pending') return 'needs_review'
  if (installed.source.kind !== 'dir') return null
  if (name === 'folder_missing') return 'folder_missing'
  if (name === 'package_path_invalid') return 'main_missing'
  return null
}

/** Either end of a `MessagePort`, as the host uses it. */
export interface PortLike {
  postMessage(message: unknown, transfer?: Transferable[]): void
  onmessage: ((ev: MessageEvent) => void) | null
  /** A message that arrived but could not be read. */
  onmessageerror?: ((ev: MessageEvent) => void) | null
  close(): void
}

/** Console lines a realm may put in the log per second; the rest are
 *  dropped, so a chatty plugin cannot push everything else out of it. */
const CONSOLE_LINES_PER_SECOND = 20

/** Console lines a realm may put in the log in one run of sustained output.
 *  Past it one notice is logged and the plugin's console is not shown again
 *  until it has been quiet for `CONSOLE_QUIET_MS`. Well under the log's 500
 *  lines, so other entries (another plugin's stop, say) survive a flood. */
const CONSOLE_LINES_PER_RUN = 200
const CONSOLE_QUIET_MS = 10_000

/** Bus calls a realm may have waiting in the host: queued and not yet
 *  dispatched, or dispatched and waiting for the document or a nested
 *  command. More is a violation. At least the per-second message limit, so
 *  a burst is stopped by that limit, which names it, first. */
export const MAX_PENDING_BUS = 1_000

/** How the host takes turns with the rest of the page: `now` is a
 *  millisecond clock and `later` runs a function in a later task. */
export interface HostScheduler {
  now(): number
  later(fn: () => void): void
}

/** `later` on a message channel (a task, so input and frames run between
 *  two), falling back to a timer. */
function defaultScheduler(): HostScheduler {
  const now = (): number => performance.now()
  if (typeof MessageChannel === 'undefined') return { now, later: (fn) => void setTimeout(fn, 0) }
  const channel = new MessageChannel()
  const queue: (() => void)[] = []
  channel.port1.onmessage = () => {
    const fn = queue.shift()
    fn?.()
  }
  return {
    now,
    later(fn) {
      queue.push(fn)
      channel.port2.postMessage(null)
    },
  }
}

/** A started realm frame: the control port to its boot script and the realm
 *  port to its Worker. */
export interface RealmChannel {
  control: PortLike
  port: PortLike
  /** Removes the frame, which ends its Worker. */
  remove(): void
}

export type { GrantDecider } from './storeAccess'

/** A state the host reports for a plugin: its `RealmState`, or `idle`
 *  (an idle unload) and `unloaded` (any other stop that leaves it on). */
export type ReportedState = RealmState | 'idle' | 'unloaded'

/** One console line as the host hands it on. */
export interface ConsoleLine {
  level: 'log' | 'info' | 'warn' | 'error'
  text: string
  /** Milliseconds since the Unix epoch, when the host took it. */
  at: number
}

/** Console lines handed on in one call, at most. */
export const CONSOLE_LINES_PER_CALL = 64

/** The most a plugin keeps in its storage for an unsaved document, held by
 *  the host until the first save: the same 5 MiB as a saved document's. */
export const SESSION_STORAGE_BYTES = 5 << 20

/** Where the host runs: a desktop document window, a web one, or Shop Mode
 *  (a viewer: every grant is cut to reading the model and the view, by the
 *  kernel, and a plugin cannot ask for more). */
export type HostPlatform = 'desktop' | 'web' | 'shop'

export interface HostEnv {
  scene(): Scene | null
  /** The live bridge's deps: the viewport and the app's handlers the
   *  dispatch tail reaches. */
  bridge: LiveBridgeDeps
  platform: HostPlatform
  store: PluginStoreAccess
  storage: PluginStorageBackend
  /** `plugin_grant_json` from the kernel module. `viewer` cuts the grant
   *  to what a viewer may do; the host passes it on Shop. */
  grantJson(id: string, document: string, namesJson: string, viewer: boolean): string
  /** Asks the person whether plugin `id` may have the optional permissions
   *  `add` (`hew.permissions.request`). Absent where nobody can be asked:
   *  every request is then refused `request_declined`. */
  grants?: GrantDecider
  /** Creates a realm frame and starts its Worker on `main`. Rejects when
   *  the frame never answers. */
  openRealm(main: ArrayBuffer, onNavigate: () => void): Promise<RealmChannel>
  toast(message: string): void
  log(level: 'info' | 'warn' | 'error', message: string): void
  /** A plugin's state changed in this window (`enabled`, `activating`,
   *  `activated`, `crashed`, `blocked`, `idle`, `unloaded`), with why.
   *  Fire and forget: the host never waits for it, and what it throws is
   *  swallowed. A promise it returns that rejects (the shell refused the
   *  report) means the state was not taken: the host sends it again with
   *  its next report of that state, and tries a stop (`crashed`,
   *  `blocked`) once more by itself after `reportRetryMs`. */
  report?(id: string, state: ReportedState, reason: string | null): unknown
  /** A plugin's console lines since the last call, at most one call a
   *  second per plugin and 64 lines a call. Fire and forget, as `report`. */
  console?(id: string, lines: ConsoleLine[]): void
  locale(): string
  theme(): 'light' | 'dark'
}

/** Timings, shortened by tests. */
export interface HostTimings {
  pingMs: number
  /** The start timeout, used twice: for what comes before the realm frame
   *  (the document id on the desktop, and the moment the document lets the
   *  plugin's connection say hello), and from hello to the realm's
   *  acknowledgement. */
  startMs: number
  /** How long a write waits for others to join it. */
  storageDebounceMs: number
  /** The most main-thread time one turn at a realm's messages takes before
   *  the host yields to the page. */
  sliceMs: number
  /** A realm with nothing open and no message either way (pongs aside) for
   *  this long is unloaded. Checked on each ping. */
  idleMs: number
  /** How long an idle unload waits for the realm's storage writes and its
   *  `unloaded`. */
  drainMs: number
  /** How long a person's act in a plugin's own UI lets it ask for a
   *  permission (once). */
  activationMs: number
  /** How long a plugin is refused `request_declined` after a no. */
  declineMs: number
  /** The longest a permission request waits for the person; the forward
   *  it was made in has its clock stopped meanwhile. */
  decisionMs: number
  /** The least time between two starts of one plugin by an event, and the
   *  most it grows to after starts that failed (it doubles each time). */
  eventStartMs: number
  eventStartMaxMs: number
  /** How long after a refused report of a stop (`crashed`, `blocked`) the
   *  host tries it once more. */
  reportRetryMs: number
}

export const DEFAULT_TIMINGS: HostTimings = {
  pingMs: 2_000,
  startMs: 10_000,
  storageDebounceMs: 250,
  sliceMs: 8,
  idleMs: 10 * 60_000,
  drainMs: 2_000,
  activationMs: 5_000,
  declineMs: 60_000,
  decisionMs: 5 * 60_000,
  eventStartMs: 10_000,
  eventStartMaxMs: 5 * 60_000,
  reportRetryMs: 10_000,
}

/** Event frames kept for a plugin while it starts (the oldest go first). */
export const MAX_HELD_EVENTS = 64

/** The document id the kernel gives a document when the shell names none:
 *  always on the web. */
const KERNEL_DEFAULT_DOCUMENT = 'doc_1'

/**
 * How long, and how many of, the forwards that ended are remembered, with
 * how each ended: answered by the realm, or given up by the host
 * (cancelled, or ended under a nested command). A reply to a forward
 * remembered as answered is a second answer, a violation. Every other
 * reply to a forward the realm was issued and that is no longer open is
 * late and dropped quietly: one remembered as given up, and one no longer
 * remembered at all, since a forgotten one could have been either and a
 * late answer must never stop a healthy plugin. Only a second answer that
 * comes within the memory is caught, and the memory is bounded: entries go
 * after five minutes (the SDK never answers twice, and a late answer is one
 * already on its way when the host gave up, seconds at worst), and past
 * 4,096 entries the oldest goes first, which needs more than 13 forwards
 * ending every second for five minutes.
 */
const ENDED_MS = 5 * 60_000
const ENDED_MAX = 4_096

/** Package bytes one file request may carry, and one realm may have read
 *  for it in any rolling second. A package entry is at most 32 MiB, so one
 *  request always fits the largest file. Requests are served one at a time
 *  per realm, so the page never holds more than one request's bytes for a
 *  realm, plus the file being read. */
export const FILES_BYTES_PER_REQUEST = 64 << 20
export const FILES_BYTES_PER_SECOND = 128 << 20

/** The refusal a bus call gets when the forward it names has ended. */
export const FORWARD_ENDED = 'forward_ended'

/** The preview kernel's `Scene.note_plugin_state` (`kernelExports` in
 *  preview-features.json): absent from a production kernel's `Scene`, so
 *  reached through this shape, never through the production type. */
interface PluginStateScene {
  note_plugin_state?: (id: string, state: string, detail?: string) => void
  api_connection_set_grant?: (conn: number, grant: string) => void
}

/** Reports a plugin's state to the kernel (`plugin_state` events); a kernel
 *  without the method has nobody to report it to. */
function notePluginState(scene: Scene, id: string, state: 'loaded' | 'crashed' | 'unloaded' | 'grant_changed', detail?: string): void {
  const note = (scene as unknown as PluginStateScene).note_plugin_state
  if (typeof note === 'function') note.call(scene, id, state, detail)
}

/** The preview kernel's `Scene.api_connection_set_grant`: replaces a
 *  plugin connection's grant in place. Throws when the kernel has none. */
function setConnectionGrant(scene: Scene, conn: number, grant: string): void {
  const set = (scene as unknown as PluginStateScene).api_connection_set_grant
  if (typeof set !== 'function') throw new Error('this kernel cannot change a grant in place')
  set.call(scene, conn, grant)
}

/** A start that could not go on for a reason that is not the plugin's: it
 *  is answered `unavailable` and the plugin stays enabled. */
class NotStarted extends Error {}

/** A start overtaken by a teardown. */
class Stale extends Error {}

/** A start the store does not allow now: the plugin is blocked, not
 *  stopped. */
class Blocked extends Error {
  constructor(readonly reason: BlockReason) {
    super(reason)
  }
}

/** The longest a plugin command may run: the kernel's
 *  `MAX_PLUGIN_TIMEOUT_MS` (crates/api/src/registry.rs), which clamps a
 *  declared `timeout_ms` to it and gives it to a command that declares
 *  none. A test reads the Rust constant and fails when the two drift. */
export const MAX_FORWARD_TIMEOUT_MS = 50_000

/** A forward's time limit: the command's own, clamped to 1 s ..
 *  `MAX_FORWARD_TIMEOUT_MS`; that ceiling when it gives none that makes
 *  sense (the kernel always fills one in). */
export function forwardTimeoutMs(declared: unknown): number {
  if (typeof declared !== 'number' || !Number.isFinite(declared) || declared <= 0) return MAX_FORWARD_TIMEOUT_MS
  return Math.min(MAX_FORWARD_TIMEOUT_MS, Math.max(1_000, Math.round(declared)))
}

const REFUSAL_NAME = /^[a-z][a-z0-9_]{0,63}$/
const REALM_FAILURES = new Set(['crashed', 'no_handler'])

/** A realm's `forward_reply` text as a reply, or null when it is not one. */
export function readForwardReply(text: string): ForwardReplyJson | null {
  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch {
    return null
  }
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return null
  const keys = Object.keys(parsed)
  if (keys.length !== 1) return null
  const r = parsed as Record<string, unknown>
  if (keys[0] === 'ok') return { ok: r.ok }
  if (keys[0] === 'failed') return typeof r.failed === 'string' && REALM_FAILURES.has(r.failed) ? ({ failed: r.failed } as ForwardReplyJson) : null
  if (keys[0] !== 'refusal') return null
  const refusal = r.refusal
  if (typeof refusal !== 'object' || refusal === null || Array.isArray(refusal)) return null
  const f = refusal as Record<string, unknown>
  if (Object.keys(f).some((k) => k !== 'name' && k !== 'explanation' && k !== 'detail')) return null
  if (typeof f.name !== 'string' || !REFUSAL_NAME.test(f.name)) return null
  if (typeof f.explanation !== 'string' || f.explanation.length > 2_000) return null
  return { refusal: { name: f.name, explanation: f.explanation, detail: f.detail ?? null } }
}

/** A path inside a plugin package, or null: relative, forward slashes, no
 *  `..`, no empty or dot segments. The store checks again. */
export function packagePath(path: string): string | null {
  if (path.length === 0 || path.length > LIMITS.pathChars) return null
  if (/[\\:\u0000-\u001f]/.test(path) || path.startsWith('/')) return null
  const segments = path.split('/')
  if (segments.some((s) => s === '' || s === '.' || s === '..')) return null
  return path
}

const replySucceeded = (reply: string): boolean => {
  try {
    return (JSON.parse(reply) as { error?: unknown }).error === undefined
  } catch {
    return false
  }
}

const refusedBusy = (reply: string): boolean => {
  try {
    const data = (JSON.parse(reply) as { error?: { data?: { refusal?: unknown } } }).error?.data
    return data?.refusal === 'document_busy'
  } catch {
    return false
  }
}

/**
 * The answer to a bus call that names a forward that has ended (answered,
 * cancelled, out of time, or ended under a nested command): `-32000`
 * `forward_ended`, nothing run. Undefined for a notification.
 */
export function forwardEndedReply(frame: string): string | undefined {
  const method = frameMethod(frame)?.slice(0, 256) ?? ''
  return refusedFrame(
    frame,
    method,
    FORWARD_ENDED,
    'The command this call was made for has already ended, so the call was not run.',
    {},
  )
}

interface OpenForward {
  fwd: ForwardRequestJson
  settle(reply: ForwardReplyJson): void
  /** Stops and restarts the forward's clock (while the person decides on
   *  a permission it asked for). Nested pauses count. */
  pause(): void
  resume(): void
}

interface LiveRealm {
  channel: RealmChannel
  conn: number
  /** The document its grant is bound to, and the grant: the JSON the
   *  kernel built and the names it was built from, as `grantNames` keys
   *  them. */
  document: string
  grant: string
  names: string
  /** When a message last went either way (pongs and pings aside). */
  lastActivity: number
  /** Set while an idle unload waits for the realm: resolved by the
   *  teardown that ends it. */
  unloading: { done: Promise<void>; resolve(): void } | null
  /** Permission requests waiting for the person. */
  requests: number
  meter: RateMeter
  /** The realm acknowledged hello. */
  ready: boolean
  nextForward: number
  forwards: Map<number, OpenForward>
  /** Forwards that ended, with when and how, oldest first, bounded by
   *  `ENDED_MS` and `ENDED_MAX`. */
  ended: Map<number, { at: number; answered: boolean }>
  storageIds: Set<number>
  fileIds: Set<number>
  /** File requests are served one at a time, in order. */
  files: Promise<void>
  /** Package bytes read for file requests, with when, oldest first. */
  fileReads: { at: number; bytes: number }[]
  /** Messages checked and counted on arrival, handled in order by
   *  `drain` in slices. */
  inbox: RealmToHost[]
  inboxHead: number
  /** Bus calls in the inbox. */
  queuedBus: number
  draining: boolean
  /** Bus calls dispatched and not yet answered (waiting for the document
   *  or a nested command). */
  waiting: number
  pingN: number
  unanswered: number
  timers: ReturnType<typeof setInterval>[]
  consoleWindow: number
  consoleLines: number
  /** Lines logged in the current run of output, when the last line came,
   *  and whether the run went over its budget. */
  consoleRun: number
  consoleLast: number
  consoleMuted: boolean
  /** Set while starting: called on hello_ack, and when the realm goes. */
  onReady?: () => void
  onGone?: () => void
}

interface Tracked {
  id: string
  version: string
  name: string
  installed: Installed
  manifest: PluginManifest
  /** Version and grant, to tell a change from a repeat notice. */
  key: string
  /** It was running when its folder changed under it: when the folder's
   *  version changed (the store says so first, the reload comes after), or
   *  when a review mark stopped it (the person may undo the change, which
   *  clears the mark). The reload, or the mark clearing, starts it again.
   *  Any other stop forgets it. */
  resume: boolean
  state: RealmState
  /** Why it is `blocked`; null in every other state. */
  blocked: BlockReason | null
  realm: LiveRealm | null
  activation: Promise<LiveRealm | null> | null
  /** The stamps of the forwards waiting for the activation: a compound
   *  one of them holds lets the plugin's connection say hello inside it. */
  joiners: Set<number>
  /** Bumped by every teardown: a stale async step checks it and stops. */
  gen: number
  /** Event frames for the plugin that came while it was not ready for
   *  them (from its listener, or on its own connection while it started),
   *  oldest first, at most `MAX_HELD_EVENTS`; delivered when it is. */
  held: string[]
  /** Frames dropped from `held` for room, logged once per start. */
  heldDropped: number
  /** The listener connection kept while the plugin does not run, when its
   *  manifest lists events. */
  listener: Listener | null
  /** Set while a listener is being opened (before `listener` is, while the
   *  desktop document has no id yet): one opening at a time. Cleared, which
   *  stops that opening, by `closeListener` and so by every teardown. */
  listenerOpening: object | null
  /** Starts by an event: when the next may come, and how far apart. */
  eventStarts: { nextAt: number; gapMs: number; timer: ReturnType<typeof setTimeout> | null }
  /** Storage writes not yet handed to the store, by scope, document id
   *  and key (`slotOf`); `null` deletes. */
  writes: Map<string, PendingWrite>
  writeTimer: ReturnType<typeof setTimeout> | null
  /** Every batch handed to the store, in order: the next one starts after
   *  it, and a read that must see them waits for it. Never rejects. */
  flushed: Promise<void>
  /** Document storage carried from an unsaved document to the id its
   *  first save gave it, whose write failed: by document id, then key.
   *  Reads see it; the plugin's next write tries it again. */
  unmigrated: Map<string, Map<string, string>>
  /** The state last reported, with its reason; null when none was, or
   *  the last one was refused. */
  reported: string | null
  /** Which report call `reported` came from: a refusal of an earlier
   *  call of the same state and reason leaves a later one's mark. */
  reportedBy: number
  /** The one later try of a refused report of a stop. */
  reportRetry: ReturnType<typeof setTimeout> | null
  /** Ends the permission request this plugin has open, when it has one:
   *  called when its realm goes, so the next realm is not refused
   *  `request_pending` for a question nobody can answer it. */
  closeRequest?: () => void
  /** Console lines not yet handed on, and the timer that hands them on. */
  consoleOut: ConsoleLine[]
  consoleTimer: ReturnType<typeof setTimeout> | null
}

/** A write not yet handed to the store. `doc` is the document id for
 *  document scope (null for plugin scope); `carried` marks data carried
 *  over from an unsaved document at its first save. */
interface PendingWrite {
  scope: 'plugin' | 'document'
  doc: string | null
  key: string
  value: string | null
  waiters: ((error: string | null) => void)[]
  carried: boolean
}

const slotOf = (scope: 'plugin' | 'document', doc: string | null, key: string): string => JSON.stringify([scope, doc, key])

/** A document id as the `.hew` manifest writes it: a lowercase UUID. */
const DOCUMENT_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/

/** The UTF-8 length of `text`. */
const utf8Length = (text: string): number => new TextEncoder().encode(text).length

/** A connection that listens, for a plugin that does not run, for the
 *  events its manifest lists. `names` are the ones its grant receives,
 *  known once `ready`. */
interface Listener {
  conn: number
  /** The document its grant is bound to. */
  document: string
  /** The permission names its connection's grant was built from, as
   *  `grantNames` keys them. */
  grant: string
  names: string[]
  ready: boolean
}

export interface PluginHost extends ForwardRouter {
  /** Dispatches `frame` on `conn` as this window's own call: queued while
   *  the document is held, and a forward it causes is served and completed
   *  here. Resolves with the reply frame, or undefined for a notification. */
  dispatch(conn: number, frame: string, stamp?: number): Promise<string | undefined>
  /** Re-reads the store: registers new plugins, drops removed ones,
   *  restarts changed ones. */
  sync(): Promise<void>
  /** Re-reads the store and reloads plugin `id` whatever changed: its realm
   *  is stopped (`reload`), its commands registered again from the stored
   *  manifest, and it starts again if it was running or starts with Hew.
   *  Its storage writes still on their way are kept. What a folder plugin's
   *  `plugins-dev-changed` does. */
  reload(id: string): Promise<void>
  states(): Record<string, RealmState>
  /** Why each `blocked` plugin is. */
  reasons(): Record<string, BlockReason>
  /** The person just acted in plugin `id`'s own UI: for `activationMs`
   *  the plugin may ask for an optional permission, once. */
  noteUserActivation(id: string): void
  /** The plugins a listener connection listens for (open and
   *  subscribed), sorted. */
  listening(): string[]
  dispose(): void
}

type Manifest = PluginManifest & {
  main?: unknown
  activation?: unknown
  contributes?: { commands?: unknown; events?: unknown }
}

/** The permission names a record grants, as the kernel takes them. */
const grantNames = (installed: Installed): string => JSON.stringify([...installed.grant, ...installed.optional_granted])

const sameSet = (a: readonly string[], b: readonly string[]): boolean => {
  const x = new Set(a)
  const y = new Set(b)
  return x.size === y.size && [...x].every((n) => y.has(n))
}

/** Whether `next` differs from `prev` only by optional permissions added:
 *  the same version and required grant, every optional one kept, at least
 *  one more. Such a change is applied to a running realm in place. */
export function onlyAddsOptional(prev: Installed, next: Installed): boolean {
  if (prev.version !== next.version || !sameSet(prev.grant, next.grant)) return false
  const before = new Set(prev.optional_granted)
  const after = new Set(next.optional_granted)
  return [...before].every((n) => after.has(n)) && [...after].some((n) => !before.has(n))
}

/** The events a plugin's manifest asks to be started by: its
 *  `contributes.events` when `activation` includes `on_event` (or names
 *  nothing, which means every kind), else none. */
export function startingEvents(manifest: PluginManifest): string[] {
  const m = manifest as Manifest
  const activation = m.activation
  if (activation !== undefined && !(Array.isArray(activation) && activation.includes('on_event'))) return []
  const events = m.contributes?.events
  if (!Array.isArray(events)) return []
  const names = events.filter((e): e is string => typeof e === 'string' && /^[a-z][a-z_]{0,63}$/.test(e))
  return [...new Set(names)].slice(0, 32)
}

const PERMISSION_REQUEST = 'hew.permissions.request'

/** `hew.permissions.request`'s permission names, or null when the params
 *  are not `{permissions: 1 to 64 names, reason?: at most 200 characters}`. */
export function requestedNames(params: unknown): string[] | null {
  if (typeof params !== 'object' || params === null || Array.isArray(params)) return null
  const p = params as Record<string, unknown>
  if (Object.keys(p).some((k) => k !== 'permissions' && k !== 'reason')) return null
  if ('reason' in p && (typeof p.reason !== 'string' || [...p.reason].length > 200)) return null
  const list = p.permissions
  if (!Array.isArray(list) || list.length === 0 || list.length > 64) return null
  if (!list.every((n): n is string => typeof n === 'string' && n.length > 0 && n.length <= 256)) return null
  return [...new Set(list)]
}

export function createPluginHost(
  env: HostEnv,
  timingsGiven: Partial<HostTimings> = {},
  sched: HostScheduler = defaultScheduler(),
): PluginHost {
  const timings: HostTimings = { ...DEFAULT_TIMINGS, ...timingsGiven }
  const viewer = env.platform === 'shop'
  const plugins = new Map<string, Tracked>()
  const byConn = new Map<number, Tracked>()
  /** A starting plugin's connection, before its realm exists. */
  const starting = new Map<number, Tracked>()
  /** Listener connections, by connection. */
  const byListener = new Map<number, Tracked>()
  /** When the person last acted in each plugin's own UI. */
  const activations = new Map<string, number>()
  /** Plugins with a permission request waiting for the person, and until
   *  when a plugin that was told no is refused. Per id, so a reload does
   *  not reset them. */
  const requestOpen = new Set<string>()
  const declinedUntil = new Map<string, number>()
  let disposed = false

  const log = (level: 'info' | 'warn' | 'error', message: string): void => {
    try {
      env.log(level, message)
    } catch {
      /* a log that throws must not stop the host */
    }
  }

  // ------------------------------------------------------------ queueing
  // Calls this window makes wait, in order, while a plugin command holds the
  // document (the kernel refuses them `document_busy` otherwise).
  const waiting: (() => void)[] = []
  const release = (): void => {
    const scene = env.scene()
    if (scene === null || scene.busy()) return
    waiting.shift()?.()
  }
  const turn = (front: boolean): Promise<void> =>
    new Promise((resolve) => {
      if (front) waiting.unshift(resolve)
      else waiting.push(resolve)
    })

  // Something that may let a waiting start go on: the document held or
  // freed, a compound opened or closed, a forward joining an activation, a
  // teardown.
  const nudges = new Set<() => void>()
  const nudge = (): void => {
    for (const fn of [...nudges]) fn()
  }
  /** Resolves on the next nudge, or after `ms`. */
  const nextNudge = (ms: number): Promise<void> =>
    new Promise((resolve) => {
      const done = (): void => {
        nudges.delete(done)
        clearTimeout(timer)
        resolve()
      }
      const timer = setTimeout(done, Math.max(0, ms))
      nudges.add(done)
    })

  const unsubscribeBusy = subscribeBusy(() => {
    if (getBusy() === null) release()
    nudge()
  })
  const unsubscribeCompounds = subscribeCompounds(nudge)

  // -------------------------------------------------------------- events
  const unregisterSink = registerEventSink((conn, frame) => {
    const listening = byListener.get(conn)
    if (listening !== undefined) return guardedHost(() => onListenerFrame(listening, conn, frame))
    const opening = starting.get(conn)
    if (opening !== undefined) return hold(opening, frame)
    const p = byConn.get(conn)
    const realm = p?.realm
    if (p === undefined || realm === null || realm === undefined || realm.conn !== conn) return
    if (realm.ready) post(p, { t: 'bus', frame })
    else hold(p, frame)
  })

  /** Runs a host step that no realm message caused; what it throws is
   *  logged, never let out. */
  const guardedHost = (fn: () => void): void => {
    try {
      fn()
    } catch (e) {
      log('error', `the plugin host failed: ${e instanceof Error ? e.message : String(e)}`)
    }
  }

  /** Keeps an event frame for `p` until its realm is ready for it. */
  const hold = (p: Tracked, frame: string): void => {
    p.held.push(frame)
    if (p.held.length > MAX_HELD_EVENTS) {
      p.held.shift()
      p.heldDropped++
    }
  }

  // A notice of another window's (or this one's) storage change. Document
  // storage goes only to a realm whose document it is.
  const unlistenStorage = env.store.onStorageChanged((id, keys, scope, documentId) => {
    const p = plugins.get(id)
    if (p?.realm?.ready !== true) return
    if (scope === 'plugin') post(p, { t: 'storage_changed', scope: 'plugin', keys })
    else if (documentId !== null && documentId === session.id) post(p, { t: 'storage_changed', scope: 'document', keys })
  })

  // ------------------------------------------------------- what is reported
  /**
   * Reports `state` for `p` when it is not what was reported last. When
   * the report is refused (its promise rejects, or it throws) the state
   * does not count as reported, so the next report of it is sent; a stop
   * (`crashed`, `blocked`) is also tried once more after `reportRetryMs`
   * if nothing else was reported meanwhile and `p` is still in it. Once:
   * a retry that is refused too is not tried again.
   */
  /** Counts report calls, to tell one from another (`reportedBy`). */
  let reportCalls = 0
  const report = (p: Tracked, state: ReportedState, reason: string | null = null, retry = true): void => {
    const key = JSON.stringify([state, reason])
    if (p.reported === key) return
    p.reported = key
    const call = ++reportCalls
    p.reportedBy = call
    if (p.reportRetry !== null) {
      clearTimeout(p.reportRetry)
      p.reportRetry = null
    }
    const refused = (): void => {
      if (p.reportedBy !== call || p.reported !== key) return
      p.reported = null
      if (!retry || (state !== 'crashed' && state !== 'blocked') || disposed) return
      p.reportRetry = setTimeout(() => {
        p.reportRetry = null
        if (p.reported === null && plugins.get(p.id) === p && p.state === state) report(p, state, reason, false)
      }, timings.reportRetryMs)
    }
    let sent: unknown
    try {
      sent = env.report?.(p.id, state, reason)
    } catch (e) {
      log('warn', `the state of ${p.id} could not be reported: ${String(e)}`)
      refused()
      return
    }
    if (typeof sent === 'object' && sent !== null && typeof (sent as { then?: unknown }).then === 'function') (sent as Promise<unknown>).then(undefined, refused)
  }
  /** Moves `p` to `state`, reporting the change. */
  const setState = (p: Tracked, state: RealmState, reason: string | null = null): void => {
    p.state = state
    report(p, state, reason)
  }

  /** Hands `p`'s console lines on, at most `CONSOLE_LINES_PER_CALL` a call. */
  const flushConsole = (p: Tracked): void => {
    if (p.consoleTimer !== null) {
      clearTimeout(p.consoleTimer)
      p.consoleTimer = null
    }
    if (p.consoleOut.length === 0) return
    const lines = p.consoleOut
    p.consoleOut = []
    for (let i = 0; i < lines.length; i += CONSOLE_LINES_PER_CALL) {
      try {
        env.console?.(p.id, lines.slice(i, i + CONSOLE_LINES_PER_CALL))
      } catch (e) {
        log('warn', `the console of ${p.id} could not be handed on: ${String(e)}`)
      }
    }
  }

  const post = (p: Tracked, message: HostToRealm, transfer: Transferable[] = []): void => {
    try {
      if (p.realm !== null && message.t !== 'ping') p.realm.lastActivity = sched.now()
      p.realm?.channel.port.postMessage(message, transfer)
    } catch (e) {
      log('warn', `could not reach the plugin ${p.id}: ${String(e)}`)
    }
  }

  // ------------------------------------------------- the window's own calls
  async function dispatch(conn: number, frame: string, stamp?: number): Promise<string | undefined> {
    let scene = env.scene()
    if (scene === null) return undefined
    if (stamp === undefined) {
      let front = false
      while (scene.busy() || (!front && waiting.length > 0)) {
        await turn(front)
        front = true
        scene = env.scene()
        if (scene === null) return undefined
      }
    } else if (!isForwardOpen(scene, stamp)) {
      // The compound the call was made in has closed: running it now would
      // land outside it, as a write of its own.
      return forwardEndedReply(frame)
    }
    try {
      const result = dispatchNow(scene, conn, frame, stamp)
      if (result.kind === 'none') return undefined
      if (result.kind === 'reply') return result.frame
      return await serveOwned(scene, conn, result.token, result.forward)
    } finally {
      queueMicrotask(release)
    }
  }

  /** Dispatches `frame` at once, as the live bridge does (the same before
   *  and after), and hands back a plugin command it causes, unserved. */
  function dispatchNow(scene: Scene, conn: number, frame: string, stamp: number | undefined): ReturnType<typeof parseDispatch> {
    // A plugin's save sees the user's camera only when its grant can save:
    // a refused one must leave the document byte-identical.
    const realmOf = byConn.get(conn)
    const mayPushCamera = realmOf === undefined || [...realmOf.installed.grant, ...realmOf.installed.optional_granted].includes('documents.save')
    const method = frameMethod(frame)
    beforeDispatch(env.bridge, scene, method, mayPushCamera)
    const raw = stamp === undefined ? scene.api_dispatch(conn, frame) : scene.api_dispatch_in_forward(conn, BigInt(stamp), frame)
    const result = parseDispatch(raw)
    if (result.kind === 'reply') {
      const mutatedNow = (): boolean => result.dryRun !== true && refreshesAfter(scene, method, result.frame)
      const declined =
        method === 'hew.view.enter' && replySucceeded(result.frame)
          ? afterEnterDispatch(env.bridge, scene, result.frame, mutatedNow)
          : afterDispatch(env.bridge, scene, result.frame, mutatedNow())
      pumpEvents(scene)
      // A window that declined an entry the kernel accepted answers the refusal.
      if (declined !== undefined) return { ...result, frame: enterRefusedReply(frame, declined) ?? result.frame }
    }
    return result
  }

  /**
   * A forward a call of this window caused: the host owns its lifecycle.
   * It ends once, whichever comes first: the plugin's answer, Cancel, the
   * forward's timeout or its plugin stopping (all through `serve`), or an
   * outer compound closing over this one (`unwind`).
   */
  function serveOwned(scene: Scene, conn: number, token: number, fwd: ForwardRequestJson): Promise<string> {
    const controller = new AbortController()
    let compound: OpenCompound | null = null
    let ownBusy = false
    let ended = false
    let resolveFrame: (frame: string) => void = () => {}
    const answered = new Promise<string>((resolve) => (resolveFrame = resolve))

    const finish = (reply: ForwardReplyJson): void => {
      if (ended) return
      ended = true
      let answer = reply
      // Whether the compound committed: the document changed whatever the
      // caller's reply says.
      let committed = false
      if (compound !== null && 'ok' in answer) {
        try {
          endCompound(scene, compound)
          committed = true
        } catch (e) {
          log('error', `Scene.compound_end refused a compound the host opened: ${String(e)}`)
          answer = { failed: 'crashed' }
        }
      }
      let frame: string
      if ('ok' in answer) {
        frame = completeForwardWith(scene, conn, token, fwd.request_id, answer)
        // A forward answers a plugin command; `hew.view.enter` is host-served and solitary, so a declined entry never reaches this reply.
        afterDispatch(env.bridge, scene, frame, committed)
      } else if (compound !== null) {
        abortCompound(scene, compound)
        afterAbort(env.bridge, scene)
        frame = completeForwardWith(scene, conn, token, fwd.request_id, answer)
      } else {
        frame = completeForwardWith(scene, conn, token, fwd.request_id, answer)
        afterDispatch(env.bridge, scene, frame, false)
      }
      pumpEvents(scene)
      if (ownBusy && !scene.busy()) setBusy(null)
      resolveFrame(frame)
    }

    if (fwd.mutating) {
      try {
        compound = beginCompound(scene, conn, token, () => {
          controller.abort()
          finish({ failed: 'cancelled' })
        })
      } catch (e) {
        log('error', `Scene.compound_begin refused a forward the host was handed: ${String(e)}`)
        return Promise.resolve(completeForwardWith(scene, conn, token, fwd.request_id, { failed: 'unavailable' }))
      }
      if (getBusy() === null) {
        ownBusy = true
        setBusy({ plugin: fwd.plugin, label: fwd.label ?? null, startedAt: performance.now(), cancel: () => controller.abort() })
      }
    }
    void serve(fwd, controller.signal).then((answer) => finish(controller.signal.aborted ? { failed: 'cancelled' } : answer))
    return answered
  }

  // ------------------------------------------------------------- serving
  function serve(fwd: ForwardRequestJson, signal: AbortSignal): Promise<ForwardReplyJson> {
    const p = plugins.get(fwd.plugin)
    if (p === undefined || p.state === 'crashed' || disposed || storeBlocked(p)) return Promise.resolve({ failed: 'unavailable' })
    if (signal.aborted) return Promise.resolve({ failed: 'cancelled' })
    const timeoutMs = forwardTimeoutMs(fwd.timeout_ms)
    // The realm's generation, taken again once the start is done: a start
    // may wait for an idle unload to end first.
    let gen = p.gen
    return new Promise<ForwardReplyJson>((resolve) => {
      let settled = false
      let realmId: number | null = null
      let timer: ReturnType<typeof setTimeout> | null = null
      /** The clock: what is left of the limit, since when it runs, and
       *  how many pauses hold it. */
      let left = timeoutMs
      let since = 0
      let pauses = 0
      const settle = (reply: ForwardReplyJson): void => {
        if (settled) return
        settled = true
        if (timer !== null) clearTimeout(timer)
        signal.removeEventListener('abort', onAbort)
        resolve(reply)
      }
      const giveUp = (): void => {
        const realm = p.realm
        if (realmId === null || realm === null || p.gen !== gen) return
        realm.forwards.delete(realmId)
        noteEnded(realm, realmId, false)
        post(p, { t: 'forward_cancel', id: realmId })
      }
      const onAbort = (): void => {
        giveUp()
        settle({ failed: 'cancelled' })
      }
      signal.addEventListener('abort', onAbort, { once: true })
      activate(p, 'command', stampFor(fwd)).then(
        (realm) => {
          if (settled) return
          if (realm === null || p.realm !== realm || plugins.get(p.id) !== p) {
            settle({ failed: p.state === 'crashed' ? 'crashed' : 'unavailable' })
            return
          }
          gen = p.gen
          realmId = ++realm.nextForward
          const run = (): void => {
            since = Date.now()
            timer = setTimeout(() => {
              giveUp()
              settle({ failed: 'timeout' })
              crash(p, gen, 'timeout', `the command ${cleanText(fwd.local, 64)} did not answer within ${Math.round(timeoutMs / 1000)} s`)
            }, left)
          }
          realm.forwards.set(realmId, {
            fwd,
            settle,
            pause() {
              if (pauses++ > 0 || settled || timer === null) return
              clearTimeout(timer)
              timer = null
              left = Math.max(0, left - (Date.now() - since))
            },
            resume() {
              if (pauses === 0 || --pauses > 0 || settled) return
              run()
            },
          })
          // The clock runs from here: starting the realm has limits of its
          // own, and a plugin that is only slow to start is not stopped.
          run()
          post(p, {
            t: 'forward',
            id: realmId,
            local: fwd.local,
            params: JSON.stringify(fwd.params ?? {}),
            caller: JSON.stringify(fwd.caller),
            timeout_ms: timeoutMs,
          })
        },
        () => settle({ failed: 'crashed' }),
      )
    })
  }

  /** The forward a call made while serving `fwd` must name to run inside
   *  the compound that holds the document: `fwd`'s own when it is
   *  mutating, else the one its caller was serving. */
  const stampFor = (fwd: ForwardRequestJson): number | undefined =>
    fwd.mutating ? fwd.token : typeof fwd.caller.forward === 'number' ? fwd.caller.forward : undefined

  /** Forgets ended forwards older than `ENDED_MS`. */
  const pruneEnded = (realm: LiveRealm): void => {
    const now = sched.now()
    for (const [id, e] of realm.ended) {
      if (now - e.at < ENDED_MS) break
      realm.ended.delete(id)
    }
  }
  const noteEnded = (realm: LiveRealm, id: number, answered: boolean): void => {
    pruneEnded(realm)
    realm.ended.delete(id)
    realm.ended.set(id, { at: sched.now(), answered })
    while (realm.ended.size > ENDED_MAX) realm.ended.delete(realm.ended.keys().next().value as number)
  }
  /** Whether forward `id` is remembered as answered by the realm. */
  const wasAnswered = (realm: LiveRealm, id: number): boolean => {
    pruneEnded(realm)
    return realm.ended.get(id)?.answered === true
  }
  /** Whether the host ever handed the realm forward `id`. */
  const issued = (realm: LiveRealm, id: number): boolean => id >= 1 && id <= realm.nextForward

  // ---------------------------------------------------------- activation
  /**
   * Starts `p`'s realm, or joins the start under way. Every caller shares
   * the one start; each forward that joins adds its stamp, so the plugin's
   * connection can say hello inside a compound that forward holds (and a
   * compound that waits for this plugin never waits for its hello too).
   */
  function activate(p: Tracked, reason: ActivationReason, stamp?: number): Promise<LiveRealm | null> {
    // A realm unloading for idleness takes nothing new: the call waits for
    // the unload to end and starts the plugin again (one start for every
    // call that waited).
    const unloading = p.realm?.unloading
    if (unloading !== null && unloading !== undefined) return unloading.done.then(() => activate(p, reason, stamp))
    if (p.state === 'activated' && p.realm !== null) return Promise.resolve(p.realm)
    if (p.state === 'crashed' || storeBlocked(p) || plugins.get(p.id) !== p) return Promise.resolve(null)
    if (stamp !== undefined && !p.joiners.has(stamp)) {
      p.joiners.add(stamp)
      nudge()
    }
    if (p.activation !== null) return p.activation
    // A plugin blocked by its last start (its folder or code could not be
    // read) tries again: the folder may be back.
    setState(p, 'activating')
    p.blocked = null
    const gen = p.gen
    const started = start(p, gen, reason).catch((e: unknown) => {
      if (e instanceof Blocked) {
        if (p.gen === gen && plugins.get(p.id) === p && p.state === 'activating') {
          p.blocked = e.reason
          setState(p, 'blocked', e.reason)
        }
        log('info', `Plugin ${p.id} ${p.version} cannot start now (${e.reason}).`)
        return null
      }
      if (e instanceof NotStarted) {
        if (p.gen === gen && p.state === 'activating') setState(p, 'enabled')
        log('warn', `Plugin ${p.id} ${p.version} did not start: ${e.message}. It starts again when one of its commands is called.`)
        return null
      }
      crash(p, gen, 'start', e instanceof Error ? e.message : String(e))
      return null
    })
    p.activation = started
    void started.finally(() => {
      if (p.gen === gen) {
        p.activation = null
        p.joiners.clear()
      }
      // A start that did not leave a realm running: what was held for it
      // is dropped, and the plugin listens for its events again.
      if (p.realm === null && p.activation === null) {
        noteHeldDropped(p)
        p.held = []
        ensureListener(p)
      }
    })
    return started
  }

  /** The id the shell gave the document. On the desktop it is waited for
   *  until `deadline`; never guessed. On the web it is the kernel's own. */
  async function documentId(p: Tracked, gen: number, deadline: number): Promise<string> {
    let id = pumpDocumentId()
    if (id !== null || env.platform !== 'desktop') return id ?? KERNEL_DEFAULT_DOCUMENT
    while (id === null && Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, 50))
      if (p.gen !== gen) throw new Stale()
      id = pumpDocumentId()
    }
    if (id === null) throw new NotStarted('the document has no id yet')
    return id
  }

  /**
   * One of the opening calls on the plugin's fresh connection. Sent at once
   * while the document is free, or stamped with the innermost compound when
   * a forward waiting for this start holds it; otherwise it waits for that
   * to change, until `deadline`. Never in the window's queue: that queue
   * waits for the document, and the document may be held by a command that
   * waits for this plugin.
   */
  async function startCall(
    p: Tracked,
    gen: number,
    scene: Scene,
    conn: number,
    deadline: number,
    id: string,
    method: string,
    params: unknown,
    /** Runs right after the call succeeded, before any event is drained:
     *  the call goes to the kernel directly, without the dispatch tail. */
    then?: (reply: string) => void,
  ): Promise<void> {
    const frame = JSON.stringify({ jsonrpc: '2.0', id, method, params })
    for (;;) {
      if (p.gen !== gen) throw new Stale()
      if (env.scene() !== scene) throw new NotStarted('the document changed')
      let stamp: number | undefined | null = undefined
      if (scene.busy()) {
        const top = innermostForward(scene)
        stamp = top !== null && p.joiners.has(top) ? top : null
      }
      if (stamp !== null) {
        let reply: string | undefined
        if (then === undefined) {
          const result = dispatchNow(scene, conn, frame, stamp)
          reply = result.kind === 'reply' ? result.frame : undefined
        } else {
          const raw = stamp === undefined ? scene.api_dispatch(conn, frame) : scene.api_dispatch_in_forward(conn, BigInt(stamp), frame)
          const result = parseDispatch(raw)
          reply = result.kind === 'reply' ? result.frame : undefined
          if (reply !== undefined && replySucceeded(reply)) {
            then(reply)
            pumpEvents(scene)
            return
          }
        }
        if (reply !== undefined && replySucceeded(reply)) return
        // The kernel holds a compound this window did not open on top.
        if (reply === undefined || !refusedBusy(reply)) throw new Error(`${method} was refused`)
      }
      const left = deadline - Date.now()
      if (left <= 0) throw new NotStarted('another command held the document for longer than the start timeout')
      await nextNudge(left)
    }
  }

  async function start(p: Tracked, gen: number, reason: ActivationReason): Promise<LiveRealm | null> {
    const manifest = p.manifest as Manifest
    const main = typeof manifest.main === 'string' ? packagePath(manifest.main) : null
    if (main === null) {
      // A folder's manifest may lose its `main` while the person edits it.
      if (p.installed.source.kind === 'dir') throw new Blocked('main_missing')
      throw new Error('the manifest names no main script')
    }
    let code: ArrayBuffer
    try {
      code = await env.store.readFile(p.id, main)
    } catch (e) {
      if (p.gen !== gen) return null
      const block = readBlock(p.installed, e)
      if (block !== null) throw new Blocked(block)
      throw e
    }
    if (p.gen !== gen) return null
    if (code.byteLength > 32 << 20) throw new Error('the main script is larger than 32 MiB')
    // Waiting for the document (its id, and the moment it lets the plugin
    // say hello) is bounded by the start timeout.
    const deadline = Date.now() + timings.startMs
    let document: string
    try {
      document = await documentId(p, gen, deadline)
    } catch (e) {
      if (e instanceof Stale) return null
      throw e
    }
    if (p.gen !== gen) return null
    const scene = env.scene()
    if (scene === null) throw new NotStarted('the document is not ready')

    // The grant comes from the kernel, from the stored record's names.
    const names = grantNames(p.installed)
    const grant = env.grantJson(p.id, document, names, viewer)
    const conn = scene.api_plugin_connection_open(grant)
    const close = (): void => {
      starting.delete(conn)
      try {
        scene.api_connection_close(conn)
      } catch {
        /* the scene is gone */
      }
    }
    // Events owed to this connection before the realm is ready are held.
    starting.set(conn, p)
    try {
      await startCall(p, gen, scene, conn, deadline, 'host-hello', 'hew.meta.hello', { protocol: 1, client: { name: p.id, version: p.version } })
      await startCall(p, gen, scene, conn, deadline, 'host-attach', 'hew.doc.attach', {})
      // The handover from the listener: the realm's connection subscribes
      // to the events the listener had, and the listener closes, in one
      // step no event drain comes between. Every event drained before went
      // to the listener and is held; every one after comes on this
      // connection and is held until the realm is ready. None is lost and
      // none arrives twice. It asks for every starting event whatever the
      // listener has listed: the realm's grant is the current one, a
      // listener still opening may have subscribed already without having
      // read the reply, and one whose widened grant's subscribe is still
      // waiting has not listed the newly allowed ones yet (names its grant
      // refuses come back denied, nothing done). A listener still opening
      // finds itself replaced and stops; the realm is subscribed either way.
      const listener = p.listener
      if (listener !== null || p.listenerOpening !== null) {
        await startCall(p, gen, scene, conn, deadline, 'host-events', 'hew.event.subscribe', { events: startingEvents(p.manifest) }, () => {
          if (p.listener === listener) closeListener(p, true)
        })
      }
    } catch (e) {
      close()
      if (e instanceof Stale) return null
      throw e
    }
    if (p.gen !== gen) {
      close()
      return null
    }

    let channel: RealmChannel
    try {
      channel = await env.openRealm(code, () => crash(p, gen, 'navigated', 'the realm frame loaded another page'))
    } catch (e) {
      close()
      throw e
    }
    if (p.gen !== gen) {
      channel.remove()
      close()
      return null
    }
    starting.delete(conn)
    const realm: LiveRealm = {
      channel,
      conn,
      document,
      grant,
      names,
      lastActivity: sched.now(),
      unloading: null,
      requests: 0,
      meter: new RateMeter(() => sched.now()),
      ready: false,
      nextForward: 0,
      forwards: new Map(),
      ended: new Map(),
      storageIds: new Set(),
      fileIds: new Set(),
      files: Promise.resolve(),
      fileReads: [],
      inbox: [],
      inboxHead: 0,
      queuedBus: 0,
      draining: false,
      waiting: 0,
      pingN: 0,
      unanswered: 0,
      timers: [],
      consoleWindow: 0,
      consoleLines: 0,
      consoleRun: 0,
      consoleLast: -Infinity,
      consoleMuted: false,
    }
    p.realm = realm
    byConn.set(conn, p)
    channel.control.onmessage = (ev) => guarded(p, gen, () => onControl(p, gen, ev))
    channel.port.onmessage = (ev) => guarded(p, gen, () => onRealm(p, gen, ev))
    const unreadable = (): void => violation(p, gen, 'a message that could not be read')
    channel.control.onmessageerror = unreadable
    channel.port.onmessageerror = unreadable

    return new Promise<LiveRealm | null>((resolve) => {
      const startTimer = setTimeout(() => {
        crash(p, gen, 'start', 'the plugin did not start in time')
        resolve(null)
      }, timings.startMs)
      realm.timers.push(startTimer)
      realm.onReady = () => {
        clearTimeout(startTimer)
        setState(p, 'activated')
        // What was held for the realm, in the order it came, before
        // anything else the host sends it.
        noteHeldDropped(p)
        const held = p.held
        p.held = []
        for (const frame of held) post(p, { t: 'bus', frame })
        try {
          notePluginState(scene, p.id, 'loaded')
        } catch {
          /* the kernel refuses an id it cannot read; nothing to report */
        }
        pumpEvents(scene)
        log('info', `Plugin ${p.id} ${p.version} started (${reason}).`)
        realm.timers.push(
          setInterval(() => {
            if (p.gen !== gen) return
            if (realm.unloading !== null) return
            if (realm.unanswered >= 2) {
              crash(p, gen, 'unresponsive', 'two pings went unanswered')
              return
            }
            realm.unanswered++
            post(p, { t: 'ping', n: ++realm.pingN })
            if (idle(p, realm)) unloadIdle(p, gen, realm)
          }, timings.pingMs),
        )
        // A grant widened while it started applies now.
        applyGrant(p)
        resolve(realm)
      }
      realm.onGone = () => resolve(null)
      post(p, {
        t: 'hello',
        plugin: p.id,
        version: p.version,
        realm: 'background',
        document,
        document_id: session.id,
        document_epoch: documentEpoch,
        locale: env.locale(),
        theme: env.theme(),
        grant,
        api: 1,
        activation: reason,
        features: [],
      })
    })
  }

  // ------------------------------------------------------------ messages
  /** Runs a handler for a realm message; anything it throws stops the realm. */
  function guarded(p: Tracked, gen: number, fn: () => void): void {
    if (p.gen !== gen) return
    try {
      fn()
    } catch (e) {
      crash(p, gen, 'host', `the host could not handle a message: ${e instanceof Error ? e.message : String(e)}`)
    }
  }

  function violation(p: Tracked, gen: number, reason: string): void {
    crash(p, gen, 'violation', reason)
  }

  function onControl(p: Tracked, gen: number, ev: MessageEvent): void {
    const realm = p.realm
    if (realm === null) return
    if (ev.ports.length > 0) return violation(p, gen, 'the frame sent a port')
    const checked = checkBootMessage(ev.data)
    if (!checked.ok) return violation(p, gen, `the frame sent ${checked.reason}`)
    const over = realm.meter.add(checked.size)
    if (over !== null) return violation(p, gen, over)
    if (checked.msg.t === 'error') crash(p, gen, 'error', checked.msg.message)
  }

  /**
   * A message from the realm's Worker, as it arrives: checked, counted, and
   * queued. Only `pong` (the Worker's sign of life), `hello_ack` (before
   * which nothing else may come) and `error` (which stops the realm) are
   * acted on at once. Everything else waits in the realm's inbox and is
   * handled in arrival order by `drain`, a slice at a time, so a burst of
   * costly calls never holds the page for more than a slice, and the
   * per-second limits see every message when it arrives, not when the host
   * got round to it. One inbox per realm keeps a forward's reply behind the
   * calls the plugin made while serving it.
   */
  function onRealm(p: Tracked, gen: number, ev: MessageEvent): void {
    const realm = p.realm
    if (realm === null) return
    if (ev.ports.length > 0) return violation(p, gen, 'the plugin sent a port')
    const checked = checkRealmMessage(ev.data)
    if (!checked.ok) return violation(p, gen, `the plugin sent ${checked.reason}`)
    const over = realm.meter.add(checked.size)
    if (over !== null) return violation(p, gen, `the plugin sent ${over}`)
    const m: RealmToHost = checked.msg
    if (m.t === 'pong') {
      if (m.n === realm.pingN) realm.unanswered = 0
      return
    }
    realm.lastActivity = sched.now()
    if (realm.unloading !== null) {
      // An idle unload's last moments: the realm's storage writes and its
      // `unloaded` are taken, in order; anything else it sends is dropped.
      if (m.t !== 'storage' && m.t !== 'unloaded') return
      realm.inbox.push(m)
      if (!realm.draining) {
        realm.draining = true
        sched.later(() => guarded(p, gen, () => drain(p, gen, realm)))
      }
      return
    }
    if (!realm.ready) {
      if (m.t !== 'hello_ack') return violation(p, gen, `the plugin sent ${m.t} before it started`)
      realm.ready = true
      realm.onReady?.()
      return
    }
    if (m.t === 'error') return crash(p, gen, 'error', m.message)
    if (m.t === 'bus') {
      if (realm.queuedBus + realm.waiting >= MAX_PENDING_BUS) return violation(p, gen, 'the plugin queued too many calls')
      realm.queuedBus++
    }
    realm.inbox.push(m)
    if (!realm.draining) {
      realm.draining = true
      sched.later(() => guarded(p, gen, () => drain(p, gen, realm)))
    }
  }

  /** Handles the realm's queued messages in order for at most `sliceMs`,
   *  then yields to the page and carries on in a later task. */
  function drain(p: Tracked, gen: number, realm: LiveRealm): void {
    const start = sched.now()
    while (realm.inboxHead < realm.inbox.length) {
      const m = realm.inbox[realm.inboxHead]
      realm.inboxHead++
      if (m.t === 'bus') realm.queuedBus--
      handle(p, gen, realm, m)
      if (p.gen !== gen) return
      if (sched.now() - start >= timings.sliceMs) break
    }
    if (realm.inboxHead >= realm.inbox.length) {
      realm.inbox = []
      realm.inboxHead = 0
      realm.draining = false
      return
    }
    if (realm.inboxHead > 1024) {
      realm.inbox = realm.inbox.slice(realm.inboxHead)
      realm.inboxHead = 0
    }
    sched.later(() => guarded(p, gen, () => drain(p, gen, realm)))
  }

  /** One queued message from a realm that has started. */
  function handle(p: Tracked, gen: number, realm: LiveRealm, m: RealmToHost): void {
    switch (m.t) {
      case 'hello_ack':
        return violation(p, gen, 'the plugin started twice')
      case 'bus':
        return onBus(p, gen, realm, m.frame, m.forward)
      case 'forward_reply': {
        const open = realm.forwards.get(m.id)
        if (open === undefined) {
          if (!issued(realm, m.id)) return violation(p, gen, 'the plugin answered a forward it was not given')
          if (wasAnswered(realm, m.id)) return violation(p, gen, 'the plugin answered a forward twice')
          // A late answer to a forward the host gave up on (or one ended so
          // long ago it is no longer remembered): dropped.
          return
        }
        const reply = readForwardReply(m.reply)
        if (reply === null) return violation(p, gen, 'the plugin answered with something that is not a reply')
        realm.forwards.delete(m.id)
        noteEnded(realm, m.id, true)
        open.settle(reply)
        return
      }
      case 'forward_progress':
        // A handler may report progress for as long as it runs, after its
        // forward was given up included.
        if (!issued(realm, m.id)) return violation(p, gen, 'the plugin reported progress on a forward it was not given')
        return
      case 'storage':
        onStorage(p, gen, realm, m)
        // While an idle unload waits, a write goes to the store at once.
        if (realm.unloading !== null && p.gen === gen) void flushWrites(p)
        return
      case 'files':
        return onFiles(p, gen, realm, m)
      case 'console':
        return onConsole(p, realm, m)
      case 'error':
        return crash(p, gen, 'error', m.message)
      case 'unloaded':
        if (realm.unloading !== null) finishIdle(p, gen)
        return
      case 'pong':
        return
    }
  }

  /** A console line, into the log within two budgets: a few lines a second,
   *  and a few hundred in one run of output (after which the plugin's
   *  console is not shown until it has been quiet a while). */
  function onConsole(p: Tracked, realm: LiveRealm, m: Extract<RealmToHost, { t: 'console' }>): void {
    const now = sched.now()
    if (now - realm.consoleLast >= CONSOLE_QUIET_MS) {
      realm.consoleRun = 0
      realm.consoleMuted = false
    }
    realm.consoleLast = now
    if (realm.consoleMuted) return
    if (now - realm.consoleWindow >= 1000) {
      realm.consoleWindow = now
      realm.consoleLines = 0
    }
    if (++realm.consoleLines > CONSOLE_LINES_PER_SECOND) return
    if (++realm.consoleRun > CONSOLE_LINES_PER_RUN) {
      realm.consoleMuted = true
      log('warn', `[${p.id}] Further output from this plugin is not shown until it has been quiet for ${CONSOLE_QUIET_MS / 1000} s.`)
      return
    }
    const text = cleanText(m.args.map((a) => cleanText(a, 500)).join(' '), 2_000)
    log(m.level === 'log' ? 'info' : m.level, `[${p.id}] ${text}`)
    if (env.console !== undefined) {
      p.consoleOut.push({ level: m.level, text, at: Date.now() })
      p.consoleTimer ??= setTimeout(() => guardedHost(() => flushConsole(p)), 1_000)
    }
  }

  function onBus(p: Tracked, gen: number, realm: LiveRealm, frame: string, forward: number | undefined): void {
    let stamp: number | undefined
    let openForward: OpenForward | undefined
    if (forward !== undefined) {
      const open = realm.forwards.get(forward)
      if (open === undefined) {
        if (!issued(realm, forward)) return violation(p, gen, 'the plugin named a forward it was not given')
        // The forward was answered, cancelled, or ended under a nested
        // command: the call is refused, never run as a write of its own.
        const reply = forwardEndedReply(frame)
        if (reply !== undefined) post(p, { t: 'bus', frame: reply })
        return
      }
      openForward = open
      stamp = stampFor(open.fwd)
    }
    // A plugin's own request for permissions is this host's to answer; it
    // never reaches the kernel, and touches no document.
    if (frameMethod(frame) === PERMISSION_REQUEST) return permissionRequest(p, gen, realm, frame, openForward)
    realm.waiting++
    dispatch(realm.conn, frame, stamp).then(
      (reply) => {
        realm.waiting--
        if (p.gen === gen && reply !== undefined) post(p, { t: 'bus', frame: reply })
      },
      (e: unknown) => {
        realm.waiting--
        log('error', `a call from ${p.id} failed in the host: ${String(e)}`)
      },
    )
  }

  // ------------------------------------------------------------- storage
  const refusalName = (e: unknown): string => {
    const name = toPluginError(e).refusal
    return REFUSAL_NAME.test(name) ? name : 'storage_unavailable'
  }

  /** One pending write, handed to the store. */
  const writeOne = (p: Tracked, w: PendingWrite): Promise<void> => {
    if (w.scope === 'plugin') return w.value === null ? env.storage.delete(p.id, w.key) : env.storage.set(p.id, w.key, w.value)
    const doc = w.doc as string
    return w.value === null ? env.storage.docDelete(p.id, doc, w.key) : env.storage.docSet(p.id, doc, w.key, w.value)
  }

  /**
   * Hands the writes not yet made to the store, as one batch after every
   * earlier one, so writes to one plugin's storage land in the order the
   * plugin made them, each under the document it was made for. Resolves
   * once this batch and every earlier one is done. Each write's failure
   * goes to the requests that made it; a carried write that fails is kept
   * (`unmigrated`) for reads and tried again with the plugin's next write.
   */
  function flushWrites(p: Tracked): Promise<void> {
    if (p.writeTimer !== null) {
      clearTimeout(p.writeTimer)
      p.writeTimer = null
    }
    if (p.writes.size === 0) return p.flushed
    const batch = [...p.writes.values()]
    p.writes.clear()
    p.flushed = p.flushed
      .then(async () => {
        for (const w of batch) {
          let error: string | null = null
          try {
            await writeOne(p, w)
          } catch (e) {
            error = refusalName(e)
          }
          if (error !== null && w.carried && w.value !== null && !p.writes.has(slotOf(w.scope, w.doc, w.key))) {
            const kept = p.unmigrated.get(w.doc as string) ?? new Map<string, string>()
            kept.set(w.key, w.value)
            p.unmigrated.set(w.doc as string, kept)
            log('warn', `Plugin ${p.id}: data kept for an unsaved document could not be written under its new id yet (${error}); it is tried again with the next write.`)
          }
          for (const done of w.waiters) done(error)
        }
      })
      .catch((e: unknown) => log('error', `writing the storage of ${p.id} failed in the host: ${String(e)}`))
    return p.flushed
  }

  /** Queues a write; the store gets it with the others after the debounce. */
  function queueWrite(p: Tracked, w: Omit<PendingWrite, 'waiters'>, waiter?: (error: string | null) => void): void {
    // Carried data that could not be written yet goes first, so a later
    // write of the same key still wins.
    if (p.unmigrated.size > 0) {
      const retry = p.unmigrated
      p.unmigrated = new Map()
      for (const [doc, kept] of retry) {
        for (const [key, value] of kept) {
          const slot = slotOf('document', doc, key)
          if (!p.writes.has(slot)) p.writes.set(slot, { scope: 'document', doc, key, value, waiters: [], carried: true })
        }
      }
    }
    const slot = slotOf(w.scope, w.doc, w.key)
    const entry = p.writes.get(slot) ?? { ...w, waiters: [] }
    // Re-inserted, so the batch keeps the order of the latest writes.
    p.writes.delete(slot)
    entry.value = w.value
    entry.carried = w.carried && entry.waiters.length === 0
    if (waiter !== undefined) entry.waiters.push(waiter)
    p.writes.set(slot, entry)
    if (w.scope === 'document') p.unmigrated.get(w.doc as string)?.delete(w.key)
    p.writeTimer ??= setTimeout(() => void flushWrites(p), timings.storageDebounceMs)
  }

  // ------------------------------------------------------ the document
  /**
   * The document this window has now, as plugin storage knows it: its
   * persistent id (the `.hew` file's, made by its first save), or null
   * while it has none. A document without an id keeps each plugin's
   * document storage here, in memory, until its first save carries it
   * under the new id; a new or opened document starts a new session.
   */
  interface DocSession {
    id: string | null
    /** By plugin id: key to value, and the JSON size of the whole. */
    data: Map<string, { values: Map<string, string>; bytes: number }>
    /** The session a close leaves (see `documentClosed`): it belongs to no
     *  document, so no save carries its data anywhere. */
    throwaway?: true
  }
  const currentDocumentId = (): string | null => {
    try {
      const id = (env.scene() as unknown as { document_id?: () => string | undefined } | null)?.document_id?.()
      return typeof id === 'string' && DOCUMENT_ID.test(id) ? id : null
    } catch {
      return null
    }
  }
  let session: DocSession = { id: currentDocumentId(), data: new Map() }
  /**
   * The count of document changes (`DocumentEpoch`): bumped whenever
   * document storage starts to mean another document (one opened or made,
   * or a save that moves a saved document to another id). A realm stamps
   * each document storage call with the epoch it last heard (in `hello`
   * and each `document` message), and a call stamped with an older one is
   * refused `document_changed`, nothing done: it was made for a document
   * this window no longer has, and handling it now would put it in the
   * new one. The first save of a document does not bump it: the data the
   * session held moves to the id that save gives, so a call made before
   * the save is still for the right document.
   */
  let documentEpoch = 0

  /** A close's news still to be told (see `documentClosed`). */
  let closeUntold = false
  /** Tells every realm that has started which document it has now. */
  const tellDocument = (): void => {
    closeUntold = false
    for (const p of plugins.values()) if (p.realm !== null) post(p, { t: 'document', document_id: session.id, epoch: documentEpoch })
  }

  /**
   * The document closed, and no other has opened yet (a `.skp` import
   * closes the old document and announces the new one only when the
   * import ends). Writes on their way land under the closed document,
   * and document storage then means no document: calls made before are
   * refused `document_changed`, and calls made after go to a throwaway
   * session, with no id, that the next opened document replaces. The
   * realms are told after the current task, and only if no document was
   * opened in it meanwhile: an ordinary open (closed, then opened) tells
   * them once, of the new document.
   */
  function documentClosed(): void {
    for (const p of plugins.values()) void flushWrites(p)
    session = { id: null, data: new Map(), throwaway: true }
    documentEpoch++
    closeUntold = true
    queueMicrotask(() => {
      if (closeUntold && !disposed) tellDocument()
    })
  }

  /** A new or opened document: writes on their way still land under the
   *  document they were made for, the unsaved one's data is dropped, and
   *  every realm is told. */
  function documentOpened(id: string | null): void {
    for (const p of plugins.values()) void flushWrites(p)
    session = { id, data: new Map() }
    documentEpoch++
    tellDocument()
  }

  /**
   * A save. The first save of a document gives it its id: what the plugins
   * kept for it in memory is written under that id, key by key (a key
   * already stored there is overwritten), before any realm is told the id.
   * A write that fails is kept and tried again (see `flushWrites`). The
   * throwaway session a close leaves is never carried: a save that comes
   * while it is current starts a new session for the saved id, and what
   * the throwaway held is dropped.
   */
  async function documentSaved(id: string | null): Promise<void> {
    if (id === null || id === session.id) return
    const before = session
    session = { id, data: new Map() }
    const carry = before.id === null && before.throwaway !== true
    // A saved document that now has another id, or one saved after a
    // close, is another document for storage; the first save of an
    // unsaved one keeps its epoch (above).
    if (!carry) documentEpoch++
    if (carry) {
      const waits: Promise<void>[] = []
      for (const [pluginId, kept] of before.data) {
        const p = plugins.get(pluginId)
        if (p === undefined) continue
        for (const [key, value] of kept.values) queueWrite(p, { scope: 'document', doc: id, key, value, carried: true })
        waits.push(flushWrites(p))
      }
      await Promise.all(waits)
      // Another document came while the data was written: its realms were
      // told already.
      if (session.id !== id) return
    }
    tellDocument()
  }

  const unregisterLifecycle = registerBroadcastSink((frame) => {
    guardedHost(() => {
      const parsed = JSON.parse(frame) as { method?: unknown; params?: { document_id?: unknown } }
      const raw = parsed.params?.document_id
      const id = typeof raw === 'string' && DOCUMENT_ID.test(raw) ? raw : null
      if (parsed.method === 'hew.event.document_opened') documentOpened(id)
      else if (parsed.method === 'hew.event.document_closed') documentClosed()
      else if (parsed.method === 'hew.event.document_saved') void documentSaved(id)
    })
  })

  /** Document storage while the document has no id: in memory, here. */
  function sessionStorage(p: Tracked, m: Extract<RealmToHost, { t: 'storage' }>, reply: (msg: HostToRealm) => void): void {
    const kept = session.data.get(p.id) ?? { values: new Map<string, string>(), bytes: 2 }
    const entryBytes = (key: string, value: string): number => utf8Length(JSON.stringify(key)) + utf8Length(JSON.stringify(value)) + 2
    const key = m.key as string
    switch (m.op) {
      case 'get':
        return reply({ t: 'storage_reply', id: m.id, scope: 'document', ok: true, value: kept.values.get(key) ?? null })
      case 'list':
        return reply({ t: 'storage_reply', id: m.id, scope: 'document', ok: true, keys: [...kept.values.keys()].sort(compareCodePoints) })
      case 'set': {
        const value = m.value as string
        const old = kept.values.get(key)
        const bytes = kept.bytes - (old === undefined ? 0 : entryBytes(key, old)) + entryBytes(key, value)
        if (bytes > SESSION_STORAGE_BYTES) return reply({ t: 'storage_reply', id: m.id, scope: 'document', ok: false, error: 'storage_quota' })
        kept.values.set(key, value)
        kept.bytes = bytes
        session.data.set(p.id, kept)
        return reply({ t: 'storage_reply', id: m.id, scope: 'document', ok: true })
      }
      case 'delete': {
        const old = kept.values.get(key)
        if (old !== undefined) {
          kept.values.delete(key)
          kept.bytes -= entryBytes(key, old)
        }
        return reply({ t: 'storage_reply', id: m.id, scope: 'document', ok: true })
      }
    }
  }

  function onStorage(p: Tracked, gen: number, realm: LiveRealm, m: Extract<RealmToHost, { t: 'storage' }>): void {
    if (realm.storageIds.has(m.id)) return violation(p, gen, 'the plugin reused a storage request id')
    if (realm.storageIds.size >= 64) return violation(p, gen, 'the plugin has too many storage requests open')
    realm.storageIds.add(m.id)
    const scope = m.scope ?? 'plugin'
    const reply = (msg: HostToRealm): void => {
      realm.storageIds.delete(m.id)
      if (p.gen === gen) post(p, msg)
    }
    const fail = (e: unknown): void => reply({ t: 'storage_reply', id: m.id, scope, ok: false, error: refusalName(e) })
    // Made for a document this window no longer has: refused, nothing done.
    if (scope === 'document' && m.epoch !== documentEpoch) return reply({ t: 'storage_reply', id: m.id, scope, ok: false, error: 'document_changed' })
    // A value no store can keep (a lone surrogate: the SDK's JSON never
    // has one, only a raw message can): refused by name here, before
    // either store (or the unsaved document's session) sees it, so the
    // web and the desktop answer alike.
    if (m.op === 'set' && !wellFormed(m.value as string)) return reply({ t: 'storage_reply', id: m.id, scope, ok: false, error: 'storage_value_invalid' })
    // The document the request is for: the one the window has now, which
    // is the one it was made for (same epoch).
    const doc = scope === 'document' ? session.id : null
    if (scope === 'document' && doc === null) return sessionStorage(p, m, reply)
    switch (m.op) {
      case 'get': {
        const key = m.key as string
        const pending = p.writes.get(slotOf(scope, doc, key))
        if (pending !== undefined) {
          reply({ t: 'storage_reply', id: m.id, scope, ok: true, value: pending.value })
          return
        }
        const kept = doc === null ? undefined : p.unmigrated.get(doc)?.get(key)
        if (kept !== undefined) {
          reply({ t: 'storage_reply', id: m.id, scope, ok: true, value: kept })
          return
        }
        // After the batches already handed to the store, so the read sees
        // every earlier write (or, when one failed, what is really stored).
        p.flushed
          .then(() => (doc === null ? env.storage.get(p.id, key) : env.storage.docGet(p.id, doc, key)))
          .then((value) => reply({ t: 'storage_reply', id: m.id, scope, ok: true, value }), fail)
        return
      }
      case 'set':
      case 'delete': {
        queueWrite(p, { scope, doc, key: m.key as string, value: m.op === 'set' ? (m.value as string) : null, carried: false }, (error) =>
          reply(error === null ? { t: 'storage_reply', id: m.id, scope, ok: true } : { t: 'storage_reply', id: m.id, scope, ok: false, error }),
        )
        return
      }
      case 'list':
        flushWrites(p)
          .then(() => (doc === null ? env.storage.list(p.id) : env.storage.docList(p.id, doc)))
          .then((keys) => {
            const kept = doc === null ? undefined : p.unmigrated.get(doc)
            const all = kept === undefined || kept.size === 0 ? keys : [...new Set([...keys, ...kept.keys()])].sort(compareCodePoints)
            reply({ t: 'storage_reply', id: m.id, scope, ok: true, keys: all })
          }, fail)
        return
    }
  }

  // --------------------------------------------------------------- files
  /** Package bytes `realm` had read for it in the last second. */
  const filesReadLastSecond = (realm: LiveRealm): number => {
    const now = sched.now()
    while (realm.fileReads.length > 0 && now - realm.fileReads[0].at >= 1000) realm.fileReads.shift()
    let total = 0
    for (const r of realm.fileReads) total += r.bytes
    return total
  }

  /**
   * Package files for the realm. Requests are served one at a time; each
   * path is read once however often it is named, and is answered once,
   * among the entries or the missing; a request carries at most
   * `FILES_BYTES_PER_REQUEST`, and a realm has at most
   * `FILES_BYTES_PER_SECOND` read for it in any second. What is over a
   * bound is answered missing.
   */
  function onFiles(p: Tracked, gen: number, realm: LiveRealm, m: Extract<RealmToHost, { t: 'files' }>): void {
    if (realm.fileIds.has(m.id)) return violation(p, gen, 'the plugin reused a file request id')
    if (realm.fileIds.size >= 16) return violation(p, gen, 'the plugin has too many file requests open')
    realm.fileIds.add(m.id)
    realm.files = realm.files.then(async () => {
      const entries: { path: string; bytes: ArrayBuffer }[] = []
      const missing: string[] = []
      const seen = new Set<string>()
      let total = 0
      for (const path of m.paths) {
        if (seen.has(path)) continue
        seen.add(path)
        const rel = packagePath(path)
        if (rel === null || p.gen !== gen || filesReadLastSecond(realm) >= FILES_BYTES_PER_SECOND) {
          missing.push(path)
          continue
        }
        try {
          const bytes = await env.store.readFile(p.id, rel)
          realm.fileReads.push({ at: sched.now(), bytes: bytes.byteLength })
          if (bytes.byteLength > LIMITS.transferBytes || total + bytes.byteLength > FILES_BYTES_PER_REQUEST || filesReadLastSecond(realm) > FILES_BYTES_PER_SECOND) {
            missing.push(path)
            continue
          }
          total += bytes.byteLength
          entries.push({ path, bytes })
        } catch {
          missing.push(path)
        }
      }
      realm.fileIds.delete(m.id)
      if (p.gen === gen) post(p, { t: 'files', id: m.id, entries, missing }, entries.map((e) => e.bytes))
    }).catch((e: unknown) => {
      realm.fileIds.delete(m.id)
      log('error', `a file request from ${p.id} failed in the host: ${String(e)}`)
      // The plugin's request is still answered: everything missing.
      if (p.gen === gen) post(p, { t: 'files', id: m.id, entries: [], missing: [...new Set(m.paths)] })
    })
  }

  // ------------------------------------------------------------ teardown
  type Ending = { kind: 'crash'; reason: string; detail: string } | { kind: 'unload'; reason: UnloadReason }

  function teardown(p: Tracked, ending: Ending): void {
    p.gen++
    p.activation = null
    p.joiners.clear()
    p.held = []
    p.heldDropped = 0
    closeListener(p)
    for (const [conn, q] of [...starting]) {
      if (q !== p) continue
      starting.delete(conn)
      try {
        env.scene()?.api_connection_close(conn)
      } catch {
        /* the scene is gone */
      }
    }
    const realm = p.realm
    p.realm = null
    const scene = env.scene()
    if (realm !== null) {
      for (const t of realm.timers) clearTimeout(t)
      // What the realm sent and the host has not handled yet is dropped.
      realm.inbox = []
      realm.inboxHead = 0
      realm.queuedBus = 0
      const failed: ForwardReplyJson = { failed: ending.kind === 'crash' ? 'crashed' : 'unavailable' }
      for (const open of [...realm.forwards.values()]) open.settle(failed)
      realm.forwards.clear()
      // An idle unload told the realm already.
      if (ending.kind === 'unload' && realm.unloading === null) {
        try {
          realm.channel.port.postMessage({ t: 'unload', reason: ending.reason } satisfies HostToRealm)
        } catch {
          /* already gone */
        }
      }
      realm.channel.port.onmessage = null
      realm.channel.control.onmessage = null
      realm.channel.port.onmessageerror = null
      realm.channel.control.onmessageerror = null
      try {
        realm.channel.port.close()
      } catch {
        /* already closed */
      }
      realm.channel.remove()
      byConn.delete(realm.conn)
      realm.onGone?.()
      // Calls that waited for an idle unload to end go on (and start the
      // plugin again, if it may still run).
      realm.unloading?.resolve()
      try {
        scene?.api_connection_close(realm.conn)
      } catch {
        /* the scene is gone */
      }
    }
    // Writes the plugin made before it stopped still land, and so do its
    // last console lines.
    if (p.writes.size > 0) void flushWrites(p)
    flushConsole(p)
    p.closeRequest?.()
    if (realm !== null && ending.kind === 'unload') report(p, ending.reason === 'idle' ? 'idle' : 'unloaded', ending.reason)
    // A start waiting for the document sees the teardown.
    nudge()
    if (scene !== null && realm !== null) {
      try {
        // The detail is the machine word only (the one the shell's report
        // carries): every subscriber gets this event, other plugins and
        // socket clients included, so what the plugin wrote never rides
        // along. Its own text stays in its console and the log.
        notePluginState(scene, p.id, ending.kind === 'crash' ? 'crashed' : 'unloaded', ending.reason)
        pumpEvents(scene)
      } catch {
        /* nothing to report it to */
      }
    }
  }

  /** Stops a realm that misbehaved. Its commands are unregistered until the
   *  plugin is turned off and on again (or the window reloads). */
  function crash(p: Tracked, gen: number, reason: string, detail: string): void {
    if (p.gen !== gen || p.state === 'crashed' || plugins.get(p.id) !== p) return
    teardown(p, { kind: 'crash', reason, detail })
    setState(p, 'crashed', reason)
    try {
      env.scene()?.unregister_plugin_commands(p.id)
    } catch {
      /* the scene is gone */
    }
    log('error', `Plugin ${p.id} ${p.version} stopped (${reason}): ${cleanText(detail, 300)}`)
    try {
      // Shop has no Settings > Plugins; a reload starts a new host there,
      // where the plugin is no longer stopped.
      env.toast(
        env.platform === 'shop'
          ? `${p.name} stopped working. Reload the page to start it again.`
          : `${p.name} stopped working. To start it again, turn it off and on in Settings > Plugins.`,
      )
    } catch {
      /* a toast that fails must not stop the host */
    }
  }

  // ---------------------------------------------------------- idle unload
  /** Whether `realm` may be unloaded for idleness now: it runs, nothing is
   *  open or queued either way, and nothing went either way for `idleMs`.
   *  A plugin that starts with Hew asked to run, and is never unloaded so. */
  function idle(p: Tracked, realm: LiveRealm): boolean {
    return (
      p.realm === realm &&
      p.state === 'activated' &&
      realm.ready &&
      realm.unloading === null &&
      !startsWithHew(p) &&
      realm.forwards.size === 0 &&
      realm.inboxHead >= realm.inbox.length &&
      !realm.draining &&
      realm.queuedBus === 0 &&
      realm.waiting === 0 &&
      realm.storageIds.size === 0 &&
      realm.fileIds.size === 0 &&
      realm.requests === 0 &&
      p.joiners.size === 0 &&
      sched.now() - realm.lastActivity >= timings.idleMs
    )
  }

  /** Tells an idle realm to unload, and gives it `drainMs` for its storage
   *  writes and its `unloaded`; the ordinary teardown follows. */
  function unloadIdle(p: Tracked, gen: number, realm: LiveRealm): void {
    let resolve: () => void = () => {}
    const done = new Promise<void>((r) => (resolve = r))
    realm.unloading = { done, resolve }
    log('info', `Plugin ${p.id} ${p.version} unloads: it has been idle for ${Math.round(timings.idleMs / 1000)} s.`)
    try {
      realm.channel.port.postMessage({ t: 'unload', reason: 'idle' } satisfies HostToRealm)
    } catch {
      /* already gone: the timer below ends it */
    }
    // `drainMs` (2 s) is longer than the SDK's `unloadWaitMs` (1.5 s, the
    // most a realm waits for its unload listeners and their storage writes
    // before it says `unloaded`), so a realm that keeps to its limit is
    // heard out; the half second is for the messages on their way.
    realm.timers.push(setTimeout(() => guardedHost(() => finishIdle(p, gen)), timings.drainMs))
  }

  /** The end of an idle unload: the ordinary teardown (pending writes still
   *  land), the plugin back to `enabled` with its commands registered, and
   *  its listener open again when it has one. */
  function finishIdle(p: Tracked, gen: number): void {
    const realm = p.realm
    if (p.gen !== gen || plugins.get(p.id) !== p || realm === null || realm.unloading === null) return
    // The storage messages the realm sent in time and the host has not got
    // to yet (the drain yields between slices) are handled now, so its last
    // writes land: only what is already in the inbox, nothing waited for.
    // The rest of the inbox is `unloaded` (or nothing: only storage and
    // `unloaded` are taken while it unloads).
    while (realm.inboxHead < realm.inbox.length) {
      const m = realm.inbox[realm.inboxHead++]
      if (m.t === 'storage') onStorage(p, gen, realm, m)
      // A message that broke a rule stopped the plugin.
      if (p.gen !== gen) return
    }
    teardown(p, { kind: 'unload', reason: 'idle' })
    setState(p, 'enabled')
    ensureListener(p)
  }

  // --------------------------------------------------------------- grants
  /**
   * Applies the stored record's grant to `p`'s running realm when it
   * differs from the one the realm holds (a grant that only adds optional
   * permissions; any other change reloads instead): the kernel builds the
   * grant, the realm's connection takes it in place, the realm is told
   * (`grant`), and the change is reported (`grant_changed`). A realm that
   * is starting picks it up when it is ready. Nothing happens if the
   * kernel cannot do it; the plugin is reloaded instead.
   */
  function applyGrant(p: Tracked): void {
    const realm = p.realm
    if (realm === null || !realm.ready || realm.unloading !== null) return
    const names = grantNames(p.installed)
    if (names === realm.names) return
    const scene = env.scene()
    if (scene === null) return
    try {
      const grant = env.grantJson(p.id, realm.document, names, viewer)
      setConnectionGrant(scene, realm.conn, grant)
      realm.names = names
      realm.grant = grant
      post(p, { t: 'grant', grant })
      try {
        notePluginState(scene, p.id, 'grant_changed')
      } catch {
        /* nothing to report it to */
      }
      pumpEvents(scene)
      log('info', `Plugin ${p.id} ${p.version} was granted more while it runs.`)
    } catch (e) {
      log('warn', `Plugin ${p.id} ${p.version}: the new grant could not be applied while it runs (${String(e)}); it is reloaded.`)
      teardown(p, { kind: 'unload', reason: 'reload' })
      setState(p, 'enabled')
      ensureListener(p)
    }
  }

  /** A listener takes a widened grant in place and asks again for the
   *  events its old grant refused. */
  function widenListener(p: Tracked): void {
    const listener = p.listener
    const scene = env.scene()
    if (listener === null || !listener.ready || scene === null) return
    const declared = startingEvents(p.manifest)
    const grant = grantNames(p.installed)
    try {
      setConnectionGrant(scene, listener.conn, env.grantJson(p.id, listener.document, grant, viewer))
      listener.grant = grant
    } catch (e) {
      log('warn', `Plugin ${p.id}: its listener could not take the new grant (${String(e)}).`)
      closeListener(p)
      ensureListener(p)
      return
    }
    if (declared.every((n) => listener.names.includes(n))) return
    void listenerCall(p, listener, scene, 'host-events', 'hew.event.subscribe', { events: declared }).then((reply) => {
      if (p.listener !== listener) return
      const subscribed = subscribedNames(reply)
      if (subscribed !== null) listener.names = declared.filter((n) => subscribed.includes(n))
      // Still nothing it can receive: no listener (the next sync opens
      // one again, with whatever grant the record holds then).
      if (listener.names.length === 0) closeListener(p)
      // Widened again while this asked: the newer grant applies too.
      else if (listener.grant !== grantNames(p.installed)) widenListener(p)
    })
  }

  /**
   * `hew.permissions.request` from `p`'s realm, on its own connection: the
   * only caller this host answers (any other gets the kernel's own
   * refusal). In order: Shop refuses it; the params must hold; every name
   * must be one of the stored manifest's optional permissions; one request
   * per plugin at a time; a minute of no after a no; names already held
   * are granted without asking; the person must have just acted in the
   * plugin's own UI (the stamp is spent); then the person decides. While
   * they do, the clock of the forward the request was made in is stopped,
   * for at most `decisionMs`. A yes is applied to the running realm before
   * the answer, which lists the names asked for that are now held. Every
   * request is answered once, and never after its realm has gone.
   */
  function permissionRequest(p: Tracked, gen: number, realm: LiveRealm, frame: string, forward: OpenForward | undefined): void {
    let parsed: { id?: unknown; params?: unknown }
    try {
      parsed = JSON.parse(frame) as { id?: unknown; params?: unknown }
    } catch {
      return
    }
    if (typeof parsed !== 'object' || parsed === null || !('id' in parsed)) return
    const id = parsed.id
    let answered = false
    const answer = (body: Record<string, unknown>): void => {
      if (answered) return
      answered = true
      if (p.gen === gen && p.realm === realm) post(p, { t: 'bus', frame: JSON.stringify({ jsonrpc: '2.0', id, ...body }) })
    }
    const refuse = (name: string, explanation: string, detail: Record<string, unknown> = {}): void => {
      const reply = refusedFrame(frame, PERMISSION_REQUEST, name, explanation, detail)
      if (reply !== undefined) answer(JSON.parse(reply) as Record<string, unknown>)
    }
    if (env.platform === 'shop') return refuse('not_available_in_shop', 'Plugins cannot ask for permissions in Shop Mode. Turn them on in the editor.')
    const params = parsed.params
    const names = requestedNames(params)
    if (names === null) {
      return answer({ error: { code: -32602, message: 'hew.permissions.request takes {permissions: 1 to 64 permission names, reason?: text of at most 200 characters}' } })
    }
    const optional = Array.isArray(p.manifest.optionalPermissions) ? p.manifest.optionalPermissions : []
    const offenders = names.filter((n) => !optional.includes(n))
    if (offenders.length > 0) {
      return refuse('permission_not_requestable', "These are not among the optional permissions in this plugin's manifest.", { permissions: offenders.map((n) => cleanText(n, 128)) })
    }
    if (requestOpen.has(p.id)) return refuse('request_pending', 'This plugin is already waiting for an answer to a permission request.')
    const until = declinedUntil.get(p.id) ?? -Infinity
    if (Date.now() < until) return refuse('request_declined', 'The person said no to this plugin a moment ago. Ask again later.')
    const held = (): string[] => {
      const q = plugins.get(p.id)
      const have = q === undefined ? [] : [...q.installed.grant, ...q.installed.optional_granted]
      return names.filter((n) => have.includes(n))
    }
    const toAsk = names.filter((n) => !held().includes(n))
    if (toAsk.length === 0) return answer({ result: { granted: names } })
    const actedAt = activations.get(p.id)
    activations.delete(p.id)
    if (actedAt === undefined || sched.now() - actedAt > timings.activationMs) {
      return refuse('no_user_activation', "A plugin can ask for a permission only right after you act in the plugin's own window or panel.")
    }
    const decider = env.grants
    if (decider === undefined) return refuse('request_declined', 'Hew cannot ask for permissions here.')

    requestOpen.add(p.id)
    realm.requests++
    forward?.pause()
    let open = true
    const close = (): void => {
      if (!open) return
      open = false
      clearTimeout(limit)
      requestOpen.delete(p.id)
      realm.requests--
      forward?.resume()
      if (p.closeRequest === close) p.closeRequest = undefined
    }
    p.closeRequest = close
    const limit = setTimeout(() => {
      close()
      refuse('request_declined', `Nobody answered within ${Math.round(timings.decisionMs / 60_000)} minutes.`)
    }, timings.decisionMs)
    let decided: Promise<'granted' | 'declined'>
    try {
      decided = decider.decide(p.id, toAsk)
    } catch (e) {
      decided = Promise.reject(e)
    }
    decided.then(
      async (outcome) => {
        if (outcome !== 'granted') {
          if (open) declinedUntil.set(p.id, Date.now() + timings.declineMs)
          close()
          return refuse('request_declined', 'The person did not grant these permissions.')
        }
        // The store changed: read it, which applies the grant to the
        // running realm, before the plugin hears yes. A list that could not
        // be read is read once more, so the answer is not from before the
        // grant; whatever happens, the request closes and is answered.
        try {
          if (!(await sync())) await sync()
        } catch (e) {
          log('warn', `Plugin ${p.id}: the plugin list could not be read after a grant: ${String(e)}`)
        } finally {
          close()
        }
        answer({ result: { granted: held() } })
      },
      (e: unknown) => {
        close()
        log('warn', `Plugin ${p.id}: its permission request could not be put to the person: ${String(e)}`)
        // Another question is on the screen: the plugin may ask again once
        // it is answered. Anything else (the plugin changed or went, a
        // name it does not declare, nowhere to ask) is a no for now, with
        // no minute of refusals after it, since the person said nothing.
        if (toPluginError(e).refusal === 'confirmation_open') {
          return refuse('request_pending', 'Hew is already asking the person something. Ask again once that is answered.')
        }
        refuse('request_declined', 'Hew could not ask for these permissions.')
      },
    )
  }

  // ------------------------------------------------------------ listeners
  /** Whether `p` should have a listener now: it may run, does not run or
   *  start, and its manifest lists events to start it. */
  function wantsListener(p: Tracked): boolean {
    return !disposed && plugins.get(p.id) === p && p.state === 'enabled' && p.realm === null && p.activation === null && startingEvents(p.manifest).length > 0
  }

  /** The names a successful `hew.event.subscribe` reply lists, or null. */
  function subscribedNames(reply: string | null): string[] | null {
    if (reply === null) return null
    try {
      const subscribed = (JSON.parse(reply) as { result?: { subscribed?: unknown } }).result?.subscribed
      return Array.isArray(subscribed) ? subscribed.filter((n): n is string => typeof n === 'string') : null
    } catch {
      return null
    }
  }

  /**
   * One call on a listener connection, straight to the kernel: plain,
   * waiting while the document is held (a listener never acts inside a
   * command), until it succeeds, is refused, or the listener is no longer
   * `p`'s. Resolves with the reply, or null.
   */
  async function listenerCall(p: Tracked, listener: Listener, scene: Scene, id: string, method: string, params: unknown): Promise<string | null> {
    const frame = JSON.stringify({ jsonrpc: '2.0', id, method, params })
    for (;;) {
      if (p.listener !== listener || env.scene() !== scene) return null
      if (!scene.busy()) {
        const result = parseDispatch(scene.api_dispatch(listener.conn, frame))
        const reply = result.kind === 'reply' ? result.frame : null
        if (reply !== null && !refusedBusy(reply)) return replySucceeded(reply) ? reply : null
      }
      await nextNudge(timings.startMs)
    }
  }

  /** Opens `p`'s listener when it should have one, has none, and is not
   *  opening one already. */
  function ensureListener(p: Tracked): void {
    if (p.listener !== null || p.listenerOpening !== null || !wantsListener(p)) return
    const opening = {}
    p.listenerOpening = opening
    void openListener(p, opening)
      .catch((e: unknown) => {
        log('warn', `Plugin ${p.id}: its events could not be listened for: ${String(e)}`)
        closeListener(p)
      })
      .finally(() => {
        if (p.listenerOpening === opening) p.listenerOpening = null
      })
  }

  async function openListener(p: Tracked, opening: object): Promise<void> {
    // The document's id: on the desktop it comes from the shell, shortly.
    // One wait at a time per plugin; a teardown or a close ends it.
    while (env.platform === 'desktop' && pumpDocumentId() === null) {
      await new Promise((r) => setTimeout(r, 250))
      if (p.listenerOpening !== opening || !wantsListener(p) || p.listener !== null) return
    }
    const scene = env.scene()
    if (p.listenerOpening !== opening || scene === null || !wantsListener(p) || p.listener !== null) return
    const document = pumpDocumentId() ?? KERNEL_DEFAULT_DOCUMENT
    const declared = startingEvents(p.manifest)
    const grant = grantNames(p.installed)
    const conn = scene.api_plugin_connection_open(env.grantJson(p.id, document, grant, viewer))
    const listener: Listener = { conn, document, grant, names: [], ready: false }
    p.listener = listener
    byListener.set(conn, p)
    const ok =
      (await listenerCall(p, listener, scene, 'host-hello', 'hew.meta.hello', { protocol: 1, client: { name: p.id, version: p.version } })) !== null &&
      (await listenerCall(p, listener, scene, 'host-attach', 'hew.doc.attach', {})) !== null
    if (p.listener !== listener) return
    const subscribed = ok ? subscribedNames(await listenerCall(p, listener, scene, 'host-events', 'hew.event.subscribe', { events: declared })) : null
    if (p.listener !== listener) return
    if (subscribed === null) {
      log('warn', `Plugin ${p.id}: its events could not be listened for.`)
      closeListener(p)
      return
    }
    // Names the grant cannot receive come back denied; they are not
    // listened for.
    listener.names = declared.filter((n) => subscribed.includes(n))
    listener.ready = true
    // A grant widened while it opened (`widenListener` waits for `ready`)
    // applies now: the connection takes it and asks again for what the
    // old one refused, and closes only if the new one gets none either.
    if (listener.grant !== grantNames(p.installed)) return widenListener(p)
    if (listener.names.length === 0) closeListener(p)
  }

  /** Closes `p`'s listener, and stops one being opened. The events it held
   *  for a start that has not come are dropped (they would reach a later
   *  start stale), unless `keepHeld`: the start that closes it hands them
   *  on. */
  function closeListener(p: Tracked, keepHeld = false): void {
    p.listenerOpening = null
    if (p.eventStarts.timer !== null) {
      clearTimeout(p.eventStarts.timer)
      p.eventStarts.timer = null
    }
    if (!keepHeld && p.realm === null && p.activation === null) {
      p.held = []
      p.heldDropped = 0
    }
    const listener = p.listener
    if (listener === null) return
    p.listener = null
    byListener.delete(listener.conn)
    try {
      env.scene()?.api_connection_close(listener.conn)
    } catch {
      /* the scene is gone */
    }
  }

  /** An event the listener heard: kept, and the plugin started (now, or
   *  when the least time between two event starts has passed). */
  function onListenerFrame(p: Tracked, conn: number, frame: string): void {
    if (p.listener?.conn !== conn || !p.listener.ready) return
    hold(p, frame)
    eventStart(p)
  }

  function eventStart(p: Tracked): void {
    if (p.activation !== null || p.realm !== null || p.state !== 'enabled' || plugins.get(p.id) !== p) return
    const starts = p.eventStarts
    const now = Date.now()
    if (now < starts.nextAt) {
      starts.timer ??= setTimeout(() => {
        starts.timer = null
        guardedHost(() => eventStart(p))
      }, starts.nextAt - now)
      return
    }
    starts.nextAt = now + starts.gapMs
    void activate(p, 'event').then((realm) => {
      // After a start that worked the gap is the least again; after one
      // that did not, it doubles, counted from that start.
      if (realm !== null) starts.gapMs = timings.eventStartMs
      else {
        starts.gapMs = Math.min(starts.gapMs * 2, timings.eventStartMaxMs)
        starts.nextAt = Math.max(starts.nextAt, now + starts.gapMs)
      }
    })
  }

  /** Logs, once per start, how many held frames were dropped for room. */
  function noteHeldDropped(p: Tracked): void {
    if (p.heldDropped === 0) return
    log('warn', `Plugin ${p.id}: ${p.heldDropped} events that came while it started were dropped (it keeps the last ${MAX_HELD_EVENTS}).`)
    p.heldDropped = 0
  }

  // ---------------------------------------------------------------- sync
  const register = (p: Tracked): boolean => {
    const scene = env.scene()
    if (scene === null) return false
    const commands = (p.manifest as Manifest).contributes?.commands
    try {
      scene.unregister_plugin_commands(p.id)
      scene.register_plugin_commands(p.id, JSON.stringify(Array.isArray(commands) ? commands : []))
      return true
    } catch (e) {
      log('error', `Plugin ${p.id} ${p.version}: its commands could not be registered: ${String(e)}`)
      return false
    }
  }

  const keyOf = (installed: Installed): string =>
    JSON.stringify([installed.version, [...installed.grant].sort(), [...installed.optional_granted].sort()])

  /** Whether `p` is blocked by what the store says, which only the store
   *  lifts. */
  function storeBlocked(p: Tracked): boolean {
    return p.state === 'blocked' && p.blocked !== null && STORE_BLOCKS.has(p.blocked)
  }

  /** Whether `p` starts with Hew: it asks to and the person allowed it. */
  function startsWithHew(p: Tracked): boolean {
    const activation = (p.manifest as Manifest).activation
    return p.installed.startup_allowed && Array.isArray(activation) && activation.includes('on_startup')
  }

  /** Blocks `p` for `reason`, stopping a realm it has or is starting. Its
   *  commands stay registered and answer `unavailable`. */
  function block(p: Tracked, reason: BlockReason): void {
    if (p.state === 'crashed') return
    const running = p.realm !== null || p.activation !== null || p.state === 'activating' || p.state === 'activated'
    if (running) teardown(p, { kind: 'unload', reason: 'disable' })
    closeListener(p)
    // Stopped by a review mark, it starts again if the mark clears; any
    // other block forgets that it ran.
    if (reason === 'needs_review') p.resume = p.resume || running
    else if (STORE_BLOCKS.has(reason)) p.resume = false
    if (p.state !== 'blocked' || p.blocked !== reason) log('info', `Plugin ${p.id} ${p.version} cannot run now (${reason}).`)
    p.blocked = reason
    setState(p, 'blocked', reason)
  }

  /** Developer mode, asked of the shell; off when it cannot say, so a
   *  folder plugin never runs on a guess. */
  async function devModeNow(): Promise<boolean> {
    if (env.store.devMode === undefined) return false
    try {
      return (await env.store.devMode()) === true
    } catch {
      return false
    }
  }

  let syncing: Promise<boolean> = Promise.resolve(true)
  /** How often a reload is tried again after the list could not be read,
   *  and how long the first wait is (each wait is longer). */
  const MAX_LIST_RETRIES = 3
  const LIST_RETRY_MS = 200
  /** Plugins whose next sync reloads them whatever changed. */
  const forced = new Set<string>()
  /** Reads the store and applies it; resolves false when the list could
   *  not be read (or the host is gone), so nothing changed. */
  function sync(): Promise<boolean> {
    syncing = syncing.then(syncNow, syncNow)
    return syncing
  }
  async function reload(id: string): Promise<void> {
    if (!disposed) forced.add(id)
    await sync()
  }

  /** Failed list reads in a row while a reload waited. */
  let listRetries = 0
  async function syncNow(): Promise<boolean> {
    if (disposed) return false
    // Taken before the store is read, so the list is at least as new as
    // the change that asked for the reload; one asked for while this runs
    // waits for the next sync, which it queued.
    const force = new Set(forced)
    forced.clear()
    let listed: Listed[]
    try {
      listed = await env.store.list()
      listRetries = 0
    } catch (e) {
      log('warn', `The plugin list could not be read: ${String(e)}`)
      // A reload that was asked for is not lost: it waits for the next
      // sync, and a few more are asked for, each a little later.
      if (force.size > 0 && !disposed) {
        for (const id of force) forced.add(id)
        if (listRetries < MAX_LIST_RETRIES) {
          listRetries++
          setTimeout(() => void sync(), LIST_RETRY_MS * listRetries)
        }
      }
      return false
    }
    if (disposed) return false
    const devMode = await devModeNow()
    if (disposed) return false
    const enabled = new Map<string, { installed: Installed; manifest: PluginManifest }>()
    for (const l of listed) {
      if (l.kind === 'installed' && l.installed.enabled && l.manifest.id === l.installed.id) enabled.set(l.installed.id, l)
    }
    for (const [id, p] of [...plugins]) {
      if (enabled.has(id)) continue
      teardown(p, { kind: 'unload', reason: 'disable' })
      plugins.delete(id)
      try {
        env.scene()?.unregister_plugin_commands(id)
      } catch {
        /* the scene is gone */
      }
    }
    for (const [id, { installed, manifest }] of enabled) {
      const key = keyOf(installed)
      const blockedBy = storeBlock(installed, devMode)
      let p = plugins.get(id)
      // A change that only adds optional permissions keeps the plugin as it
      // is and applies the grant to it in place; anything else rebuilds it.
      const widened = p !== undefined && p.key !== key && !force.has(id) && p.state !== 'crashed' && onlyAddsOptional(p.installed, installed)
      if (p !== undefined && (p.key === key || widened) && !force.has(id)) {
        p.installed = installed
        p.manifest = manifest
        p.key = key
        if (widened) {
          applyGrant(p)
          widenListener(p)
        }
        if (blockedBy !== null) block(p, blockedBy)
        else if (storeBlocked(p)) {
          // The store lets it run again: it starts if it starts with Hew,
          // or if a review mark stopped it while it ran.
          const resume = p.resume
          p.resume = false
          setState(p, 'enabled')
          p.blocked = null
          if (startsWithHew(p)) startUp(p)
          else if (resume) void activate(p, 'command')
        } else p.resume = false
        continue
      }
      // Running, starting, or stopped by a start that could not read its
      // code (which the change may have fixed); or rebuilt from running by
      // an earlier sync that saw the record change (a folder's refresh
      // tells the store first and asks for the reload after).
      const wasRunning =
        p !== undefined &&
        (p.state === 'activating' || p.state === 'activated' || (p.state === 'blocked' && !storeBlocked(p)) || (p.resume && p.state === 'enabled'))
      if (p !== undefined) teardown(p, { kind: 'unload', reason: 'reload' })
      p = {
        id,
        version: installed.version,
        name: displayName(manifest),
        installed,
        manifest,
        key,
        // Only a folder's new version is followed by the reload that starts
        // it again; a grant change, or a package's update, is not.
        resume: wasRunning && !force.has(id) && installed.source.kind === 'dir' && p !== undefined && p.version !== installed.version,
        state: 'enabled',
        blocked: null,
        realm: null,
        activation: null,
        joiners: new Set(),
        gen: (p?.gen ?? 0) + 1,
        held: [],
        heldDropped: 0,
        listener: null,
        listenerOpening: null,
        eventStarts: { nextAt: p?.eventStarts.nextAt ?? 0, gapMs: p?.eventStarts.gapMs ?? timings.eventStartMs, timer: null },
        writes: p?.writes ?? new Map(),
        writeTimer: null,
        flushed: p?.flushed ?? Promise.resolve(),
        unmigrated: p?.unmigrated ?? new Map(),
        reported: null,
        reportedBy: 0,
        reportRetry: null,
        consoleOut: [],
        consoleTimer: null,
      }
      plugins.set(id, p)
      setState(p, 'enabled')
      if (!register(p)) {
        setState(p, 'crashed')
        continue
      }
      if (blockedBy !== null) {
        block(p, blockedBy)
        continue
      }
      if (startsWithHew(p)) startUp(p)
      else if (wasRunning && force.has(id)) void activate(p, 'command')
    }
    // Every plugin that should listen for its events and does not yet.
    for (const p of plugins.values()) ensureListener(p)
    return true
  }

  function startUp(p: Tracked): void {
    // The start itself waits while a command holds the document.
    if (plugins.get(p.id) === p && p.state === 'enabled') void activate(p, 'startup')
  }

  const unlistenStore = env.store.onChanged(() => void sync())
  // A folder plugin's folder changed. The notice is a hint: the store is
  // read again either way. Files changed with the same permissions reload
  // the plugin; a change the person must review has already marked the
  // record, which the sync reads.
  const unlistenDev =
    env.store.onDevChanged?.((change) => {
      if (!change.manifest_changed && change.problem === null) void reload(change.id)
      else void sync()
    }) ?? (() => {})

  return {
    serve,
    dispatch,
    async sync() {
      await sync()
    },
    reload,
    states() {
      const out: Record<string, RealmState> = {}
      for (const [id, p] of plugins) out[id] = p.state
      return out
    },
    reasons() {
      const out: Record<string, BlockReason> = {}
      for (const [id, p] of plugins) if (p.state === 'blocked' && p.blocked !== null) out[id] = p.blocked
      return out
    },
    noteUserActivation(id) {
      const p = plugins.get(id)
      if (!disposed && p !== undefined && p.state === 'activated' && p.realm?.ready === true) activations.set(id, sched.now())
    },
    listening() {
      return [...plugins.values()]
        .filter((p) => p.listener?.ready === true)
        .map((p) => p.id)
        .sort()
    },
    dispose() {
      if (disposed) return
      disposed = true
      unlistenStore()
      unlistenDev()
      unlistenStorage()
      unregisterSink()
      unregisterLifecycle()
      unsubscribeBusy()
      unsubscribeCompounds()
      for (const p of plugins.values()) {
        if (p.reportRetry !== null) clearTimeout(p.reportRetry)
        p.reportRetry = null
        teardown(p, { kind: 'unload', reason: 'window_closed' })
        try {
          env.scene()?.unregister_plugin_commands(p.id)
        } catch {
          /* the scene is gone */
        }
      }
      plugins.clear()
      for (const w of waiting.splice(0)) w()
      nudge()
    },
  }
}
