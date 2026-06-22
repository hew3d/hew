/**
 * Length-unit setting — module-level singleton.
 *
 * The kernel's base length unit is always f64 METERS (see docs/DEVELOPMENT.md). This
 * module owns ONLY a display-layer preference: which unit the UI formats
 * lengths in. It never touches kernel state.
 *
 * Persistence + cross-window sync:
 *   - The current unit is persisted to localStorage under
 *     `hew.settings.lengthUnit`.
 *   - Under Tauri, each window (main + the separate Settings window) is a
 *     distinct webview with its OWN localStorage-backed `window` object, so
 *     the native 'storage' event does NOT fire across them (that event only
 *     fires in OTHER same-origin documents/tabs sharing one storage area —
 *     which is the case for the web build's tabs, but not for separate Tauri
 *     webview windows). To keep both in sync there we ALSO broadcast a Tauri
 *     global event ('settings-changed') on every change, and every window
 *     subscribes to both channels on load.
 */

import { isTauri } from '../io/fileHost'

export type LengthUnit = 'm' | 'cm' | 'mm' | 'ft' | 'in'

export interface LengthUnitOption {
  value: LengthUnit
  label: string
}

/** Length units offered, matching SketchUp's unit choices. */
export const LENGTH_UNIT_OPTIONS: LengthUnitOption[] = [
  { value: 'm', label: 'Meters' },
  { value: 'cm', label: 'Centimeters' },
  { value: 'mm', label: 'Millimeters' },
  { value: 'ft', label: 'Feet' },
  { value: 'in', label: 'Inches' },
]

const STORAGE_KEY = 'hew.settings.lengthUnit'
const DEFAULT_UNIT: LengthUnit = 'm'

/** Meters-per-unit conversion factors. */
const METERS_PER_UNIT: Record<LengthUnit, number> = {
  m: 1,
  cm: 0.01,
  mm: 0.001,
  ft: 0.3048,
  in: 0.0254,
}

const UNIT_SUFFIX: Record<LengthUnit, string> = {
  m: 'm',
  cm: 'cm',
  mm: 'mm',
  ft: 'ft',
  in: 'in',
}

function isLengthUnit(v: unknown): v is LengthUnit {
  return v === 'm' || v === 'cm' || v === 'mm' || v === 'ft' || v === 'in'
}

function loadInitial(): LengthUnit {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (isLengthUnit(raw)) return raw
  } catch {
    /* ignore — privacy mode / unavailable storage */
  }
  return DEFAULT_UNIT
}

let currentUnit: LengthUnit = loadInitial()
const subscribers = new Set<(unit: LengthUnit) => void>()

function notify(): void {
  for (const cb of subscribers) cb(currentUnit)
}

/** Read the current length unit. */
export function getLengthUnit(): LengthUnit {
  return currentUnit
}

/**
 * Set the current length unit. Persists to localStorage, notifies local
 * subscribers, and broadcasts to other windows (Tauri global event; the
 * 'storage' event covers same-origin web tabs automatically).
 */
export function setLengthUnit(unit: LengthUnit): void {
  currentUnit = unit
  try {
    localStorage.setItem(STORAGE_KEY, unit)
  } catch {
    /* ignore quota / privacy-mode errors */
  }
  notify()
  broadcastTauri(unit)
}

/** Subscribe to unit changes (local + cross-window). Returns an unsubscribe fn. */
export function subscribe(cb: (unit: LengthUnit) => void): () => void {
  subscribers.add(cb)
  return () => subscribers.delete(cb)
}

/** Pure formatting helper — convert meters to `unit` and format with trimmed precision. */
export function formatLengthIn(meters: number, unit: LengthUnit): string {
  const converted = meters / METERS_PER_UNIT[unit]
  // Trim to a sensible precision per unit, then strip trailing zeros.
  const decimals = unit === 'm' ? 3 : unit === 'cm' || unit === 'ft' || unit === 'in' ? 2 : 1
  const rounded = converted.toFixed(decimals)
  const trimmed = rounded.includes('.')
    ? rounded.replace(/0+$/, '').replace(/\.$/, '')
    : rounded
  return `${trimmed} ${UNIT_SUFFIX[unit]}`
}

/** Format meters using the CURRENT singleton unit. */
export function formatLength(meters: number): string {
  return formatLengthIn(meters, currentUnit)
}

/**
 * Convert a value expressed in `unit` (default: the current singleton unit)
 * to meters. Inverse of dividing by `METERS_PER_UNIT` in `formatLengthIn`.
 */
export function metersFromUnit(value: number, unit: LengthUnit = getLengthUnit()): number {
  return value * METERS_PER_UNIT[unit]
}

/** The short suffix for `unit` (default: the current singleton unit), e.g. 'cm'. */
export function getLengthUnitSuffix(unit: LengthUnit = getLengthUnit()): string {
  return UNIT_SUFFIX[unit]
}

// ---------------------------------------------------------------------------
// Cross-window sync
// ---------------------------------------------------------------------------

let tauriEmit: ((event: string, payload?: unknown) => Promise<void>) | null = null

function broadcastTauri(unit: LengthUnit): void {
  if (!isTauri) return
  if (tauriEmit !== null) {
    tauriEmit('settings-changed', { lengthUnit: unit }).catch(() => { /* ignore */ })
    return
  }
  import('@tauri-apps/api/event').then(({ emit }) => {
    tauriEmit = emit
    return emit('settings-changed', { lengthUnit: unit })
  }).catch(() => { /* ignore */ })
}

function applyExternalUnit(next: unknown): void {
  if (!isLengthUnit(next) || next === currentUnit) return
  currentUnit = next
  notify()
}

// Refresh the singleton + notify subscribers when the OTHER window changes
// the unit. Two channels:
//   - Tauri global event 'settings-changed' (separate webview windows).
//   - Browser 'storage' event (same-origin web tabs).
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (ev) => {
    if (ev.key !== STORAGE_KEY) return
    applyExternalUnit(ev.newValue)
  })

  if (isTauri) {
    import('@tauri-apps/api/event').then(({ listen }) => {
      return listen<{ lengthUnit?: unknown }>('settings-changed', (event) => {
        applyExternalUnit(event.payload?.lengthUnit)
      })
    }).catch(() => { /* ignore */ })
  }
}
