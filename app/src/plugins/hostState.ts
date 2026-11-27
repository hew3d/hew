/**
 * What this page's plugin host says about its plugins, for the Plugins
 * pane to read: each plugin's last reported state (with why, and when),
 * and its last 500 console lines. On the web this is the only record (the
 * pane reads the host in the same page); on the desktop the shell keeps
 * the lasting one (`plugin_report_state`, `plugin_console_append`) and
 * this is the same window's view of it.
 *
 * Preview-only: imported only by code under `app/src/plugins/`, which a
 * production build leaves out.
 */

import type { ConsoleLine, ReportedState } from './hostCore'

export interface HostStateEntry {
  state: ReportedState
  reason: string | null
  /** Milliseconds since the Unix epoch. */
  at: number
}

/** Console lines kept per plugin. */
export const MAX_CONSOLE_LINES = 500
/** State changes kept per plugin. */
export const MAX_ACTIVITY = 500

let states: Record<string, HostStateEntry> = {}
const stateListeners = new Set<() => void>()
const lines = new Map<string, ConsoleLine[]>()
const changes = new Map<string, HostStateEntry[]>()
const consoleListeners = new Map<string, Set<() => void>>()

const notify = (fns: Iterable<() => void>): void => {
  for (const fn of [...fns]) {
    try {
      fn()
    } catch {
      /* a listener that throws must not stop the host */
    }
  }
}

/** Every plugin's last reported state, by id. A new object after each
 *  change, so a React store can compare it. */
export function hostStates(): Readonly<Record<string, HostStateEntry>> {
  return states
}

/** Calls `fn` after each state change; returns the function that stops. */
export function subscribeHostStates(fn: () => void): () => void {
  stateListeners.add(fn)
  return () => {
    stateListeners.delete(fn)
  }
}

/** The host reports `state` for plugin `id`. */
export function noteHostState(id: string, state: ReportedState, reason: string | null, at: number = Date.now()): void {
  const entry = { state, reason, at }
  states = { ...states, [id]: entry }
  const kept = [...(changes.get(id) ?? []), entry]
  changes.set(id, kept.length > MAX_ACTIVITY ? kept.slice(kept.length - MAX_ACTIVITY) : kept)
  notify(stateListeners)
}

/** The state changes the host saw for plugin `id`, oldest first, at most
 *  `MAX_ACTIVITY`: kept whether or not anything was reading them. */
export function activity(id: string): readonly HostStateEntry[] {
  return changes.get(id) ?? []
}

/** Plugin `id`'s console lines, oldest first, at most `MAX_CONSOLE_LINES`. */
export function consoleLines(id: string): readonly ConsoleLine[] {
  return lines.get(id) ?? []
}

/** Calls `fn` after plugin `id` logs; returns the function that stops. */
export function subscribeConsole(id: string, fn: () => void): () => void {
  let set = consoleListeners.get(id)
  if (set === undefined) {
    set = new Set()
    consoleListeners.set(id, set)
  }
  set.add(fn)
  return () => {
    set.delete(fn)
  }
}

/** The host hands on plugin `id`'s console lines. */
export function noteConsole(id: string, more: readonly ConsoleLine[]): void {
  if (more.length === 0) return
  const next = [...(lines.get(id) ?? []), ...more]
  lines.set(id, next.length > MAX_CONSOLE_LINES ? next.slice(next.length - MAX_CONSOLE_LINES) : next)
  notify(consoleListeners.get(id) ?? [])
}

/** Forgets everything (tests, and a host that stops). */
export function resetHostState(): void {
  states = {}
  lines.clear()
  changes.clear()
  notify(stateListeners)
}
