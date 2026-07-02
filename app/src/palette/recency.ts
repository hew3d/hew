/**
 * palette/recency — tracks the last-run palette action ids (
 * `04_command_palette.md`'s "recency of the user's own usage" ranking signal
 * + the footer's "Recent:" breadcrumb).
 *
 * Module-level singleton, same persistence shape as `settings/theme.ts` /
 * `settings/debugMode.ts` (localStorage + cross-window Tauri broadcast) —
 * but simpler: no cross-window sync is needed here (recency is a soft
 * ranking hint, not a setting two windows must agree on), so this module
 * only persists + notifies local subscribers.
 */

const STORAGE_KEY = 'hew.palette.recent'
const MAX_RECENT = 10

function loadInitial(): string[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw === null) return []
    const parsed: unknown = JSON.parse(raw)
    return Array.isArray(parsed) ? parsed.filter((v): v is string => typeof v === 'string') : []
  } catch {
    return []
  }
}

let recent: string[] = loadInitial()
const subscribers = new Set<(ids: string[]) => void>()

function notify(): void {
  for (const cb of subscribers) cb(recent)
}

/** Most-recently-run action ids first, capped at MAX_RECENT. */
export function getRecent(): string[] {
  return recent
}

/** Record that `id` was just run — moves it to the front, persists, notifies. */
export function recordRun(id: string): void {
  recent = [id, ...recent.filter((r) => r !== id)].slice(0, MAX_RECENT)
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(recent))
  } catch {
    /* ignore quota / privacy-mode errors */
  }
  notify()
}

export function subscribe(cb: (ids: string[]) => void): () => void {
  subscribers.add(cb)
  return () => subscribers.delete(cb)
}

/** Clear all recency history (mirrors the existing "Clear Recent" affordance
 * for recent files — `MenuBar.tsx`'s File ▸ Open Recent ▸ Clear Recent). */
export function clearRecent(): void {
  recent = []
  try {
    localStorage.removeItem(STORAGE_KEY)
  } catch {
    /* ignore */
  }
  notify()
}
