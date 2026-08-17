/**
 * Scene transitions setting — module-level singleton (docs/design/scenes.md §5).
 *
 * One boolean: whether activating a Scene animates the camera (600 ms
 * ease-in-out, `SCENE_TRANSITION_MS`) or snaps. Default on. Surfaced as a
 * View ▸ Scenes ▸ Scene Transitions checkmark rather than a Settings-pane
 * row: Settings has no pane it belongs in, and the duration is fixed —
 * SketchUp's per-model 2 s is a common complaint, and nobody tunes it.
 * `prefers-reduced-motion` forces instant regardless (Viewport's tween).
 *
 * Persistence + cross-window sync mirrors settings/units.ts exactly:
 *   - Persisted to localStorage under `hew.settings.sceneTransitions`.
 *   - Under Tauri, separate webview windows (main + Settings) do NOT share a
 *     `storage` event, so changes are ALSO broadcast via the same
 *     'settings-changed' Tauri global event units.ts uses (different payload
 *     key — `sceneTransitions` vs `lengthUnit` — so both listeners coexist on one
 *     event channel without colliding).
 *   - The browser 'storage' event covers same-origin web tabs.
 */

import { isTauri } from '../io/fileHost'

/** Camera tween duration when transitions are on (SPEC.md §3). */
export const SCENE_TRANSITION_MS = 600

const STORAGE_KEY = 'hew.settings.sceneTransitions'
const DEFAULT_SCENE_TRANSITIONS = true

function isBoolString(v: unknown): v is string {
  return v === 'true' || v === 'false'
}

function loadInitial(): boolean {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (isBoolString(raw)) return raw === 'true'
  } catch {
    /* ignore — privacy mode / unavailable storage */
  }
  return DEFAULT_SCENE_TRANSITIONS
}

let currentSceneTransitions: boolean = loadInitial()
const subscribers = new Set<(on: boolean) => void>()

function notify(): void {
  for (const cb of subscribers) cb(currentSceneTransitions)
}

/** Read the current Scene transitions setting. */
export function getSceneTransitions(): boolean {
  return currentSceneTransitions
}

/**
 * Set Scene transitions. Persists to localStorage, notifies local subscribers, and
 * broadcasts to other windows (Tauri global event; the 'storage' event
 * covers same-origin web tabs automatically).
 */
export function setSceneTransitions(on: boolean): void {
  currentSceneTransitions = on
  try {
    localStorage.setItem(STORAGE_KEY, String(on))
  } catch {
    /* ignore quota / privacy-mode errors */
  }
  notify()
  broadcastTauri(on)
}

/** Subscribe to Scene transitions changes (local + cross-window). Returns an unsubscribe fn. */
export function subscribe(cb: (on: boolean) => void): () => void {
  subscribers.add(cb)
  return () => subscribers.delete(cb)
}

// ---------------------------------------------------------------------------
// Cross-window sync
// ---------------------------------------------------------------------------

let tauriEmit: ((event: string, payload?: unknown) => Promise<void>) | null = null

function broadcastTauri(on: boolean): void {
  if (!isTauri) return
  if (tauriEmit !== null) {
    tauriEmit('settings-changed', { sceneTransitions: on }).catch(() => { /* ignore */ })
    return
  }
  import('@tauri-apps/api/event').then(({ emit }) => {
    tauriEmit = emit
    return emit('settings-changed', { sceneTransitions: on })
  }).catch(() => { /* ignore */ })
}

function applyExternal(next: unknown): void {
  if (typeof next !== 'boolean' || next === currentSceneTransitions) return
  currentSceneTransitions = next
  notify()
}

// Refresh the singleton + notify subscribers when the OTHER window changes
// the setting. Two channels:
//   - Tauri global event 'settings-changed' (separate webview windows).
//   - Browser 'storage' event (same-origin web tabs).
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (ev) => {
    if (ev.key !== STORAGE_KEY) return
    if (isBoolString(ev.newValue)) {
      applyExternal(ev.newValue === 'true')
    }
  })

  if (isTauri) {
    import('@tauri-apps/api/event').then(({ listen }) => {
      return listen<{ sceneTransitions?: unknown }>('settings-changed', (event) => {
        applyExternal(event.payload?.sceneTransitions)
      })
    }).catch(() => { /* ignore */ })
  }
}
