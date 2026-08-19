/**
 * Print preferences (docs/design/printing.md §10): the last-used dialog
 * values, per mode, plus paper/orientation/margins shared by both. App-level
 * settings (localStorage), never document state — a print job is not model
 * data. Same singleton + cross-window pattern as `sceneTransitions.ts`.
 *
 * `paperSeeded` records that the paper was seeded once (from the OS default
 * on desktop, from the locale on the web) so the seed never overrides a
 * later user choice.
 */
import { isTauri } from '../io/fileHost'
import type { PrintScale } from '../print/scale'
import type { MarginPreset, Orientation, PaperSpec } from '../print/paper'
import type { PrintExtentKind, PrintStyle, PrintViewKind, PrintZoom } from '../print/printJob'

export interface PrintPrefs {
  paper: PaperSpec
  orientation: Orientation
  margin: MarginPreset
  lastMode: 'standard' | 'scaled'
  paperSeeded: boolean
  cutList: boolean
  standard: { style: PrintStyle; includeGuides: boolean; includeGridAxes: boolean; titleBlock: boolean; zoom: PrintZoom }
  scaled: {
    style: PrintStyle
    includeGuides: boolean
    hiddenDashed: boolean
    overlap: boolean
    marks: boolean
    scaleBar: boolean
    titleBlock: boolean
    scale: PrintScale | null
    extent: PrintExtentKind
    view: PrintViewKind
  }
}

const STORAGE_KEY = 'hew.settings.print'

export const DEFAULT_PRINT_PREFS: PrintPrefs = {
  paper: 'a4',
  orientation: 'auto',
  margin: 'normal',
  lastMode: 'standard',
  paperSeeded: false,
  cutList: false,
  standard: { style: 'shaded', includeGuides: false, includeGridAxes: false, titleBlock: true, zoom: 'current' },
  scaled: { style: 'lineart', includeGuides: false, hiddenDashed: false, overlap: true, marks: true, scaleBar: true, titleBlock: true, scale: null, extent: 'model', view: 'current' },
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null
}

/** Merge a stored blob over the defaults, dropping anything malformed. */
export function normalizePrintPrefs(raw: unknown): PrintPrefs {
  const d = DEFAULT_PRINT_PREFS
  if (!isRecord(raw)) return structuredClone(d)
  const paperNames = new Set(['letter', 'legal', 'tabloid', 'a5', 'a4', 'a3'])
  const paper: PaperSpec =
    typeof raw.paper === 'string' && paperNames.has(raw.paper)
      ? (raw.paper as PaperSpec)
      : isRecord(raw.paper) && typeof raw.paper.w_mm === 'number' && typeof raw.paper.h_mm === 'number'
        ? { w_mm: raw.paper.w_mm, h_mm: raw.paper.h_mm }
        : d.paper
  const orientation = raw.orientation === 'portrait' || raw.orientation === 'landscape' || raw.orientation === 'auto' ? raw.orientation : d.orientation
  const margin = raw.margin === 'narrow' || raw.margin === 'normal' ? raw.margin : d.margin
  const lastMode = raw.lastMode === 'scaled' ? 'scaled' : 'standard'
  const bool = (v: unknown, dflt: boolean): boolean => (typeof v === 'boolean' ? v : dflt)
  const style = (v: unknown, dflt: PrintStyle): PrintStyle => (v === 'shaded' || v === 'lineart' ? v : dflt)
  const st = isRecord(raw.standard) ? raw.standard : {}
  const sc = isRecord(raw.scaled) ? raw.scaled : {}
  let scale: PrintScale | null = null
  if (isRecord(sc.scale) && typeof sc.scale.paperMeters === 'number' && typeof sc.scale.modelMeters === 'number' && typeof sc.scale.label === 'string') {
    if (sc.scale.paperMeters > 0 && sc.scale.modelMeters > 0) {
      scale = { paperMeters: sc.scale.paperMeters, modelMeters: sc.scale.modelMeters, label: sc.scale.label, kind: sc.scale.kind === 'custom' ? 'custom' : sc.scale.kind === 'fit' ? 'fit' : 'preset' }
    }
  }
  const extents = new Set(['model', 'selection', 'view'])
  const views = new Set(['current', 'top', 'bottom', 'front', 'back', 'left', 'right', 'iso'])
  return {
    paper,
    orientation,
    margin,
    lastMode,
    paperSeeded: bool(raw.paperSeeded, false),
    cutList: bool(raw.cutList, false),
    standard: {
      style: style(st.style, d.standard.style),
      includeGuides: bool(st.includeGuides, d.standard.includeGuides),
      includeGridAxes: bool(st.includeGridAxes, d.standard.includeGridAxes),
      titleBlock: bool(st.titleBlock, d.standard.titleBlock),
      zoom: st.zoom === 'fit' ? 'fit' : 'current',
    },
    scaled: {
      style: style(sc.style, d.scaled.style),
      includeGuides: bool(sc.includeGuides, d.scaled.includeGuides),
      hiddenDashed: bool(sc.hiddenDashed, d.scaled.hiddenDashed),
      overlap: bool(sc.overlap, d.scaled.overlap),
      marks: bool(sc.marks, d.scaled.marks),
      scaleBar: bool(sc.scaleBar, d.scaled.scaleBar),
      titleBlock: bool(sc.titleBlock, d.scaled.titleBlock),
      scale,
      extent: typeof sc.extent === 'string' && extents.has(sc.extent) ? (sc.extent as PrintExtentKind) : d.scaled.extent,
      view: typeof sc.view === 'string' && views.has(sc.view) ? (sc.view as PrintViewKind) : d.scaled.view,
    },
  }
}

function loadInitial(): PrintPrefs {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw !== null) return normalizePrintPrefs(JSON.parse(raw))
  } catch {
    /* ignore — privacy mode / unavailable storage / bad JSON */
  }
  return structuredClone(DEFAULT_PRINT_PREFS)
}

let current: PrintPrefs = loadInitial()
const subscribers = new Set<(p: PrintPrefs) => void>()

function notify(): void {
  for (const cb of subscribers) cb(current)
}

export function getPrintPrefs(): PrintPrefs {
  return current
}

/** Replace the prefs (partial updates: spread over `getPrintPrefs()`). */
export function setPrintPrefs(next: PrintPrefs): void {
  current = normalizePrintPrefs(next)
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(current))
  } catch {
    /* ignore quota / privacy-mode errors */
  }
  notify()
  broadcastTauri(current)
}

export function subscribe(cb: (p: PrintPrefs) => void): () => void {
  subscribers.add(cb)
  return () => subscribers.delete(cb)
}

// ---------------------------------------------------------------------------
// Cross-window sync (see sceneTransitions.ts)
// ---------------------------------------------------------------------------

let tauriEmit: ((event: string, payload?: unknown) => Promise<void>) | null = null

function broadcastTauri(p: PrintPrefs): void {
  if (!isTauri) return
  if (tauriEmit !== null) {
    tauriEmit('settings-changed', { print: p }).catch(() => { /* ignore */ })
    return
  }
  import('@tauri-apps/api/event').then(({ emit }) => {
    tauriEmit = emit
    return emit('settings-changed', { print: p })
  }).catch(() => { /* ignore */ })
}

function applyExternal(next: unknown): void {
  if (next === undefined || next === null) return
  current = normalizePrintPrefs(next)
  notify()
}

if (typeof window !== 'undefined') {
  window.addEventListener('storage', (ev) => {
    if (ev.key !== STORAGE_KEY || ev.newValue === null) return
    try {
      applyExternal(JSON.parse(ev.newValue))
    } catch {
      /* ignore */
    }
  })
  if (isTauri) {
    import('@tauri-apps/api/event').then(({ listen }) => {
      return listen<{ print?: unknown }>('settings-changed', (event) => {
        applyExternal(event.payload?.print)
      })
    }).catch(() => { /* ignore */ })
  }
}
