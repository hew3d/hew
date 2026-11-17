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
 */

import type { ForwardRequestJson } from '../api/dispatchResult'
import { parseDispatch } from '../api/dispatchResult'
import { getBusy, setBusy, subscribeBusy } from '../api/busyState'
import { pumpDocumentId, pumpEvents, registerEventSink } from '../api/eventPump'
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
  beforeDispatch,
  frameMethod,
  refreshesAfter,
  completeForwardWith,
  type LiveBridgeDeps,
} from '../api/liveBridge'
import type { Scene } from '../wasm/loader'
import type { ActivationReason, HostToRealm, RealmToHost, UnloadReason } from '../../../sdk/plugin/src/protocol'
import { toPluginError } from './pluginError'
import { cleanText, displayName } from './pluginText'
import { checkBootMessage, checkRealmMessage, LIMITS, RateMeter } from './realm/validate'
import type { PluginStorageBackend } from './storageBackend'
import type { PluginStoreAccess } from './storeAccess'
import type { Installed, Listed, PluginManifest } from './types'

/** Where a plugin stands in this window. */
export type RealmState = 'enabled' | 'activating' | 'activated' | 'crashed'

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

export interface HostEnv {
  scene(): Scene | null
  /** The live bridge's deps: the viewport and the app's handlers the
   *  dispatch tail reaches. */
  bridge: LiveBridgeDeps
  platform: 'desktop' | 'web'
  store: PluginStoreAccess
  storage: PluginStorageBackend
  /** `plugin_grant_json` from the kernel module. */
  grantJson(id: string, document: string, namesJson: string): string
  /** Creates a realm frame and starts its Worker on `main`. Rejects when
   *  the frame never answers. */
  openRealm(main: ArrayBuffer, onNavigate: () => void): Promise<RealmChannel>
  toast(message: string): void
  log(level: 'info' | 'warn' | 'error', message: string): void
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
}

export const DEFAULT_TIMINGS: HostTimings = {
  pingMs: 2_000,
  startMs: 10_000,
  storageDebounceMs: 250,
  sliceMs: 8,
}

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
}

/** Reports a plugin's state to the kernel (`plugin_state` events); a kernel
 *  without the method has nobody to report it to. */
function notePluginState(scene: Scene, id: string, state: 'loaded' | 'crashed' | 'unloaded', detail?: string): void {
  const note = (scene as unknown as PluginStateScene).note_plugin_state
  if (typeof note === 'function') note.call(scene, id, state, detail)
}

/** A start that could not go on for a reason that is not the plugin's: it
 *  is answered `unavailable` and the plugin stays enabled. */
class NotStarted extends Error {}

/** A start overtaken by a teardown. */
class Stale extends Error {}

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
  let id: unknown = null
  let method = ''
  try {
    const parsed = JSON.parse(frame) as { id?: unknown; method?: unknown }
    if (typeof parsed !== 'object' || parsed === null) return undefined
    if (!('id' in parsed)) return undefined
    id = parsed.id
    if (typeof parsed.method === 'string') method = parsed.method.slice(0, 256)
  } catch {
    id = null
  }
  return JSON.stringify({
    jsonrpc: '2.0',
    id,
    error: {
      code: -32000,
      message: 'refused',
      data: {
        refusal: FORWARD_ENDED,
        failed_index: 0,
        failed_method: method,
        detail: {},
        explanation: 'The command this call was made for has already ended, so the call was not run.',
      },
    },
  })
}

interface OpenForward {
  fwd: ForwardRequestJson
  settle(reply: ForwardReplyJson): void
}

interface LiveRealm {
  channel: RealmChannel
  conn: number
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
  state: RealmState
  realm: LiveRealm | null
  activation: Promise<LiveRealm | null> | null
  /** The stamps of the forwards waiting for the activation: a compound
   *  one of them holds lets the plugin's connection say hello inside it. */
  joiners: Set<number>
  /** Bumped by every teardown: a stale async step checks it and stops. */
  gen: number
  /** Storage writes not yet handed to the store, by key (`null` deletes). */
  writes: Map<string, { value: string | null; waiters: ((error: string | null) => void)[] }>
  writeTimer: ReturnType<typeof setTimeout> | null
  /** Every batch handed to the store, in order: the next one starts after
   *  it, and a read that must see them waits for it. Never rejects. */
  flushed: Promise<void>
}

export interface PluginHost extends ForwardRouter {
  /** Dispatches `frame` on `conn` as this window's own call: queued while
   *  the document is held, and a forward it causes is served and completed
   *  here. Resolves with the reply frame, or undefined for a notification. */
  dispatch(conn: number, frame: string, stamp?: number): Promise<string | undefined>
  /** Re-reads the store: registers new plugins, drops removed ones,
   *  restarts changed ones. */
  sync(): Promise<void>
  states(): Record<string, RealmState>
  dispose(): void
}

type Manifest = PluginManifest & {
  main?: unknown
  activation?: unknown
  contributes?: { commands?: unknown }
}

export function createPluginHost(
  env: HostEnv,
  timings: HostTimings = DEFAULT_TIMINGS,
  sched: HostScheduler = defaultScheduler(),
): PluginHost {
  const plugins = new Map<string, Tracked>()
  const byConn = new Map<number, Tracked>()
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
    const p = byConn.get(conn)
    if (p?.realm?.ready) post(p, { t: 'bus', frame })
  })

  const unlistenStorage = env.store.onStorageChanged((id, keys) => {
    const p = plugins.get(id)
    if (p?.realm?.ready) post(p, { t: 'storage_changed', scope: 'plugin', keys })
  })

  const post = (p: Tracked, message: HostToRealm, transfer: Transferable[] = []): void => {
    try {
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
      const mutated = result.dryRun !== true && refreshesAfter(scene, method, result.frame)
      afterDispatch(env.bridge, scene, result.frame, mutated)
      pumpEvents(scene)
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
    if (p === undefined || p.state === 'crashed' || disposed) return Promise.resolve({ failed: 'unavailable' })
    if (signal.aborted) return Promise.resolve({ failed: 'cancelled' })
    const timeoutMs = forwardTimeoutMs(fwd.timeout_ms)
    const gen = p.gen
    return new Promise<ForwardReplyJson>((resolve) => {
      let settled = false
      let realmId: number | null = null
      let timer: ReturnType<typeof setTimeout> | null = null
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
          if (realm === null || p.gen !== gen) {
            settle({ failed: p.state === 'crashed' ? 'crashed' : 'unavailable' })
            return
          }
          realmId = ++realm.nextForward
          realm.forwards.set(realmId, { fwd, settle })
          // The clock runs from here: starting the realm has limits of its
          // own, and a plugin that is only slow to start is not stopped.
          timer = setTimeout(() => {
            giveUp()
            settle({ failed: 'timeout' })
            crash(p, gen, 'timeout', `the command ${cleanText(fwd.local, 64)} did not answer within ${Math.round(timeoutMs / 1000)} s`)
          }, timeoutMs)
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
    if (p.state === 'activated' && p.realm !== null) return Promise.resolve(p.realm)
    if (p.state === 'crashed') return Promise.resolve(null)
    if (stamp !== undefined && !p.joiners.has(stamp)) {
      p.joiners.add(stamp)
      nudge()
    }
    if (p.activation !== null) return p.activation
    p.state = 'activating'
    const gen = p.gen
    const started = start(p, gen, reason).catch((e: unknown) => {
      if (e instanceof NotStarted) {
        if (p.gen === gen && p.state === 'activating') p.state = 'enabled'
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
  async function startCall(p: Tracked, gen: number, scene: Scene, conn: number, deadline: number, id: string, method: string, params: unknown): Promise<void> {
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
        const result = dispatchNow(scene, conn, frame, stamp)
        const reply = result.kind === 'reply' ? result.frame : undefined
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
    if (main === null) throw new Error('the manifest names no main script')
    const code = await env.store.readFile(p.id, main)
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
    const names = [...p.installed.grant, ...p.installed.optional_granted]
    const grant = env.grantJson(p.id, document, JSON.stringify(names))
    const conn = scene.api_plugin_connection_open(grant)
    try {
      await startCall(p, gen, scene, conn, deadline, 'host-hello', 'hew.meta.hello', { protocol: 1, client: { name: p.id, version: p.version } })
      await startCall(p, gen, scene, conn, deadline, 'host-attach', 'hew.doc.attach', {})
    } catch (e) {
      scene.api_connection_close(conn)
      if (e instanceof Stale) return null
      throw e
    }
    if (p.gen !== gen) {
      scene.api_connection_close(conn)
      return null
    }

    let channel: RealmChannel
    try {
      channel = await env.openRealm(code, () => crash(p, gen, 'navigated', 'the realm frame loaded another page'))
    } catch (e) {
      scene.api_connection_close(conn)
      throw e
    }
    if (p.gen !== gen) {
      channel.remove()
      scene.api_connection_close(conn)
      return null
    }
    const realm: LiveRealm = {
      channel,
      conn,
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
        p.state = 'activated'
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
            if (realm.unanswered >= 2) {
              crash(p, gen, 'unresponsive', 'two pings went unanswered')
              return
            }
            realm.unanswered++
            post(p, { t: 'ping', n: ++realm.pingN })
          }, timings.pingMs),
        )
        resolve(realm)
      }
      realm.onGone = () => resolve(null)
      post(p, {
        t: 'hello',
        plugin: p.id,
        version: p.version,
        realm: 'background',
        document,
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
        return onStorage(p, gen, realm, m)
      case 'files':
        return onFiles(p, gen, realm, m)
      case 'console':
        return onConsole(p, realm, m)
      case 'error':
        return crash(p, gen, 'error', m.message)
      case 'pong':
      case 'unloaded':
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
    const text = m.args.map((a) => cleanText(a, 500)).join(' ')
    log(m.level === 'log' ? 'info' : m.level, `[${p.id}] ${cleanText(text, 2_000)}`)
  }

  function onBus(p: Tracked, gen: number, realm: LiveRealm, frame: string, forward: number | undefined): void {
    let stamp: number | undefined
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
      stamp = stampFor(open.fwd)
    }
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

  /**
   * Hands the writes not yet made to the store, as one batch after every
   * earlier one, so writes to one plugin's storage land in the order the
   * plugin made them. Resolves once this batch and every earlier one is
   * done. Each write's failure goes to the requests that made it.
   */
  function flushWrites(p: Tracked): Promise<void> {
    if (p.writeTimer !== null) {
      clearTimeout(p.writeTimer)
      p.writeTimer = null
    }
    if (p.writes.size === 0) return p.flushed
    const batch = [...p.writes]
    p.writes.clear()
    p.flushed = p.flushed
      .then(async () => {
        for (const [key, w] of batch) {
          let error: string | null = null
          try {
            if (w.value === null) await env.storage.delete(p.id, key)
            else await env.storage.set(p.id, key, w.value)
          } catch (e) {
            error = refusalName(e)
          }
          for (const done of w.waiters) done(error)
        }
      })
      .catch((e: unknown) => log('error', `writing the storage of ${p.id} failed in the host: ${String(e)}`))
    return p.flushed
  }

  function onStorage(p: Tracked, gen: number, realm: LiveRealm, m: Extract<RealmToHost, { t: 'storage' }>): void {
    if (realm.storageIds.has(m.id)) return violation(p, gen, 'the plugin reused a storage request id')
    if (realm.storageIds.size >= 64) return violation(p, gen, 'the plugin has too many storage requests open')
    realm.storageIds.add(m.id)
    const reply = (msg: HostToRealm): void => {
      realm.storageIds.delete(m.id)
      if (p.gen === gen) post(p, msg)
    }
    const fail = (e: unknown): void => reply({ t: 'storage_reply', id: m.id, ok: false, error: refusalName(e) })
    switch (m.op) {
      case 'get': {
        const key = m.key as string
        const pending = p.writes.get(key)
        if (pending !== undefined) {
          reply({ t: 'storage_reply', id: m.id, ok: true, value: pending.value })
          return
        }
        // After the batches already handed to the store, so the read sees
        // every earlier write (or, when one failed, what is really stored).
        p.flushed
          .then(() => env.storage.get(p.id, key))
          .then((value) => reply({ t: 'storage_reply', id: m.id, ok: true, value }), fail)
        return
      }
      case 'set':
      case 'delete': {
        const key = m.key as string
        const entry = p.writes.get(key) ?? { value: null, waiters: [] }
        entry.value = m.op === 'set' ? (m.value as string) : null
        entry.waiters.push((error) =>
          reply(error === null ? { t: 'storage_reply', id: m.id, ok: true } : { t: 'storage_reply', id: m.id, ok: false, error }),
        )
        p.writes.set(key, entry)
        p.writeTimer ??= setTimeout(() => void flushWrites(p), timings.storageDebounceMs)
        return
      }
      case 'list':
        flushWrites(p)
          .then(() => env.storage.list(p.id))
          .then((keys) => reply({ t: 'storage_reply', id: m.id, ok: true, keys }), fail)
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
      if (ending.kind === 'unload') {
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
      try {
        scene?.api_connection_close(realm.conn)
      } catch {
        /* the scene is gone */
      }
    }
    // Writes the plugin made before it stopped still land.
    if (p.writes.size > 0) void flushWrites(p)
    // A start waiting for the document sees the teardown.
    nudge()
    if (scene !== null && realm !== null) {
      try {
        notePluginState(scene, p.id, ending.kind === 'crash' ? 'crashed' : 'unloaded', ending.kind === 'crash' ? ending.detail : ending.reason)
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
    p.state = 'crashed'
    try {
      env.scene()?.unregister_plugin_commands(p.id)
    } catch {
      /* the scene is gone */
    }
    log('error', `Plugin ${p.id} ${p.version} stopped (${reason}): ${cleanText(detail, 300)}`)
    try {
      env.toast(`${p.name} stopped working. To start it again, turn it off and on in Settings > Plugins.`)
    } catch {
      /* a toast that fails must not stop the host */
    }
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

  let syncing: Promise<void> = Promise.resolve()
  function sync(): Promise<void> {
    syncing = syncing.then(syncNow, syncNow)
    return syncing
  }

  async function syncNow(): Promise<void> {
    if (disposed) return
    let listed: Listed[]
    try {
      listed = await env.store.list()
    } catch (e) {
      log('warn', `The plugin list could not be read: ${String(e)}`)
      return
    }
    if (disposed) return
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
      let p = plugins.get(id)
      if (p !== undefined && p.key === key) {
        p.installed = installed
        p.manifest = manifest
        continue
      }
      if (p !== undefined) teardown(p, { kind: 'unload', reason: 'reload' })
      p = {
        id,
        version: installed.version,
        name: displayName(manifest),
        installed,
        manifest,
        key,
        state: 'enabled',
        realm: null,
        activation: null,
        joiners: new Set(),
        gen: (p?.gen ?? 0) + 1,
        writes: p?.writes ?? new Map(),
        writeTimer: null,
        flushed: p?.flushed ?? Promise.resolve(),
      }
      plugins.set(id, p)
      if (!register(p)) {
        p.state = 'crashed'
        continue
      }
      const activation = (manifest as Manifest).activation
      if (installed.startup_allowed && Array.isArray(activation) && activation.includes('on_startup')) {
        startUp(p)
      }
    }
  }

  function startUp(p: Tracked): void {
    // The start itself waits while a command holds the document.
    if (plugins.get(p.id) === p && p.state === 'enabled') void activate(p, 'startup')
  }

  const unlistenStore = env.store.onChanged(() => void sync())

  return {
    serve,
    dispatch,
    sync,
    states() {
      const out: Record<string, RealmState> = {}
      for (const [id, p] of plugins) out[id] = p.state
      return out
    },
    dispose() {
      if (disposed) return
      disposed = true
      unlistenStore()
      unlistenStorage()
      unregisterSink()
      unsubscribeBusy()
      unsubscribeCompounds()
      for (const p of plugins.values()) {
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
