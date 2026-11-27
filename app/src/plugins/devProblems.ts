/**
 * The last problem the folder watcher reported for each folder plugin.
 * The watcher says a problem once, as an event, so a pane that opens later
 * would never hear it. This keeps what was said, per installer, for the life
 * of the window: it listens from the first time the pane asks, a later notice
 * without a problem clears the entry, and a list that no longer holds the
 * plugin drops it.
 */

import type { Listed, PluginInstaller } from './types'

interface Store {
  problems: Map<string, string>
  listeners: Set<() => void>
}

const stores = new WeakMap<PluginInstaller, Store>()

function storeOf(inst: PluginInstaller): Store {
  let s = stores.get(inst)
  if (s === undefined) {
    const made: Store = { problems: new Map(), listeners: new Set() }
    s = made
    stores.set(inst, made)
    // Held for the life of the window, so a notice that arrives while the
    // pane is closed is still there when it opens.
    inst.onDevChanged?.((change) => {
      if (change.problem === null) {
        if (!made.problems.delete(change.id)) return
      } else if (made.problems.get(change.id) === change.problem) {
        return
      } else {
        made.problems.set(change.id, change.problem)
      }
      made.listeners.forEach((l) => l())
    })
  }
  return s
}

/** The problems known now, by plugin id. */
export function devProblems(inst: PluginInstaller): Record<string, string> {
  return Object.fromEntries(storeOf(inst).problems)
}

/** Calls `fn` when the known problems change. Returns the unsubscribe. */
export function watchDevProblems(inst: PluginInstaller, fn: () => void): () => void {
  const s = storeOf(inst)
  s.listeners.add(fn)
  return () => s.listeners.delete(fn)
}

/** Takes what the list says the watcher has now (`dev_problem` on a folder
 *  plugin's row): a problem is kept, `null` clears it, a row without the
 *  field says nothing. So a problem reported before the pane first opened
 *  in this window is still shown. */
export function seedDevProblems(inst: PluginInstaller, listed: readonly Listed[]): void {
  const s = storeOf(inst)
  let changed = false
  for (const l of listed) {
    if (l.kind !== 'installed' || l.dev_problem === undefined) continue
    const id = l.installed.id
    if (l.dev_problem === null) {
      if (s.problems.delete(id)) changed = true
    } else if (typeof l.dev_problem === 'string' && s.problems.get(id) !== l.dev_problem) {
      s.problems.set(id, l.dev_problem)
      changed = true
    }
  }
  if (changed) s.listeners.forEach((l) => l())
}

/** Forgets the problem of every plugin not in `present`. */
export function pruneDevProblems(inst: PluginInstaller, present: Iterable<string>): void {
  const s = storeOf(inst)
  const keep = new Set(present)
  let changed = false
  for (const id of [...s.problems.keys()]) {
    if (!keep.has(id)) {
      s.problems.delete(id)
      changed = true
    }
  }
  if (changed) s.listeners.forEach((l) => l())
}
