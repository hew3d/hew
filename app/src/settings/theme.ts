/**
 * Theme setting — module-level singleton.
 *
 * A three-way Auto/Light/Dark preference. This module owns ONLY the persisted
 * setting + its cross-window sync + the pure Auto->resolved-theme logic; it
 * never touches the DOM (see `theme/applyTheme.ts`, which subscribes here and
 * applies the resolved theme as a `data-theme` attribute).
 *
 * Persistence + cross-window sync mirrors settings/debugMode.ts exactly:
 *   - Persisted to localStorage under `hew.settings.theme`.
 *   - Under Tauri, separate webview windows (main + Settings) do NOT share a
 *     `storage` event, so changes are ALSO broadcast via the same
 *     'settings-changed' Tauri global event debugMode.ts/units.ts use
 *     (different payload key — `theme` — so all three listeners coexist on
 *     one event channel without colliding).
 *   - The browser 'storage' event covers same-origin web tabs.
 */

import { isTauri } from '../io/fileHost'

export type ThemeSetting = 'auto' | 'light' | 'dark'
export type ResolvedTheme = 'light' | 'dark'

const STORAGE_KEY = 'hew.settings.theme'
const DEFAULT_THEME_SETTING: ThemeSetting = 'auto'

function isThemeSetting(v: unknown): v is ThemeSetting {
  return v === 'auto' || v === 'light' || v === 'dark'
}

function loadInitial(): ThemeSetting {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (isThemeSetting(raw)) return raw
  } catch {
    /* ignore — privacy mode / unavailable storage */
  }
  return DEFAULT_THEME_SETTING
}

let currentThemeSetting: ThemeSetting = loadInitial()
const subscribers = new Set<(setting: ThemeSetting) => void>()

function notify(): void {
  for (const cb of subscribers) cb(currentThemeSetting)
}

/** Read the current Theme setting ('auto' | 'light' | 'dark'). */
export function getThemeSetting(): ThemeSetting {
  return currentThemeSetting
}

/**
 * Set the Theme setting. Persists to localStorage, notifies local
 * subscribers, and broadcasts to other windows (Tauri global event; the
 * 'storage' event covers same-origin web tabs automatically).
 */
export function setThemeSetting(next: ThemeSetting): void {
  currentThemeSetting = next
  try {
    localStorage.setItem(STORAGE_KEY, next)
  } catch {
    /* ignore quota / privacy-mode errors */
  }
  notify()
  broadcastTauri(next)
}

/** Subscribe to Theme setting changes (local + cross-window). Returns an unsubscribe fn. */
export function subscribe(cb: (setting: ThemeSetting) => void): () => void {
  subscribers.add(cb)
  return () => subscribers.delete(cb)
}

/**
 * Resolve a Theme setting to a concrete 'light' | 'dark' value. 'auto'
 * follows the OS-level `prefers-color-scheme` media query; explicit values
 * pass through unchanged.
 */
export function resolveTheme(setting: ThemeSetting): ResolvedTheme {
  if (setting !== 'auto') return setting
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return 'dark'
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
}

/** Convenience: the currently EFFECTIVE theme (resolves 'auto'). */
export function getResolvedTheme(): ResolvedTheme {
  return resolveTheme(currentThemeSetting)
}

// ---------------------------------------------------------------------------
// Cross-window sync
// ---------------------------------------------------------------------------

let tauriEmit: ((event: string, payload?: unknown) => Promise<void>) | null = null

function broadcastTauri(setting: ThemeSetting): void {
  if (!isTauri) return
  if (tauriEmit !== null) {
    tauriEmit('settings-changed', { theme: setting }).catch(() => { /* ignore */ })
    return
  }
  import('@tauri-apps/api/event').then(({ emit }) => {
    tauriEmit = emit
    return emit('settings-changed', { theme: setting })
  }).catch(() => { /* ignore */ })
}

function applyExternal(next: unknown): void {
  if (!isThemeSetting(next) || next === currentThemeSetting) return
  currentThemeSetting = next
  notify()
}

// Refresh the singleton + notify subscribers when the OTHER window changes
// the setting. Two channels:
//   - Tauri global event 'settings-changed' (separate webview windows).
//   - Browser 'storage' event (same-origin web tabs).
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (ev) => {
    if (ev.key !== STORAGE_KEY) return
    applyExternal(ev.newValue)
  })

  if (isTauri) {
    import('@tauri-apps/api/event').then(({ listen }) => {
      return listen<{ theme?: unknown }>('settings-changed', (event) => {
        applyExternal(event.payload?.theme)
      })
    }).catch(() => { /* ignore */ })
  }
}
