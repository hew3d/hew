/**
 * Length-unit setting — module-level singleton.
 *
 * The kernel's base length unit is always f64 METERS (see docs/DEVELOPMENT.md). This
 * module owns ONLY a display-layer preference: which **format** the UI
 * formats lengths in. It never touches kernel state.
 *
 * Two-level system × format model:
 *   - Metric system  → formats: Meters ('m'), Centimeters ('cm'), Millimeters ('mm').
 *   - Imperial system → formats: Architectural ('arch', `5' 3-1/8"`),
 *     Fractional inches ('frac_in', `60-1/8"`), Decimal inches ('dec_in', `60.125"`).
 * `LengthFormat` is the source of truth; `LENGTH_SYSTEM_OF` derives the
 * system grouping for the UI, so there is exactly one place that knows the
 * mapping.
 *
 * Persistence + cross-window sync:
 *   - The current format is persisted to localStorage under
 *     `hew.settings.lengthUnit` (name kept for back-compat; see migration
 *     below — it now stores a `LengthFormat`, not the old `LengthUnit`).
 *   - Under Tauri, each window (main + the separate Settings window) is a
 *     distinct webview with its OWN localStorage-backed `window` object, so
 *     the native 'storage' event does NOT fire across them (that event only
 *     fires in OTHER same-origin documents/tabs sharing one storage area —
 *     which is the case for the web build's tabs, but not for separate Tauri
 *     webview windows). To keep both in sync there we ALSO broadcast a Tauri
 *     global event ('settings-changed') on every change, and every window
 *     subscribes to both channels on load.
 *
 * Back-compat: old persisted values were `'m'|'cm'|'mm'|'ft'|'in'`. On load
 * we migrate m→m, cm→cm, mm→mm, in→dec_in, ft→arch, then persist forward in
 * the new vocabulary.
 */

import { isTauri } from '../io/fileHost'

/** The unit/format a length is displayed and parsed in. */
export type LengthFormat = 'm' | 'cm' | 'mm' | 'arch' | 'frac_in' | 'dec_in'

/** The two unit systems; each format belongs to exactly one. */
export type LengthSystem = 'metric' | 'imperial'

/** Old (pre-) persisted vocabulary, kept only for the migration. */
type LegacyLengthUnit = 'm' | 'cm' | 'mm' | 'ft' | 'in'

export interface LengthFormatOption {
  value: LengthFormat
  label: string
}

/** Length formats offered, grouped by system. */
export const LENGTH_FORMAT_OPTIONS: LengthFormatOption[] = [
  { value: 'm', label: 'Meters' },
  { value: 'cm', label: 'Centimeters' },
  { value: 'mm', label: 'Millimeters' },
  { value: 'arch', label: 'Architectural (5\' 3-1/8")' },
  { value: 'frac_in', label: 'Fractional inches (60-1/8")' },
  { value: 'dec_in', label: 'Decimal inches (60.125")' },
]

/** Which system each format belongs to. The single source of truth for the
 * system × format grouping used by the Settings UI. */
export const LENGTH_SYSTEM_OF: Record<LengthFormat, LengthSystem> = {
  m: 'metric',
  cm: 'metric',
  mm: 'metric',
  arch: 'imperial',
  frac_in: 'imperial',
  dec_in: 'imperial',
}

/** Formats available within each system, in display order. */
export const LENGTH_FORMATS_BY_SYSTEM: Record<LengthSystem, LengthFormat[]> = {
  metric: ['m', 'cm', 'mm'],
  imperial: ['arch', 'frac_in', 'dec_in'],
}

/** The default format selected when switching TO a system. */
export const DEFAULT_FORMAT_FOR_SYSTEM: Record<LengthSystem, LengthFormat> = {
  metric: 'm',
  imperial: 'arch',
}

const STORAGE_KEY = 'hew.settings.lengthUnit'
const DEFAULT_FORMAT: LengthFormat = 'm'

/** Meters-per-unit conversion factors (metric formats; imperial formats use
 * the feet-inch-fraction grammar in `parseLengthToMeters` instead). */
const METERS_PER_UNIT: Record<'m' | 'cm' | 'mm', number> = {
  m: 1,
  cm: 0.01,
  mm: 0.001,
}

/** Exact meters-per-inch / meters-per-foot — never approximate these. */
const METERS_PER_INCH = 0.0254
const METERS_PER_FOOT = 0.3048
const INCHES_PER_FOOT = 12

/** Default display/rounding denominator for imperial fractions (nearest 1/16"). */
const DEFAULT_FRACTION_DENOMINATOR = 16

const UNIT_SUFFIX: Record<LengthFormat, string> = {
  m: 'm',
  cm: 'cm',
  mm: 'mm',
  arch: '', // composite (feet + inches) — no single suffix
  frac_in: '"',
  dec_in: '"',
}

function isLengthFormat(v: unknown): v is LengthFormat {
  return v === 'm' || v === 'cm' || v === 'mm' || v === 'arch' || v === 'frac_in' || v === 'dec_in'
}

function isLegacyLengthUnit(v: unknown): v is LegacyLengthUnit {
  return v === 'm' || v === 'cm' || v === 'mm' || v === 'ft' || v === 'in'
}

/** Migrate a legacy persisted value (`ft`/`in`) to the new vocabulary. New
 * values pass through unchanged. */
function migrateLegacy(v: LegacyLengthUnit): LengthFormat {
  if (v === 'ft') return 'arch'
  if (v === 'in') return 'dec_in'
  return v
}

function loadInitial(): LengthFormat {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (isLengthFormat(raw)) return raw
    if (isLegacyLengthUnit(raw)) {
      const migrated = migrateLegacy(raw)
      // Persist forward in the new vocabulary so subsequent loads (and the
      // other window, via storage/broadcast) see the migrated value.
      try {
        localStorage.setItem(STORAGE_KEY, migrated)
      } catch {
        /* ignore quota / privacy-mode errors */
      }
      return migrated
    }
  } catch {
    /* ignore — privacy mode / unavailable storage */
  }
  return DEFAULT_FORMAT
}

let currentFormat: LengthFormat = loadInitial()
const subscribers = new Set<(format: LengthFormat) => void>()

function notify(): void {
  for (const cb of subscribers) cb(currentFormat)
}

/** Read the current length format. */
export function getLengthUnit(): LengthFormat {
  return currentFormat
}

/** Read the system (Metric/Imperial) the current format belongs to. */
export function getLengthSystem(): LengthSystem {
  return LENGTH_SYSTEM_OF[currentFormat]
}

/**
 * Set the current length format. Persists to localStorage, notifies local
 * subscribers, and broadcasts to other windows (Tauri global event; the
 * 'storage' event covers same-origin web tabs automatically).
 */
export function setLengthUnit(format: LengthFormat): void {
  currentFormat = format
  try {
    localStorage.setItem(STORAGE_KEY, format)
  } catch {
    /* ignore quota / privacy-mode errors */
  }
  notify()
  broadcastTauri(format)
}

/** Subscribe to format changes (local + cross-window). Returns an unsubscribe fn. */
export function subscribe(cb: (format: LengthFormat) => void): () => void {
  subscribers.add(cb)
  return () => subscribers.delete(cb)
}

// ---------------------------------------------------------------------------
// Fraction helpers (imperial display)
// ---------------------------------------------------------------------------

/** Round `value` to the nearest 1/`denominator`, returning [whole, numerator,
 * denominator] with the fraction already reduced (numerator may be 0). */
function roundToFraction(
  value: number,
  denominator: number = DEFAULT_FRACTION_DENOMINATOR,
): { whole: number; num: number; den: number } {
  const totalSixteenths = Math.round(value * denominator)
  let whole = Math.floor(totalSixteenths / denominator)
  let num = totalSixteenths - whole * denominator
  if (num === 0) return { whole, num: 0, den: denominator }
  // Reduce the fraction.
  const g = gcd(num, denominator)
  const rDen = denominator / g
  num = num / g
  return { whole, num, den: rDen }
}

function gcd(a: number, b: number): number {
  let x = Math.abs(a)
  let y = Math.abs(b)
  while (y !== 0) {
    ;[x, y] = [y, x % y]
  }
  return x === 0 ? 1 : x
}

/** Format a non-negative inch value as `frac_in` digits without the sign or
 * trailing `"` — e.g. 60.125 -> "60-1/8", 0.5 -> "1/2", 60 -> "60". */
function formatFractionalInches(absInches: number): string {
  const { whole, num, den } = roundToFraction(absInches)
  if (num === 0) return `${whole}`
  if (whole === 0) return `${num}/${den}`
  return `${whole}-${num}/${den}`
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

/** Pure formatting helper — convert meters to `format` and render as text. */
export function formatLengthIn(meters: number, format: LengthFormat): string {
  if (format === 'm' || format === 'cm' || format === 'mm') {
    const converted = meters / METERS_PER_UNIT[format]
    const decimals = format === 'm' ? 3 : format === 'cm' ? 2 : 1
    const rounded = converted.toFixed(decimals)
    const trimmed = rounded.includes('.')
      ? rounded.replace(/0+$/, '').replace(/\.$/, '')
      : rounded
    return `${trimmed} ${UNIT_SUFFIX[format]}`
  }

  const sign = meters < 0 ? '-' : ''
  const totalInches = Math.abs(meters) / METERS_PER_INCH

  if (format === 'dec_in') {
    const rounded = totalInches.toFixed(3)
    const trimmed = rounded.includes('.')
      ? rounded.replace(/0+$/, '').replace(/\.$/, '')
      : rounded
    return `${sign}${trimmed}"`
  }

  if (format === 'frac_in') {
    return `${sign}${formatFractionalInches(totalInches)}"`
  }

  // arch: feet + inches + fraction. Round to the nearest 1/16" first so the
  // foot/inch carry (e.g. 11.97" -> 1') is computed from the SAME rounded
  // value the inches text displays, rather than rounding twice.
  const { whole: roundedWholeInches, num, den } = roundToFraction(totalInches)
  const feet = Math.floor(roundedWholeInches / INCHES_PER_FOOT)
  const inches = roundedWholeInches - feet * INCHES_PER_FOOT

  const inchFraction = num === 0 ? '' : `-${num}/${den}`
  const feetPart = feet > 0 ? `${feet}'` : ''

  if (feet > 0 && inches === 0 && num === 0) {
    return `${sign}${feetPart}`
  }
  const inchesPart = `${inches}${inchFraction}"`
  if (feet === 0) {
    return `${sign}${inchesPart}`
  }
  return `${sign}${feetPart} ${inchesPart}`
}

/** Format meters using the CURRENT singleton format. */
export function formatLength(meters: number): string {
  return formatLengthIn(meters, currentFormat)
}

// ---------------------------------------------------------------------------
// Parse
// ---------------------------------------------------------------------------

/**
 * Convert a value expressed in `unit` (default: the current singleton
 * format) to meters. Only meaningful for metric formats and the bare-number
 * case — imperial composite grammar goes through `parseLengthToMeters`.
 * Inverse of dividing by `METERS_PER_UNIT` in `formatLengthIn`.
 */
export function metersFromUnit(value: number, unit: LengthFormat = getLengthUnit()): number {
  if (unit === 'm' || unit === 'cm' || unit === 'mm') return value * METERS_PER_UNIT[unit]
  // Bare number in an imperial format is interpreted as inches.
  return value * METERS_PER_INCH
}

/** The short suffix for `format` (default: the current singleton format),
 * e.g. 'cm', '"'. Empty string for 'arch' (composite, no single suffix). */
export function getLengthUnitSuffix(format: LengthFormat = getLengthUnit()): string {
  return UNIT_SUFFIX[format]
}

/** Explicit-unit tokens in a typed component: feet/inch marks or a unit
 * letter ("cm", "ft", …). A bare `/` (fraction) is NOT one — "3 1/2" in an
 * imperial mode is still a bare number in display units. */
const EXPLICIT_UNIT_TOKEN_RE = /['"a-z]/i

/**
 * VCB display readout for a typed buffer.
 *
 * The display format's suffix is appended only while the buffer's current
 * component (after the last `,`/`x`/`X` dims separator, if any) is still a
 * bare number: the instant the component carries an explicit unit token
 * (`'`, `"`, or a unit letter), the default suffix disappears — "10cm" must
 * never read "10cm m". In a dims buffer the suffix reappears for the next
 * component after the separator, until that one gets its own unit.
 */
export function typedReadout(buf: string, format: LengthFormat = getLengthUnit()): string {
  const suffix = getLengthUnitSuffix(format)
  if (suffix === '' || buf === '') return buf
  const lastSep = Math.max(buf.lastIndexOf(','), buf.lastIndexOf('x'), buf.lastIndexOf('X'))
  const component = buf.slice(lastSep + 1).trim()
  // Nothing (or just a sign) typed in this component yet — no suffix to
  // dangle after a bare separator.
  if (component === '' || component === '-') return buf
  if (EXPLICIT_UNIT_TOKEN_RE.test(component)) return buf
  return `${buf} ${suffix}`
}

// Explicit unit suffixes accepted in typed input regardless of the current
// display format — "1cm", "100 mm", "2.5m", "3km", "5ft", "6in". The display
// format only governs how a BARE number is interpreted; an explicit suffix
// always wins. Case-insensitive.
const EXPLICIT_SUFFIX_METERS: Record<string, number> = {
  km: 1000,
  cm: 0.01,
  mm: 0.001,
  m: 1,
  ft: METERS_PER_FOOT,
  in: METERS_PER_INCH,
}

// number (optional sign, ".5"/"3." tolerated, matching what the VCB buffers
// can produce) + optional whitespace + one explicit unit word. Longer
// suffixes listed first so "mm"/"cm"/"km" are never truncated to "m".
const EXPLICIT_SUFFIX_RE = /^(-?(?:\d+\.?\d*|\.\d+))\s*(km|cm|mm|m|ft|in)$/i

// A plain decimal number, matching what the numeric VCB buffers can produce
// (leading '-', trailing '.', leading '.').
const BARE_NUMBER_RE = /^-?(?:\d+\.?\d*|\.\d+)$/

// Feet-inch-fraction grammar (SketchUp style):
//   5'            -> feet only
//   3"            -> inches only
//   5'3"  5' 3"   -> feet + inches (space optional)
//   5' 3-1/2"  5' 3 1/2"   -> feet + inches + fraction (hyphen or space)
//   3 1/2"  3-1/2"         -> inches + fraction
//   1/2"  5/8"             -> fraction only
//   60  60.125              -> bare number, interpreted as inches
const FEET_INCHES_RE =
  /^\s*(?:(-?\d+(?:\.\d+)?)\s*'\s*)?(?:(-?\d+(?:\.\d+)?)?(?:[\s-]+)?(?:(\d+)\/(\d+))?\s*"?\s*)?$/

/** Parse the feet-inch-fraction grammar to meters, or null if it doesn't
 * match. Shared by the explicit-mark path (any mode) and the imperial
 * bare-grammar fallback in `parseLengthToMeters`. */
function parseFeetInchesToMeters(trimmed: string): number | null {
  const m = FEET_INCHES_RE.exec(trimmed)
  if (m === null) return null
  const [, feetStr, inchStr, fracNumStr, fracDenStr] = m
  // Reject a match with no actual tokens (e.g. blank/whitespace-only, or a
  // lone `"`/`'` with no digits at all).
  if (feetStr === undefined && inchStr === undefined && fracNumStr === undefined) return null

  const feet = feetStr !== undefined ? parseFloat(feetStr) : 0
  let inches = inchStr !== undefined ? parseFloat(inchStr) : 0
  if (fracNumStr !== undefined && fracDenStr !== undefined) {
    const den = parseFloat(fracDenStr)
    if (den === 0) return null
    const fracValue = parseFloat(fracNumStr) / den
    inches += inches < 0 ? -fracValue : fracValue
  }

  const totalInches = feet * INCHES_PER_FOOT + inches
  return totalInches * METERS_PER_INCH
}

/**
 * Parse a typed length string to meters.
 *
 * Explicit units are accepted in ANY display mode — `format` only decides
 * how a BARE number is interpreted:
 *   - explicit suffix: "1cm", "100 mm", "2.5m", "3km", "5ft", "6in"
 *     (case-insensitive, optional space before the suffix);
 *   - explicit feet/inch marks: "5'", "23\"", "5'6\"", "5/8\"", "2-1/4\"",
 *     "5' 2-1/4\"" — anything containing a `'` or `"` goes through the
 *     feet-inch-fraction grammar;
 *   - bare number: interpreted in `format` (m/cm/mm for metric formats;
 *     inches for imperial formats). In imperial formats the mark-less
 *     feet-inch-fraction forms (e.g. "3 1/2") are also accepted, as before.
 * Returns null on empty/invalid input. Sign convention is unchanged: a
 * leading '-' negates the value.
 */
export function parseLengthToMeters(
  input: string,
  format: LengthFormat = getLengthUnit(),
): number | null {
  const trimmed = input.trim()
  if (trimmed === '') return null

  // Explicit unit suffix — honored in any mode.
  const suffixMatch = EXPLICIT_SUFFIX_RE.exec(trimmed)
  if (suffixMatch !== null) {
    const n = parseFloat(suffixMatch[1])
    if (!isFinite(n)) return null
    return n * EXPLICIT_SUFFIX_METERS[suffixMatch[2].toLowerCase()]
  }

  // Explicit feet/inch marks — honored in any mode.
  if (trimmed.includes("'") || trimmed.includes('"')) {
    return parseFeetInchesToMeters(trimmed)
  }

  if (format === 'm' || format === 'cm' || format === 'mm') {
    // Bare number in the active metric unit.
    if (!BARE_NUMBER_RE.test(trimmed)) return null
    const n = parseFloat(trimmed)
    if (!isFinite(n)) return null
    return n * METERS_PER_UNIT[format]
  }

  // Imperial: bare-number fast path first (covers "60", "60.125", "-3.5" —
  // including forms the feet/inch regex below would also accept, but
  // parseFloat is simpler and exact for this common case).
  if (/^-?\d+(?:\.\d+)?$/.test(trimmed)) {
    const n = parseFloat(trimmed)
    return n * METERS_PER_INCH
  }

  // Imperial: mark-less feet-inch-fraction forms, e.g. "3 1/2".
  return parseFeetInchesToMeters(trimmed)
}

/**
 * Parse a typed dimensions string (Rectangle VCB) into `[width, depth]` in
 * METERS. Each component goes through `parseLengthToMeters`, so components
 * may carry explicit units and mix them freely — "1cm,100mm", "5',23\"" —
 * regardless of the current display format.
 *
 * The component separator is a comma or `x`/`X` (whitespace can't be the
 * primary separator because it is part of the length grammar itself:
 * "5' 3\"", "1 cm"). A single component is a square. As a fallback, two
 * whitespace-separated components ("3 4", "1cm 2cm") are accepted when the
 * whole string doesn't parse as one length — preserving the legacy
 * space-separated form.
 *
 * Returns null if the string is empty/malformed or any component is not a
 * finite length > 0.
 */
export function parseDimensionsToMeters(
  input: string,
  format: LengthFormat = getLengthUnit(),
): [number, number] | null {
  const trimmed = input.trim()
  if (trimmed === '') return null

  const componentPair = (a: string, b: string): [number, number] | null => {
    if (a === '' || b === '') return null
    const w = parseLengthToMeters(a, format)
    const d = parseLengthToMeters(b, format)
    if (w === null || d === null || !isFinite(w) || !isFinite(d) || w <= 0 || d <= 0) return null
    return [w, d]
  }

  // Explicit separator first — unambiguous. Deliberately NOT filtering empty
  // segments — a leading/trailing/doubled separator ("3,", ",4", "3,,4")
  // must produce an empty part so it's rejected, rather than silently
  // treated as a single value.
  const parts = trimmed.split(/\s*[,xX]\s*/)
  if (parts.length > 2) return null
  if (parts.length === 2) return componentPair(parts[0], parts[1])

  // Single component: a square ("3", "1cm", "5' 3\"").
  const single = parseLengthToMeters(trimmed, format)
  if (single !== null) {
    if (!isFinite(single) || single <= 0) return null
    return [single, single]
  }

  // Legacy space-separated pair ("3 4"), including self-delimited unit
  // components ("1cm 2cm").
  const wsParts = trimmed.split(/\s+/)
  if (wsParts.length === 2) return componentPair(wsParts[0], wsParts[1])
  return null
}

// ---------------------------------------------------------------------------
// Cross-window sync
// ---------------------------------------------------------------------------

let tauriEmit: ((event: string, payload?: unknown) => Promise<void>) | null = null

function broadcastTauri(format: LengthFormat): void {
  if (!isTauri) return
  if (tauriEmit !== null) {
    tauriEmit('settings-changed', { lengthUnit: format }).catch(() => { /* ignore */ })
    return
  }
  import('@tauri-apps/api/event').then(({ emit }) => {
    tauriEmit = emit
    return emit('settings-changed', { lengthUnit: format })
  }).catch(() => { /* ignore */ })
}

function applyExternalFormat(next: unknown): void {
  if (!isLengthFormat(next) || next === currentFormat) return
  currentFormat = next
  notify()
}

// Refresh the singleton + notify subscribers when the OTHER window changes
// the format. Two channels:
//   - Tauri global event 'settings-changed' (separate webview windows).
//   - Browser 'storage' event (same-origin web tabs).
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (ev) => {
    if (ev.key !== STORAGE_KEY) return
    if (isLengthFormat(ev.newValue)) {
      applyExternalFormat(ev.newValue)
    } else if (isLegacyLengthUnit(ev.newValue)) {
      applyExternalFormat(migrateLegacy(ev.newValue))
    }
  })

  if (isTauri) {
    import('@tauri-apps/api/event').then(({ listen }) => {
      return listen<{ lengthUnit?: unknown }>('settings-changed', (event) => {
        applyExternalFormat(event.payload?.lengthUnit)
      })
    }).catch(() => { /* ignore */ })
  }
}
