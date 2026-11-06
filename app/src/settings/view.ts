/**
 * View settings — module-level singletons for the Settings ▸ View pane.
 *
 * Five small per-person preferences about how the viewport feels:
 *   - zoom wheel speed   (five stops, Slow to Fast; the middle is today's speed)
 *   - smooth camera      (animated camera moves and orbit inertia, default on)
 *   - show tool hints    (the text that follows the pointer, default on)
 *   - show tool pointer  (the tool-shaped cursor icon, default on)
 *   - Measurements box position (top right or bottom right, default top right)
 *
 * Every default is today's behavior, so nothing changes until someone picks.
 *
 * Persistence + cross-window sync mirrors settings/theme.ts exactly, once per
 * setting (a small factory below keeps the five copies identical):
 *   - Persisted to localStorage under `hew.settings.<name>`.
 *   - Under Tauri, separate webview windows (main + Settings) do NOT share a
 *     `storage` event, so changes are ALSO broadcast via the same
 *     'settings-changed' Tauri global event the other settings use, each with
 *     its own payload key so every listener coexists on one channel.
 *   - The browser 'storage' event covers same-origin web tabs.
 */

import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'
import { isTauri } from '../io/fileHost'

export interface ViewSetting<T> {
  get(): T
  set(next: T): void
  subscribe(cb: (value: T) => void): () => void
}

export function makeSetting<T>(
  storageKey: string,
  payloadKey: string,
  fallback: T,
  parse: (raw: unknown) => T | undefined,
  serialize: (value: T) => string,
): ViewSetting<T> {
  function loadInitial(): T {
    try {
      const parsed = parse(localStorage.getItem(storageKey))
      if (parsed !== undefined) return parsed
    } catch {
      /* ignore — privacy mode / unavailable storage */
    }
    return fallback
  }

  let current: T = loadInitial()
  const subscribers = new Set<(value: T) => void>()
  let tauriEmit: ((event: string, payload?: unknown) => Promise<void>) | null = null

  function notify(): void {
    for (const cb of subscribers) cb(current)
  }

  function broadcastTauri(value: T): void {
    if (!isTauri) return
    if (tauriEmit !== null) {
      tauriEmit('settings-changed', { [payloadKey]: value }).catch(() => { /* ignore */ })
      return
    }
    import('@tauri-apps/api/event').then(({ emit }) => {
      tauriEmit = emit
      return emit('settings-changed', { [payloadKey]: value })
    }).catch(() => { /* ignore */ })
  }

  function applyExternal(next: T | undefined): void {
    if (next === undefined || next === current) return
    current = next
    notify()
  }

  if (typeof window !== 'undefined') {
    window.addEventListener('storage', (ev) => {
      if (ev.key !== storageKey) return
      applyExternal(parse(ev.newValue))
    })

    if (isTauri) {
      import('@tauri-apps/api/event').then(({ listen }) => {
        return listen<Record<string, unknown>>('settings-changed', (event) => {
          const payload = event.payload
          if (payload === null || typeof payload !== 'object' || !(payloadKey in payload)) return
          applyExternal(parse(payload[payloadKey]))
        })
      }).catch(() => { /* ignore */ })
    }
  }

  return {
    get: () => current,
    set(next: T): void {
      current = next
      try {
        localStorage.setItem(storageKey, serialize(next))
      } catch {
        /* ignore quota / privacy-mode errors */
      }
      notify()
      broadcastTauri(next)
    },
    subscribe(cb: (value: T) => void): () => void {
      subscribers.add(cb)
      return () => subscribers.delete(cb)
    },
  }
}

/** Parses a persisted/broadcast boolean given as 'true'/'false' or a real boolean. */
function parseBool(raw: unknown): boolean | undefined {
  if (raw === true || raw === 'true') return true
  if (raw === false || raw === 'false') return false
  return undefined
}

// ---------------------------------------------------------------------------
// Zoom wheel speed
// ---------------------------------------------------------------------------

/** Multipliers on the wheel dolly step, Slow to Fast. The middle (1) is today's speed. */
export const ZOOM_WHEEL_FACTORS = [0.5, 0.71, 1, 1.41, 2] as const
/** Index of the default (today's speed) stop. */
export const DEFAULT_ZOOM_WHEEL_STOP = 2
export const ZOOM_WHEEL_LABELS: readonly MessageDescriptor[] = [msg`Slow`, msg`Slower`, msg`Normal`, msg`Faster`, msg`Fast`]

function parseStop(raw: unknown): number | undefined {
  if (raw === null || raw === undefined || raw === '') return undefined
  const n = typeof raw === 'number' ? raw : Number(raw)
  return Number.isInteger(n) && n >= 0 && n < ZOOM_WHEEL_FACTORS.length ? n : undefined
}

const zoomWheelStop = makeSetting<number>(
  'hew.settings.zoomWheelSpeed', 'zoomWheelSpeed', DEFAULT_ZOOM_WHEEL_STOP, parseStop, String,
)
/** The chosen wheel-speed stop, 0 (Slow) to 4 (Fast). */
export const getZoomWheelStop = zoomWheelStop.get
export const setZoomWheelStop = zoomWheelStop.set
export const subscribeZoomWheelStop = zoomWheelStop.subscribe

/** The multiplier for a stop (out-of-range stops clamp to the default). */
export function zoomWheelFactor(stop: number): number {
  return ZOOM_WHEEL_FACTORS[stop] ?? ZOOM_WHEEL_FACTORS[DEFAULT_ZOOM_WHEEL_STOP]
}

/**
 * The value for OrbitControls' `zoomSpeed` (its default, 1, is today's
 * speed): the wheel dolly step is scaled by this.
 */
export function getZoomWheelSpeed(): number {
  return zoomWheelFactor(zoomWheelStop.get())
}

// ---------------------------------------------------------------------------
// Smooth camera
// ---------------------------------------------------------------------------

const smoothCamera = makeSetting<boolean>(
  'hew.settings.smoothCamera', 'smoothCamera', true, parseBool, String,
)
export const getSmoothCamera = smoothCamera.get
export const setSmoothCamera = smoothCamera.set
export const subscribeSmoothCamera = smoothCamera.subscribe

/**
 * Whether a camera move should animate. Off when the requested duration is
 * zero, the OS asks for reduced motion, or Smooth camera moves is off (which
 * wins over the Scene Transitions menu check).
 */
export function shouldAnimateCamera(durationMs: number, reducedMotion: boolean, smooth: boolean): boolean {
  return durationMs > 0 && !reducedMotion && smooth
}

/** Whether orbit/pan inertia (the coasting tail after a drag) is on. */
export function inertiaEnabled(preciseOrbitHeld: boolean, smooth: boolean): boolean {
  return smooth && !preciseOrbitHeld
}

// ---------------------------------------------------------------------------
// Cursor hints
// ---------------------------------------------------------------------------

const showToolHints = makeSetting<boolean>(
  'hew.settings.showToolHints', 'showToolHints', true, parseBool, String,
)
export const getShowToolHints = showToolHints.get
export const setShowToolHints = showToolHints.set
export const subscribeShowToolHints = showToolHints.subscribe

const showToolPointer = makeSetting<boolean>(
  'hew.settings.showToolPointer', 'showToolPointer', true, parseBool, String,
)
export const getShowToolPointer = showToolPointer.get
export const setShowToolPointer = showToolPointer.set
export const subscribeShowToolPointer = showToolPointer.subscribe

/** The CSS cursor used when the tool-shaped pointer is switched off. */
export const PLAIN_CURSOR = 'default'

/** The canvas cursor: the tool-shaped one, or the plain arrow when that is off. */
export function resolveToolCursor(showPointer: boolean, toolCursor: () => string): string {
  return showPointer ? toolCursor() : PLAIN_CURSOR
}

/** Cursor overrides that outrank the tool cursor while they last. */
export interface TransientCursorState {
  shiftPan: boolean
  shiftFov: boolean
  fovDrag: boolean
  eyedropper: boolean
}

/**
 * Whether flipping the pointer setting may repaint the tool cursor right now.
 * Not while a transient cursor (Shift-pan, Shift-fov, a fov drag, the paint
 * eyedropper) is live: that gesture's own writer restores through the same
 * setting-aware pipeline when it ends.
 */
export function mayReapplyToolCursor(t: TransientCursorState): boolean {
  return !(t.shiftPan || t.shiftFov || t.fovDrag || t.eyedropper)
}

// ---------------------------------------------------------------------------
// Measurements box position
// ---------------------------------------------------------------------------

export type MeasurementsPosition = 'top-right' | 'bottom-right'

function parsePosition(raw: unknown): MeasurementsPosition | undefined {
  return raw === 'top-right' || raw === 'bottom-right' ? raw : undefined
}

const measurementsPosition = makeSetting<MeasurementsPosition>(
  'hew.settings.measurementsPosition', 'measurementsPosition', 'top-right', parsePosition, (v) => v,
)
export const getMeasurementsPosition = measurementsPosition.get
export const setMeasurementsPosition = measurementsPosition.set
export const subscribeMeasurementsPosition = measurementsPosition.subscribe

// ---------------------------------------------------------------------------
// Outliner sort
// ---------------------------------------------------------------------------

/** How the Outliner orders the rows at each level. */
export type OutlinerSort = 'document' | 'name' | 'kind'

export const OUTLINER_SORT_LABELS: Record<OutlinerSort, MessageDescriptor> = {
  document: msg`Document order`,
  name: msg`Name`,
  kind: msg`Kind`,
}

function parseOutlinerSort(raw: unknown): OutlinerSort | undefined {
  return raw === 'document' || raw === 'name' || raw === 'kind' ? raw : undefined
}

const outlinerSort = makeSetting<OutlinerSort>(
  'hew.settings.outlinerSort', 'outlinerSort', 'document', parseOutlinerSort, (v) => v,
)
export const getOutlinerSort = outlinerSort.get
export const setOutlinerSort = outlinerSort.set
export const subscribeOutlinerSort = outlinerSort.subscribe
