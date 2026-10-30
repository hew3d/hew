/**
 * Whether a plugin command holds this window's document
 * (docs/agents/PLUGINS.md "Compounds"): the one piece of state the live
 * bridge, the test harness, and the busy notice share. While it is set the
 * kernel refuses every UI edit (`DocumentBusy`), the input layer ignores
 * gestures, and `BusyNotice` offers Cancel after a quarter second.
 *
 * A tiny external store rather than React state: the bridge and the
 * harness are not components, and `useBusy` reads it through
 * `useSyncExternalStore`.
 */

import { useSyncExternalStore } from 'react'

export interface BusyState {
  /** The plugin's id (its display name arrives with installed plugins). */
  plugin: string
  /** The command's own label, when it declares one. */
  label: string | null
  /** `performance.now()` when the command started. */
  startedAt: number
  /** Aborts the command: the document goes back to how it was. */
  cancel(): void
}

let current: BusyState | null = null
const listeners = new Set<() => void>()

/** Sets (or, with `null`, clears) the busy state and tells every
 * subscriber. */
export function setBusy(next: BusyState | null): void {
  if (next === current) return
  current = next
  for (const listener of [...listeners]) listener()
}

/** The current busy state, or `null` when the document is free. */
export function getBusy(): BusyState | null {
  return current
}

/** Calls `listener` after every change; returns the unsubscribe. */
export function subscribeBusy(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

/** The busy state, re-rendering on every change. */
export function useBusy(): BusyState | null {
  return useSyncExternalStore(subscribeBusy, getBusy, getBusy)
}
