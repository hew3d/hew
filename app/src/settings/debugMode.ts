/**
 * Debug Mode setting — module-level singleton (docs/DEVELOPMENT.md).
 *
 * A single boolean toggle that, when enabled, turns on three independently
 * expensive diagnostics for the CURRENT session:
 *   - the rolling diagnostic-log FILE sink (`diagnosticLog.setFileLogging`).
 *   - low-level input recording (`recording/inputRecorder`).
 *   - kernel torture mode (`Scene.set_torture_mode` — extra validation + a
 *     re-tessellation self-check after every op; docs/DEVELOPMENT.md).
 *
 * This module owns ONLY the persisted boolean + its cross-window sync; it
 * never touches the kernel/recorder/log-file directly (see App.tsx, which
 * subscribes and applies the global effects, and wasm/loader.ts, which
 * applies it to freshly-created Scenes).
 *
 * Persistence + cross-window sync mirrors settings/units.ts exactly:
 *   - Persisted to localStorage under `hew.settings.debugMode`.
 *   - Under Tauri, separate webview windows (main + Settings) do NOT share a
 *     `storage` event, so changes are ALSO broadcast via the same
 *     'settings-changed' Tauri global event units.ts uses (different payload
 *     key — `debugMode` vs `lengthUnit` — so both listeners coexist on one
 *     event channel without colliding).
 *   - The browser 'storage' event covers same-origin web tabs.
 */

import { isTauri } from '../io/fileHost'

const STORAGE_KEY = 'hew.settings.debugMode'
const DEFAULT_DEBUG_MODE = false

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
  return DEFAULT_DEBUG_MODE
}

let currentDebugMode: boolean = loadInitial()
const subscribers = new Set<(on: boolean) => void>()

function notify(): void {
  for (const cb of subscribers) cb(currentDebugMode)
}

/** Read the current Debug Mode setting. */
export function getDebugMode(): boolean {
  return currentDebugMode
}

/**
 * Set Debug Mode. Persists to localStorage, notifies local subscribers, and
 * broadcasts to other windows (Tauri global event; the 'storage' event
 * covers same-origin web tabs automatically).
 */
export function setDebugMode(on: boolean): void {
  currentDebugMode = on
  try {
    localStorage.setItem(STORAGE_KEY, String(on))
  } catch {
    /* ignore quota / privacy-mode errors */
  }
  notify()
  broadcastTauri(on)
}

/** Subscribe to Debug Mode changes (local + cross-window). Returns an unsubscribe fn. */
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
    tauriEmit('settings-changed', { debugMode: on }).catch(() => { /* ignore */ })
    return
  }
  import('@tauri-apps/api/event').then(({ emit }) => {
    tauriEmit = emit
    return emit('settings-changed', { debugMode: on })
  }).catch(() => { /* ignore */ })
}

function applyExternal(next: unknown): void {
  if (typeof next !== 'boolean' || next === currentDebugMode) return
  currentDebugMode = next
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
      return listen<{ debugMode?: unknown }>('settings-changed', (event) => {
        applyExternal(event.payload?.debugMode)
      })
    }).catch(() => { /* ignore */ })
  }
}
