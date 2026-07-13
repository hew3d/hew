/**
 * Tray layout — module-level singleton.
 *
 * Persists the docked right tray's per-section expanded/collapsed state
 * (Object Info / Outliner / Materials / Tags) across launches, so the
 * app stops forgetting the user's layout. This module owns ONLY the persisted
 * flags + their cross-window sync; App.tsx keeps owning the React state (its
 * four showX/setShowX pairs initialize from here and write back on change),
 * so every existing keyboard shortcut and Window-menu checkmark works
 * unchanged.
 *
 * Persistence + cross-window sync mirrors settings/theme.ts exactly:
 *   - Persisted to localStorage under `hew.settings.trayLayout` (one JSON
 *     object — the four flags are one logical "layout", not four settings).
 *   - Under Tauri, separate webview windows (main + Settings) do NOT share a
 *     `storage` event, so changes are ALSO broadcast via the same
 *     'settings-changed' Tauri global event theme.ts/debugMode.ts/units.ts
 *     use (different payload key — `trayLayout` — so all listeners coexist
 *     on one event channel without colliding).
 *   - The browser 'storage' event covers same-origin web tabs.
 */

import { isTauri } from '../io/fileHost'

/** Expanded (true) / collapsed (false) per tray section. Key names follow
 * App.tsx's showX state names, not the section titles (modelInfo = the
 * Outliner section, objectInfo = Object Info — a rename kept for continuity
 * with the pre- floating-panel era). */
export interface TrayLayout {
  modelInfo: boolean
  objectInfo: boolean
  materials: boolean
  tags: boolean
}

const STORAGE_KEY = 'hew.settings.trayLayout'

const KEYS = ['modelInfo', 'objectInfo', 'materials', 'tags'] as const

/**  defaults: Object Info + Outliner open, Materials + Tags collapsed. */
export const DEFAULT_TRAY_LAYOUT: TrayLayout = {
  modelInfo: true,
  objectInfo: true,
  materials: false,
  tags: false,
}

/** Parse a persisted/broadcast value. Unknown shapes return null; individual
 * missing/mistyped flags fall back to their default (forward-compatible if a
 * fifth section is ever added). */
function parseTrayLayout(v: unknown): TrayLayout | null {
  let obj: unknown = v
  if (typeof v === 'string') {
    try {
      obj = JSON.parse(v)
    } catch {
      return null
    }
  }
  if (typeof obj !== 'object' || obj === null) return null
  const out = { ...DEFAULT_TRAY_LAYOUT }
  for (const k of KEYS) {
    const flag = (obj as Record<string, unknown>)[k]
    if (typeof flag === 'boolean') out[k] = flag
  }
  return out
}

function loadInitial(): TrayLayout {
  try {
    const parsed = parseTrayLayout(localStorage.getItem(STORAGE_KEY))
    if (parsed !== null) return parsed
  } catch {
    /* ignore — privacy mode / unavailable storage */
  }
  return { ...DEFAULT_TRAY_LAYOUT }
}

let currentTrayLayout: TrayLayout = loadInitial()
const subscribers = new Set<(layout: TrayLayout) => void>()

function notify(): void {
  for (const cb of subscribers) cb(currentTrayLayout)
}

/** Read the current tray layout (expanded flags per section). */
export function getTrayLayout(): TrayLayout {
  return currentTrayLayout
}

/**
 * Set the tray layout. Persists to localStorage, notifies local subscribers,
 * and broadcasts to other windows (Tauri global event; the 'storage' event
 * covers same-origin web tabs automatically).
 */
export function setTrayLayout(next: TrayLayout): void {
  currentTrayLayout = { ...next }
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(currentTrayLayout))
  } catch {
    /* ignore quota / privacy-mode errors */
  }
  notify()
  broadcastTauri(currentTrayLayout)
}

/** Subscribe to tray-layout changes (local + cross-window). Returns an unsubscribe fn. */
export function subscribe(cb: (layout: TrayLayout) => void): () => void {
  subscribers.add(cb)
  return () => subscribers.delete(cb)
}

// ---------------------------------------------------------------------------
// Cross-window sync
// ---------------------------------------------------------------------------

let tauriEmit: ((event: string, payload?: unknown) => Promise<void>) | null = null

function broadcastTauri(layout: TrayLayout): void {
  if (!isTauri) return
  if (tauriEmit !== null) {
    tauriEmit('settings-changed', { trayLayout: layout }).catch(() => { /* ignore */ })
    return
  }
  import('@tauri-apps/api/event').then(({ emit }) => {
    tauriEmit = emit
    return emit('settings-changed', { trayLayout: layout })
  }).catch(() => { /* ignore */ })
}

function sameLayout(a: TrayLayout, b: TrayLayout): boolean {
  return KEYS.every((k) => a[k] === b[k])
}

function applyExternal(next: unknown): void {
  const parsed = parseTrayLayout(next)
  if (parsed === null || sameLayout(parsed, currentTrayLayout)) return
  currentTrayLayout = parsed
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
      return listen<{ trayLayout?: unknown }>('settings-changed', (event) => {
        applyExternal(event.payload?.trayLayout)
      })
    }).catch(() => { /* ignore */ })
  }
}
