/* oxlint-disable lingui/no-unlocalized-strings -- plugin pane copy and protocol words; English only until the feature graduates. */
/**
 * What Settings > Plugins knows about plugins that are running: whether each
 * one runs, why one stopped, the lines it printed, and the changes of state
 * it went through.
 *
 * Desktop: the shell keeps all of it (`plugin_list` rows carry `running_in`,
 * `stopped_in`, `blocked_in` and `last_stop`; `plugin_console_read` and
 * `plugin_activity_read` answer for one plugin; `plugins-state` and
 * `plugins-console` are hints to read again). Web: the host in this page
 * keeps it (`hostState.ts`), read through the small `HostStateSource`
 * interface below so a test can stand in for it.
 *
 * Every word a plugin wrote (console text, a stop reason) is data. Console
 * text is cleaned and cut here and shown only as text; a reason is matched
 * against a fixed table and never shown itself.
 */

import { cleanText } from './pluginText'
import type { Listed } from './types'

/** The most console lines kept for the view, and the most characters of one. */
export const MAX_CONSOLE_LINES = 500
export const MAX_LINE_CHARS = 1000
/** The most changes of state kept for the view. */
export const MAX_ACTIVITY = 500

export type ConsoleLevel = 'log' | 'info' | 'warn' | 'error'

export interface ConsoleLine {
  /** Milliseconds since the epoch. */
  at: number
  level: ConsoleLevel
  /** Cleaned and cut; text, never markup. */
  text: string
}

export interface ActivityEntry {
  at: number
  /** One of the host's state words, or a word this build does not know. */
  state: string
  /** A reason word as the host wrote it, or null. Never shown as is. */
  reason: string | null
}

/** What a plugin's host reports about it. */
export interface HostEntry {
  state: string
  reason: string | null
  at: number | null
}

/** The part of the web host's state module the pane reads. */
export interface HostStateSource {
  hostStates(): unknown
  subscribeHostStates(fn: () => void): () => void
  consoleLines(id: string): unknown
  subscribeConsole(id: string, fn: () => void): () => void
  /** The host's own changes of state for one plugin, when it keeps them. */
  activity?(id: string): unknown
}

export interface PaneData {
  readonly tier: 'desktop' | 'web'
  /** Web: each plugin's state as the host in this page says. Desktop: null
   *  (the list rows carry it). */
  hostStates(): Record<string, HostEntry> | null
  /** Calls `fn` when some plugin's state may have changed. */
  onState(fn: () => void): () => void
  /** Newest first, at most `MAX_CONSOLE_LINES`. */
  console(id: string): Promise<ConsoleLine[]>
  /** Calls `fn` when `id`'s console may have new lines. */
  onConsole(id: string, fn: () => void): () => void
  /** Newest first, at most `MAX_ACTIVITY`. */
  activity(id: string): Promise<ActivityEntry[]>
  /** Calls `fn` when `id`'s activity may have a new entry. */
  onActivity(id: string, fn: () => void): () => void
}

const LEVELS: ReadonlySet<string> = new Set(['log', 'info', 'warn', 'error'])
/** A reason or state word as the host writes them. */
const WORD = /^[a-z][a-z0-9_]{0,63}$/

/** One console line, checked, cleaned and cut; null for anything that is
 *  not one. */
export function consoleLine(raw: unknown): ConsoleLine | null {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return null
  const r = raw as Record<string, unknown>
  if (typeof r.text !== 'string') return null
  const at = typeof r.at === 'number' && Number.isFinite(r.at) ? r.at : 0
  const level = typeof r.level === 'string' && LEVELS.has(r.level) ? (r.level as ConsoleLevel) : 'log'
  return { at, level, text: cleanText(r.text.slice(0, 4 * MAX_LINE_CHARS), MAX_LINE_CHARS) }
}

function word(v: unknown): string | null {
  return typeof v === 'string' && WORD.test(v) ? v : null
}

/** One activity entry, checked; null for anything that is not one. */
export function activityEntry(raw: unknown): ActivityEntry | null {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return null
  const r = raw as Record<string, unknown>
  const state = word(r.state)
  if (state === null) return null
  return { at: typeof r.at === 'number' && Number.isFinite(r.at) ? r.at : 0, state, reason: word(r.reason) }
}

/** `list`, as lines newest first: the shell's order is kept, a host module's
 *  (oldest first, appended as they come) is reversed. */
function bounded<T>(list: unknown, parse: (raw: unknown) => T | null, max: number, newestFirst: boolean): T[] {
  if (!Array.isArray(list)) return []
  const out: T[] = []
  for (const raw of list) {
    const one = parse(raw)
    if (one !== null) out.push(one)
  }
  const ordered = newestFirst ? out : out.reverse()
  return ordered.slice(0, max)
}

function hostEntry(raw: unknown): HostEntry | null {
  if (typeof raw === 'string') {
    const state = word(raw)
    return state === null ? null : { state, reason: null, at: null }
  }
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return null
  const r = raw as Record<string, unknown>
  const state = word(r.state)
  if (state === null) return null
  return { state, reason: word(r.reason), at: typeof r.at === 'number' && Number.isFinite(r.at) ? r.at : null }
}

function hostEntries(raw: unknown): Record<string, HostEntry> {
  const out: Record<string, HostEntry> = {}
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return out
  for (const [id, value] of Object.entries(raw as Record<string, unknown>)) {
    const entry = hostEntry(value)
    if (entry !== null) out[id] = entry
  }
  return out
}

// ---------------------------------------------------------------- what a row says

/** Why a plugin stopped, in plain words. The key is the machine word the
 *  host reports; nothing the plugin wrote is ever a key's value. */
const STOP_REASONS: Readonly<Record<string, string>> = {
  unresponsive: 'it stopped answering',
  timeout: 'it took too long to answer',
  violation: 'it tried to do something it was not allowed to',
  error: 'it ran into an error',
  start: 'it could not start',
}

/** The plain words for a stop reason, or null for a word this build does not
 *  know (including no reason at all). */
export function stopReasonText(reason: string | null | undefined): string | null {
  if (typeof reason !== 'string') return null
  return Object.prototype.hasOwnProperty.call(STOP_REASONS, reason) ? STOP_REASONS[reason] : null
}

export const RESTART_LINE = 'Turn it off and on to start it again.'

export type RunView =
  | { kind: 'running' }
  | { kind: 'not_running_yet' }
  | { kind: 'asleep' }
  | { kind: 'stopped'; reason: string | null; stopped: number; windows: number }
  | { kind: 'held_back' }

/** What the row of an enabled plugin says it is doing, or null when there is
 *  nothing to say (it is off). `host` is the web host's entry for it. */
export function runView(l: Extract<Listed, { kind: 'installed' }>, host: HostEntry | undefined): RunView | null {
  if (!l.installed.enabled) return null
  // The shell's counts are what it wrote, but a row is still read with care:
  // anything that is not a count counts as none.
  const count = (v: number | undefined): number => (typeof v === 'number' && Number.isFinite(v) && v > 0 ? Math.floor(v) : 0)
  if (l.running_in !== undefined || l.stopped_in !== undefined || l.blocked_in !== undefined || l.idle_in !== undefined) {
    const running = count(l.running_in)
    const stopped = count(l.stopped_in)
    const blocked = count(l.blocked_in)
    const idle = count(l.idle_in)
    if (stopped > 0) return { kind: 'stopped', reason: word(l.last_stop?.reason ?? null), stopped, windows: running + stopped + blocked + idle }
    if (running > 0) return { kind: 'running' }
    if (blocked > 0) return { kind: 'held_back' }
    if (idle > 0) return { kind: 'asleep' }
    return { kind: 'not_running_yet' }
  }
  if (host === undefined) return { kind: 'not_running_yet' }
  switch (host.state) {
    case 'activating':
    case 'activated':
      return { kind: 'running' }
    case 'crashed':
      return { kind: 'stopped', reason: host.reason, stopped: 1, windows: 1 }
    case 'blocked':
      return { kind: 'held_back' }
    case 'idle':
    case 'unloaded':
      return { kind: 'asleep' }
    default:
      return { kind: 'not_running_yet' }
  }
}

/** The sentences for a run view. */
export function runLines(view: RunView): string[] {
  switch (view.kind) {
    case 'running':
      return ['Running']
    case 'not_running_yet':
      return ['Not running yet']
    case 'asleep':
      return ['Not running right now. It starts again when it is needed.']
    case 'held_back':
      return ['Hew is not running this plugin right now.']
    case 'stopped': {
      const why = stopReasonText(view.reason)
      const where = view.windows > view.stopped ? `Stopped in ${view.stopped} of ${view.windows} windows` : 'Stopped'
      return [`${where}${why === null ? '' : `: ${why}`}. ${RESTART_LINE}`]
    }
  }
}

/**
 * The entries the Activity list shows. An `enabled` entry is the host's own
 * bookkeeping (it is reported on every start, after an idle sleep and after a
 * reload, never with a reason that says the person did anything), so it is
 * left out.
 */
export function visibleActivity(list: readonly ActivityEntry[]): ActivityEntry[] {
  return list.filter((e) => e.state !== 'enabled')
}

/** One activity entry in plain words. */
export function activityText(entry: ActivityEntry): string {
  const why = stopReasonText(entry.reason)
  switch (entry.state) {
    case 'activating':
      return 'Starting'
    case 'activated':
      return 'Running'
    case 'crashed':
      return why === null ? 'Stopped' : `Stopped: ${why}`
    case 'blocked':
      return 'Held back by Hew'
    case 'idle':
      return 'Went to sleep after sitting idle'
    case 'unloaded':
      // The host says why it took the plugin down; the words say who did.
      switch (entry.reason) {
        case 'disable':
          return 'Turned off'
        case 'reload':
          return 'Reloaded'
        case 'window_closed':
          return 'Stopped because the window was closed'
        default:
          return 'Stopped running'
      }
    default:
      return 'Changed state'
  }
}

// ---------------------------------------------------------------- desktop

type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>
type Listen = (event: string, handler: (event: { payload: unknown }) => void) => Promise<() => void>

export interface DesktopPaneDeps {
  invoke?: Invoke
  listen?: Listen
}

/** The id of a `plugins-state` or `plugins-console` payload, or null. */
function noticeId(payload: unknown): string | null {
  if (typeof payload !== 'object' || payload === null) return null
  const id = (payload as { id?: unknown }).id
  return typeof id === 'string' && id.length > 0 && id.length <= 128 ? id : null
}

function listening(listen: Listen, event: string, fn: (id: string) => void): () => void {
  let stop: (() => void) | null = null
  let closed = false
  listen(event, (e) => {
    const id = noticeId(e.payload)
    if (id !== null) fn(id)
  })
    .then((off) => {
      if (closed) off()
      else stop = off
    })
    .catch(() => {
      /* no event bridge: the view reads again when it is opened */
    })
  return () => {
    closed = true
    stop?.()
  }
}

/** The desktop's data, over the shell's commands and events. */
export function makeDesktopPaneData(deps: DesktopPaneDeps = {}): PaneData {
  const invoke: Invoke =
    deps.invoke ??
    (async (command, args) => {
      const { invoke: call } = await import('@tauri-apps/api/core')
      return call(command, args)
    })
  const listen: Listen =
    deps.listen ??
    (async (event, handler) => {
      const { listen: on } = await import('@tauri-apps/api/event')
      return on(event, handler)
    })
  return {
    tier: 'desktop',
    hostStates: () => null,
    onState: (fn) => listening(listen, 'plugins-state', () => fn()),
    async console(id) {
      return bounded(await invoke<unknown>('plugin_console_read', { id, limit: MAX_CONSOLE_LINES }), consoleLine, MAX_CONSOLE_LINES, true)
    },
    onConsole: (id, fn) => listening(listen, 'plugins-console', (got) => (got === id ? fn() : undefined)),
    async activity(id) {
      return bounded(await invoke<unknown>('plugin_activity_read', { id }), activityEntry, MAX_ACTIVITY, true)
    },
    onActivity: (id, fn) => listening(listen, 'plugins-state', (got) => (got === id ? fn() : undefined)),
  }
}

// ---------------------------------------------------------------- web

/** Loads the web host's state module. It exists only in a preview build,
 *  and only once the host does; null when there is none to read. */
async function loadHostState(): Promise<HostStateSource | null> {
  try {
    return await (await import('./hostStateLink')).hostStateSource()
  } catch {
    return null
  }
}

/** The web's data, over the host in this page. `load` is the test seam. */
export function makeWebPaneData(load: () => Promise<HostStateSource | null> = loadHostState): PaneData {
  let source: HostStateSource | null = null
  let loading: Promise<HostStateSource | null> | null = null
  const ready = (): Promise<HostStateSource | null> => {
    loading ??= load().then((s) => {
      source = s
      return s
    })
    return loading
  }
  void ready()

  // Changes of state seen while someone listens, per plugin, oldest first.
  const seen = new Map<string, ActivityEntry[]>()
  let last: Record<string, HostEntry> = {}
  const stateListeners = new Set<() => void>()
  let stopSource: (() => void) | null = null
  const read = (): Record<string, HostEntry> => {
    try {
      return source === null ? {} : hostEntries(source.hostStates())
    } catch {
      return {}
    }
  }
  const changed = (): void => {
    const now = read()
    for (const [id, entry] of Object.entries(now)) {
      const before = last[id]
      if (before !== undefined && before.state === entry.state && before.reason === entry.reason) continue
      const list = seen.get(id) ?? []
      list.push({ at: entry.at ?? Date.now(), state: entry.state, reason: entry.reason })
      if (list.length > MAX_ACTIVITY) list.splice(0, list.length - MAX_ACTIVITY)
      seen.set(id, list)
    }
    last = now
    ;[...stateListeners].forEach((l) => l())
  }

  const addStateListener = (fn: () => void): (() => void) => {
    stateListeners.add(fn)
    void ready().then((s) => {
      if (s === null || stopSource !== null || stateListeners.size === 0) return
      last = read()
      stopSource = s.subscribeHostStates(changed)
      // What changed while the module loaded.
      fn()
    })
    return () => {
      stateListeners.delete(fn)
      if (stateListeners.size === 0 && stopSource !== null) {
        stopSource()
        stopSource = null
      }
    }
  }

  return {
    tier: 'web',
    hostStates: () => read(),
    onState: addStateListener,
    async console(id) {
      const s = await ready()
      return s === null ? [] : bounded(s.consoleLines(id), consoleLine, MAX_CONSOLE_LINES, false)
    },
    onConsole(id, fn) {
      let stop: (() => void) | null = null
      let closed = false
      void ready().then((s) => {
        if (s === null || closed) return
        stop = s.subscribeConsole(id, fn)
      })
      return () => {
        closed = true
        stop?.()
      }
    },
    async activity(id) {
      const s = await ready()
      if (s?.activity !== undefined) {
        const own = bounded(s.activity(id), activityEntry, MAX_ACTIVITY, false)
        if (own.length > 0) return own
      }
      return [...(seen.get(id) ?? [])].reverse().slice(0, MAX_ACTIVITY)
    },
    onActivity: (_id, fn) => addStateListener(fn),
  }
}

let shared: PaneData | null = null

/** The data for this window: the shell's on the desktop, else the host's. */
export function paneDataFor(tier: 'desktop' | 'web'): PaneData {
  if (shared === null || shared.tier !== tier) shared = tier === 'desktop' ? makeDesktopPaneData() : makeWebPaneData()
  return shared
}
