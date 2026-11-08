/**
 * Length-unit setting — module-level singleton.
 *
 * The kernel's base length unit is always f64 METERS (see docs/dev/DEVELOPMENT.md). This
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
 * Precision (`Precision`, below) is a second display-only preference: how many
 * digits a readout shows. One value per display FORMAT (m, cm, mm and dec_in
 * count decimals; frac_in and arch pick a fraction of an inch, or a whole
 * inch or foot), defaulting to the rounding the readouts always had. A
 * choice that rounds a value prefixes `~`; Exact never rounds. Persisted
 * under `hew.settings.precision` and synced across windows like the format.
 *
 * Back-compat: old persisted values were `'m'|'cm'|'mm'|'ft'|'in'`. On load
 * we migrate m→m, cm→cm, mm→mm, in→dec_in, ft→arch, then persist forward in
 * the new vocabulary.
 */

import { msg, plural, t } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'
import { isTauri } from '../io/fileHost'
import { formatNumber } from '../i18n/format'

/** The unit/format a length is displayed and parsed in. */
export type LengthFormat = 'm' | 'cm' | 'mm' | 'arch' | 'frac_in' | 'dec_in'

/** The two unit systems; each format belongs to exactly one. */
export type LengthSystem = 'metric' | 'imperial'

/** Old (pre-) persisted vocabulary, kept only for the migration. */
type LegacyLengthUnit = 'm' | 'cm' | 'mm' | 'ft' | 'in'

export interface LengthFormatOption {
  value: LengthFormat
  label: MessageDescriptor
}

/** Length formats offered, grouped by system. */
export const LENGTH_FORMAT_OPTIONS: LengthFormatOption[] = [
  { value: 'm', label: msg`Meters` },
  { value: 'cm', label: msg`Centimeters` },
  { value: 'mm', label: msg`Millimeters` },
  { value: 'arch', label: msg`Architectural (5' 3-1/8")` },
  { value: 'frac_in', label: msg`Fractional inches (60-1/8")` },
  { value: 'dec_in', label: msg`Decimal inches (60.125")` },
]

/** Bare display name per format — no parenthetical example, unlike
 * `LENGTH_FORMAT_OPTIONS`'s `label` (which carries one for the desktop
 * dropdown, the one place with no live preview column to make the example
 * redundant). Shop Mode's unit picker and its Settings "Units" row
 * (design_handoff_shop_mode/README.md §7/§6) show just the name — the
 * picker already renders a live preview next to it. */
export const LENGTH_FORMAT_NAME: Record<LengthFormat, MessageDescriptor> = {
  m: msg`Meters`,
  cm: msg`Centimeters`,
  mm: msg`Millimeters`,
  arch: msg`Architectural`,
  frac_in: msg`Fractional inches`,
  dec_in: msg`Decimal inches`,
}

/** Short mono label for a compact chip (Shop Mode's cutlist header unit
 * chip, design §1 "Sheet header") — the format's own short NAME shown on its
 * own, distinct from `getLengthUnitSuffix` (the per-VALUE suffix appended
 * after a number). */
export const LENGTH_FORMAT_SHORT_LABEL: Record<LengthFormat, string> = {
  m: 'm',
  cm: 'cm',
  mm: 'mm',
  arch: 'ft-in',
  frac_in: 'in',
  dec_in: 'in.d',
}

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

/**
 * True if a format was ever persisted to this device (current or legacy
 * vocabulary) — i.e. `currentFormat` reflects an explicit past choice
 * rather than `DEFAULT_FORMAT`. Read-only: does NOT migrate/write, unlike
 * `loadInitial()` (that already ran once at module load). Exists for
 * one-time-seed callers — Shop Mode's locale-based unit default
 * (`shop/localeUnits.ts`) needs to tell "nothing chosen yet" apart from "the
 * user (or a previous seed) explicitly chose Meters" before writing a
 * locale-implied default, without this module's own default ever changing.
 */
export function hasPersistedLengthUnit(): boolean {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    return isLengthFormat(raw) || isLegacyLengthUnit(raw)
  } catch {
    return false
  }
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
  broadcastTauri({ lengthUnit: format })
}

/** Subscribe to format changes (local + cross-window). Returns an unsubscribe fn. */
export function subscribe(cb: (format: LengthFormat) => void): () => void {
  subscribers.add(cb)
  return () => subscribers.delete(cb)
}

// ---------------------------------------------------------------------------
// Precision (display-only; per format)
// ---------------------------------------------------------------------------

/** What a readout rounds to.
 *   - `'exact'`: the digits the value needs (with a ceiling), never rounded.
 *   - `'foot'`: the nearest 12" (`frac_in` / `arch` only).
 *   - a number, by format: decimals of the display unit for `m`, `cm`, `mm`
 *     and `dec_in` (0 to 6); the fraction denominator for `frac_in` and
 *     `arch` (1 = whole inches, 2 = halves, ... 64). */
export type Precision = 'exact' | 'foot' | number

const DECIMAL_PRECISIONS: Precision[] = ['exact', 0, 1, 2, 3, 4, 5, 6]
const FRACTION_PRECISIONS: Precision[] = ['exact', 'foot', 1, 2, 4, 8, 16, 32, 64]

/** Precision choices per format, in display order; the first is Exact. */
export const PRECISION_OPTIONS: Record<LengthFormat, Precision[]> = {
  m: DECIMAL_PRECISIONS,
  cm: DECIMAL_PRECISIONS,
  mm: DECIMAL_PRECISIONS,
  dec_in: DECIMAL_PRECISIONS,
  frac_in: FRACTION_PRECISIONS,
  arch: FRACTION_PRECISIONS,
}

/** The rounding each format always had before Precision existed. */
export const DEFAULT_PRECISION: Record<LengthFormat, Precision> = {
  m: 3,
  cm: 2,
  mm: 1,
  dec_in: 3,
  frac_in: 16,
  arch: 16,
}

const PRECISION_STORAGE_KEY = 'hew.settings.precision'

/** Micrometer: the tolerance for "the value is unchanged" in both Exact
 * round-tripping and the `~` decision. */
const ROUND_TRIP_METERS = 1e-6
/** Exact ceilings: decimals of the display unit (metric) and decimal inches
 * for the imperial fallback. */
const EXACT_METRIC_MAX_DECIMALS = 6
const EXACT_INCH_MAX_DECIMALS = 5
/** Finest fraction Exact will use for imperial. */
const EXACT_MAX_DENOMINATOR = 64
const EXACT_FRACTION_TOLERANCE_IN = 1e-6

function isValidPrecision(format: LengthFormat, v: unknown): v is Precision {
  return (PRECISION_OPTIONS[format] as unknown[]).includes(v)
}

type PrecisionMap = Record<LengthFormat, Precision>

const ALL_FORMATS: LengthFormat[] = ['m', 'cm', 'mm', 'arch', 'frac_in', 'dec_in']

function defaultPrecisionMap(): PrecisionMap {
  return { ...DEFAULT_PRECISION }
}

/** Turn a stored or received value into a complete map. Understands the
 * per-format shape and the older `{ metric, imperial }` one (a metric number
 * applies to m/cm/mm, an imperial denominator to frac_in/arch, `'exact'` to
 * every format the old value covered; dec_in keeps its default). Anything
 * invalid falls back to that format's default. */
function sanitizePrecisionMap(raw: unknown): PrecisionMap {
  const out = defaultPrecisionMap()
  if (typeof raw !== 'object' || raw === null) return out
  const r = raw as Record<string, unknown>
  const legacy = 'metric' in r || 'imperial' in r
  if (legacy) {
    const metric = r.metric
    if (metric === 'exact' || (typeof metric === 'number' && DECIMAL_PRECISIONS.includes(metric))) {
      out.m = out.cm = out.mm = metric
    }
    const imperial = r.imperial
    if (imperial === 'exact' || (typeof imperial === 'number' && FRACTION_PRECISIONS.includes(imperial))) {
      out.frac_in = out.arch = imperial
    }
    return out
  }
  for (const f of ALL_FORMATS) {
    if (isValidPrecision(f, r[f])) out[f] = r[f] as Precision
  }
  return out
}

function loadPrecision(): PrecisionMap {
  try {
    const raw = localStorage.getItem(PRECISION_STORAGE_KEY)
    if (raw !== null) return sanitizePrecisionMap(JSON.parse(raw))
  } catch {
    /* ignore — privacy mode / unavailable storage / bad JSON */
  }
  return defaultPrecisionMap()
}

let currentPrecision: PrecisionMap = loadPrecision()

/** The precision for `format` (default: the current format). */
export function getPrecision(format: LengthFormat = getLengthUnit()): Precision {
  return currentPrecision[format]
}

/** Set the precision for `format` (default: the current format). Persists,
 * notifies subscribers, and broadcasts to other windows. Ignores a value
 * that is not offered for that format. */
export function setPrecision(precision: Precision, format: LengthFormat = getLengthUnit()): void {
  if (!isValidPrecision(format, precision)) return
  currentPrecision = { ...currentPrecision, [format]: precision }
  try {
    localStorage.setItem(PRECISION_STORAGE_KEY, JSON.stringify(currentPrecision))
  } catch {
    /* ignore quota / privacy-mode errors */
  }
  notify()
  broadcastTauri({ precision: currentPrecision })
}

/** Label for a precision choice in the Units pane. */
export function precisionLabel(format: LengthFormat, p: Precision): string {
  if (p === 'exact') return t`Exact`
  if (p === 'foot') return t`Nearest 1'`
  if (LENGTH_SYSTEM_OF[format] === 'metric' || format === 'dec_in') {
    if (p === 0) {
      switch (format) {
        case 'm':
          return t`Whole meters`
        case 'cm':
          return t`Whole centimeters`
        case 'mm':
          return t`Whole millimeters`
        case 'dec_in':
          return t`Whole inches`
        default:
          return t`Whole units`
      }
    }
    return plural(p, { one: '# decimal', other: '# decimals' })
  }
  return p === 1 ? t`Nearest 1"` : t`Nearest 1/${p}"`
}

/**
 * How much closer than the meter-scale default the EMPTY-scene camera should
 * start for a display format. A small-scale format (cm, mm, inches) implies
 * a small model, so a blank document frames proportionally closer — the
 * default meter framing renders a 4×10 cm part near-invisibly tiny. Meters
 * and architectural feet keep the default. Only empty-scene framing reads
 * this (Viewport mount + the welcome screen's unit choice); it never moves
 * the camera over an existing model.
 */
const HOME_FRAMING_SCALE: Record<LengthFormat, number> = {
  m: 1,
  arch: 1,
  cm: 0.1,
  mm: 0.02,
  frac_in: 0.1,
  dec_in: 0.1,
}

/** The empty-scene framing scale for `format` (default: the current format). */
export function homeFramingScale(format: LengthFormat = getLengthUnit()): number {
  return HOME_FRAMING_SCALE[format]
}

// ---------------------------------------------------------------------------
// Fraction helpers (imperial display)
// ---------------------------------------------------------------------------

function gcd(a: number, b: number): number {
  let x = Math.abs(a)
  let y = Math.abs(b)
  while (y !== 0) {
    ;[x, y] = [y, x % y]
  }
  return x === 0 ? 1 : x
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

/** Shortest count of decimals (0..maxDecimals) at which `value` (in display
 * units, `metersPerUnit` meters each) is within a micrometer of itself. */
function shortestDecimals(value: number, metersPerUnit: number, maxDecimals: number): number {
  for (let d = 0; d < maxDecimals; d++) {
    const r = Math.round(value * 10 ** d) / 10 ** d
    if (Math.abs(r - value) * metersPerUnit <= ROUND_TRIP_METERS) return d
  }
  return maxDecimals
}

/** `ticks / 10^decimals` as trimmed decimal text, sign-free (ticks >= 0). */
function ticksToDecimalText(ticks: number, decimals: number): string {
  // Grouping off: a length reads "1234.5 m", never "1,234.5 m".
  return formatNumber(ticks / 10 ** decimals, { maxFractionDigits: decimals, grouping: false })
}

/** The inch grid a value is rounded to: `perInch` ticks per inch, and
 * `decimals` when the ticks are shown as a decimal (null = as a fraction). */
interface InchGrid {
  perInch: number
  decimals: number | null
}

/** Smallest power-of-two denominator up to 1/64 that holds `inches` exactly
 * (within 1e-6 in), or null. */
function exactDenominator(inches: number): number | null {
  for (let den = 1; den <= EXACT_MAX_DENOMINATOR; den *= 2) {
    if (Math.abs(inches * den - Math.round(inches * den)) / den <= EXACT_FRACTION_TOLERANCE_IN) return den
  }
  return null
}

/**
 * Pure formatting helper — convert meters to `format` and render as text.
 *
 * `precision` defaults to the saved choice for the format. Exact
 * shows the digits the value needs (metric: shortest decimal within 1 µm,
 * at most 6 decimals; imperial: the smallest power-of-two fraction up to
 * 1/64 that is exact, else decimal inches, at most 5 decimals) and never
 * carries `~`. A coarser precision rounds to its grid and prefixes `~` when
 * the rounding moved the value by more than 1 µm. The prefix leads the whole
 * text, ahead of any minus sign.
 */
export function formatLengthIn(
  meters: number,
  format: LengthFormat,
  precision?: Precision,
): string {
  const system = LENGTH_SYSTEM_OF[format]
  const p = precision !== undefined && isValidPrecision(format, precision) ? precision : getPrecision(format)

  if (system === 'metric') {
    const unit = METERS_PER_UNIT[format as 'm' | 'cm' | 'mm']
    const converted = meters / unit
    const decimals = p === 'exact' ? shortestDecimals(converted, unit, EXACT_METRIC_MAX_DECIMALS) : (p as number)
    const ticks = Math.round(Math.abs(converted) * 10 ** decimals)
    const rounded = ticks / 10 ** decimals
    const changed = p !== 'exact' && Math.abs(rounded * unit - Math.abs(meters)) > ROUND_TRIP_METERS
    const sign = meters < 0 && ticks !== 0 ? '-' : ''
    return `${changed ? '~' : ''}${sign}${ticksToDecimalText(ticks, decimals)} ${UNIT_SUFFIX[format]}`
  }

  const absInches = Math.abs(meters) / METERS_PER_INCH
  let grid: InchGrid
  if (p === 'exact') {
    const den = format === 'dec_in' ? null : exactDenominator(absInches)
    if (den !== null) {
      grid = { perInch: EXACT_MAX_DENOMINATOR, decimals: null }
    } else {
      const decimals = shortestDecimals(absInches, METERS_PER_INCH, EXACT_INCH_MAX_DECIMALS)
      grid = { perInch: 10 ** decimals, decimals }
    }
  } else {
    // dec_in counts decimals; the fractional formats pick a denominator
    // (1 = whole inches, 'foot' = whole inches in steps of 12).
    grid =
      format === 'dec_in'
        ? { perInch: 10 ** (p as number), decimals: p as number }
        : { perInch: p === 'foot' ? 1 : (p as number), decimals: null }
  }

  // Round to the grid ONCE; every digit (including the foot/inch carry) is
  // derived from the same integer tick count.
  const { perInch } = grid
  const ticks = p === 'foot' ? Math.round(absInches / INCHES_PER_FOOT) * INCHES_PER_FOOT : Math.round(absInches * perInch)
  const changed = p !== 'exact' && Math.abs(ticks / perInch - absInches) * METERS_PER_INCH > ROUND_TRIP_METERS
  const sign = meters < 0 && ticks !== 0 ? '-' : ''
  const tilde = changed ? '~' : ''

  const wholeInches = Math.floor(ticks / perInch)
  const rem = ticks - wholeInches * perInch // ticks past the whole inch

  // Part after the whole inch, '' when zero: reduced fraction or decimal digits.
  const isDec = grid.decimals !== null
  let fracText = ''
  if (rem !== 0) {
    if (grid.decimals === null) {
      const g = gcd(rem, perInch)
      fracText = `${rem / g}/${perInch / g}`
    } else {
      const digits = Math.round((rem * 10 ** grid.decimals) / perInch)
      fracText = String(digits).padStart(grid.decimals, '0').replace(/0+$/, '')
    }
  }

  if (format === 'dec_in' || format === 'frac_in') {
    let body: string
    if (isDec) body = formatNumber(ticks / perInch, { maxFractionDigits: grid.decimals as number, grouping: false })
    else if (fracText === '') body = `${wholeInches}`
    else body = wholeInches === 0 ? fracText : `${wholeInches}-${fracText}`
    return `${tilde}${sign}${body}"`
  }

  // arch: feet + inches (+ fraction). Feet and inches come from the rounded
  // tick count, so 11.97" carries to 1' rather than rounding twice.
  const feet = Math.floor(wholeInches / INCHES_PER_FOOT)
  const inches = wholeInches - feet * INCHES_PER_FOOT
  const feetPart = feet > 0 ? `${feet}'` : ''
  if (feet > 0 && inches === 0 && rem === 0) return `${tilde}${sign}${feetPart}`
  let inchesPart: string
  if (isDec) {
    inchesPart = `${formatNumber(inches + rem / perInch, { maxFractionDigits: grid.decimals as number, grouping: false })}"`
  } else if (rem === 0) {
    inchesPart = `${inches}"`
  } else if (feet === 0 && inches === 0) {
    // Standalone sub-inch value reads `3/4"`, not `0-3/4"`. Inside a
    // composite (`1' 0-3/4"`) the zero is the whole-inch count and stays.
    inchesPart = `${fracText}"`
  } else {
    inchesPart = `${inches}-${fracText}"`
  }
  return feet === 0 ? `${tilde}${sign}${inchesPart}` : `${tilde}${sign}${feetPart} ${inchesPart}`
}

// ---------------------------------------------------------------------------
// Area and volume (display only; the kernel is in meters)
// ---------------------------------------------------------------------------

/** Decimals for an area or volume: the saved precision when it counts
 * decimals (metric and decimal inches), else 2 — a fraction is never shown
 * for an area or a volume. */
function areaVolumeDecimals(format: LengthFormat, precision: Precision | undefined): number {
  const p = precision !== undefined && isValidPrecision(format, precision) ? precision : getPrecision(format)
  if (typeof p === 'number' && (LENGTH_SYSTEM_OF[format] === 'metric' || format === 'dec_in')) return p
  return 2
}

/** Rounding smaller than this (in the shown unit) is not marked with `~`. */
const ROUND_TRIP_AREA_VOLUME = 1e-6

/** `value` in the small unit, or in the large unit once the rounded small
 * value reaches `smallPerLarge` (so 999.999 cm² reads 0.1 m², not 1000 cm²). */
function pickAreaVolumeUnit(
  small: number,
  smallPerLarge: number,
  decimals: number,
  smallLabel: string,
  largeLabel: string,
): string {
  const rounded = Math.round(small * 10 ** decimals) / 10 ** decimals
  const useLarge = rounded >= smallPerLarge
  const n = useLarge ? small / smallPerLarge : small
  // Same rule as a length readout: a rounded value is marked with `~`.
  const shown = Math.round(n * 10 ** decimals) / 10 ** decimals
  const mark = Math.abs(shown - n) > ROUND_TRIP_AREA_VOLUME ? '~' : ''
  return `${mark}${formatNumber(n, { maxFractionDigits: decimals, grouping: false })} ${useLarge ? largeLabel : smallLabel}`
}

/* oxlint-disable lingui/no-unlocalized-strings -- unit symbols */
/**
 * An area (square meters) as text in the display system: metric shows cm²
 * below 1 m² and m² above; imperial shows in² below 1 ft² and ft² above.
 */
export function formatAreaIn(m2: number, format: LengthFormat, precision?: Precision): string {
  if (!Number.isFinite(m2)) return '\u2014'
  const decimals = areaVolumeDecimals(format, precision)
  if (LENGTH_SYSTEM_OF[format] === 'metric') {
    return pickAreaVolumeUnit(m2 * 1e4, 1e4, decimals, 'cm\u00b2', 'm\u00b2')
  }
  return pickAreaVolumeUnit(m2 / (METERS_PER_INCH * METERS_PER_INCH), INCHES_PER_FOOT ** 2, decimals, 'in\u00b2', 'ft\u00b2')
}

/**
 * A volume (cubic meters) as text in the display system: metric shows cm³
 * below 1 m³ and m³ above; imperial shows in³ below 1 ft³ and ft³ above.
 */
export function formatVolumeIn(m3: number, format: LengthFormat, precision?: Precision): string {
  if (!Number.isFinite(m3)) return '\u2014'
  const decimals = areaVolumeDecimals(format, precision)
  if (LENGTH_SYSTEM_OF[format] === 'metric') {
    return pickAreaVolumeUnit(m3 * 1e6, 1e6, decimals, 'cm\u00b3', 'm\u00b3')
  }
  return pickAreaVolumeUnit(m3 / METERS_PER_INCH ** 3, INCHES_PER_FOOT ** 3, decimals, 'in\u00b3', 'ft\u00b3')
}
/* oxlint-enable lingui/no-unlocalized-strings */

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

// A plain decimal number, matching what the numeric VCB buffers can produce
// (leading '-', trailing '.', leading '.').
const BARE_NUMBER_RE = /^-?(?:\d+\.?\d*|\.\d+)$/

// A bare fraction with an optional whole part — "1/2", "3 1/2", "6-3/4" —
// and an optional leading '-' that negates the whole value. Accepted as a
// bare number in EVERY display mode, so "3 1/2" means 3.5 of the display
// unit (3.5 m in meters mode, 3.5" in the imperial modes).
const BARE_FRACTION_RE = /^(-)?(?:(\d+)(?:\s+|-))?(\d+)\/(\d+)$/

/** Parse a bare number in display units: a plain decimal ("60", "-3.5",
 * ".5") or a fraction ("1/2", "3 1/2", "-6-3/4"). Returns the raw number
 * (NOT meters), or null if the string is neither. */
function parseBareNumber(text: string): number | null {
  if (BARE_NUMBER_RE.test(text)) {
    const n = parseFloat(text)
    return isFinite(n) ? n : null
  }
  const m = BARE_FRACTION_RE.exec(text)
  if (m === null) return null
  const den = parseFloat(m[4])
  if (den === 0) return null
  const value = (m[2] !== undefined ? parseFloat(m[2]) : 0) + parseFloat(m[3]) / den
  return m[1] !== undefined ? -value : value
}

/**
 * VCB display readout for a typed buffer.
 *
 * The display format's suffix is appended only while the buffer's current
 * component (after the last `,`/`x`/`X` dims separator, if any) actually
 * PARSES as a bare number in display units — a plain decimal or a fraction
 * ("3 1/2"). The instant the component carries an explicit unit token
 * (`'`, `"`, or a unit letter) or is incomplete ("3 1/", "24-", a lone
 * "-"), no suffix is shown: "10cm" must never read "10cm m", and a buffer
 * that would NOT commit on Enter must never be dressed up to look like it
 * would. In a dims buffer the rule re-applies per component after the
 * separator.
 */
export function typedReadout(buf: string, format: LengthFormat = getLengthUnit()): string {
  const suffix = getLengthUnitSuffix(format)
  if (suffix === '' || buf === '') return buf
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- dims separator characters typed by the user
  const lastSep = Math.max(buf.lastIndexOf(','), buf.lastIndexOf('x'), buf.lastIndexOf('X'))
  const component = buf.slice(lastSep + 1).trim()
  if (parseBareNumber(component) === null) return buf
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

// Feet-inch-fraction grammar (SketchUp style), UNSIGNED — the caller
// (`parseFeetInchesToMeters`) strips one optional leading '-' and applies it
// to the WHOLE value, so "-5'6\"" is -66" (not -60" + 6") and "-1/2\"" is
// -0.5" (the minus is never mistaken for the inch-fraction separator).
//   5'            -> feet only
//   3"            -> inches only
//   5'3"  5' 3"   -> feet + inches (space optional)
//   5' 3-1/2"  5' 3 1/2"   -> feet + inches + fraction (hyphen or space)
//   3 1/2"  3-1/2"         -> inches + fraction
//   1/2"  5/8"             -> fraction only
//   60  60.125              -> bare number, interpreted as inches
// A hyphen is ONLY valid as the digit-fraction separator ("3-1/2"): dangling
// or doubled hyphens ("24-", "3--1/2") do not match, so incomplete input is
// rejected instead of silently committing.
// The feet and inches components each also accept a leading-dot decimal
// with no integer part ("-.5\"", ".75\""), parsed exactly as "0.75" would
// be — matching what a numeric VCB buffer can produce. A bare "." (no
// digits at all) is not a number and is rejected, same as today.
// Groups: 1 = feet, 2 = inches, 3/4 = fraction after inches, 5/6 = fraction
// with no inches part.
const FEET_INCHES_RE =
  /^(?:(\d+\.?\d*|\.\d+)\s*'\s*)?(?:(?:(\d+\.?\d*|\.\d+)(?:(?:\s+|-)(\d+)\/(\d+))?|(\d+)\/(\d+))\s*"?)?\s*$/

/** Parse the feet-inch-fraction grammar to meters, or null if it doesn't
 * match. Shared by the explicit-mark path (any mode) and the imperial
 * bare-grammar fallback in `parseLengthToMeters`. One optional leading '-'
 * negates the whole value; the grammar itself is unsigned, so interior
 * signs ("5'-6\"") and non-fraction hyphens ("24-") are rejected. */
function parseFeetInchesToMeters(input: string): number | null {
  let trimmed = input.trim()
  let sign = 1
  if (trimmed.startsWith('-')) {
    sign = -1
    trimmed = trimmed.slice(1).trimStart()
  }
  const m = FEET_INCHES_RE.exec(trimmed)
  if (m === null) return null
  const [, feetStr, inchStr, fracNumA, fracDenA, fracNumB, fracDenB] = m
  const fracNumStr = fracNumA ?? fracNumB
  const fracDenStr = fracDenA ?? fracDenB
  // Reject a match with no actual tokens (e.g. blank/whitespace-only, or a
  // lone `-` with no digits at all).
  if (feetStr === undefined && inchStr === undefined && fracNumStr === undefined) return null

  const feet = feetStr !== undefined ? parseFloat(feetStr) : 0
  let inches = inchStr !== undefined ? parseFloat(inchStr) : 0
  if (fracNumStr !== undefined && fracDenStr !== undefined) {
    const den = parseFloat(fracDenStr)
    if (den === 0) return null
    inches += parseFloat(fracNumStr) / den
  }

  const totalInches = feet * INCHES_PER_FOOT + inches
  return sign * totalInches * METERS_PER_INCH
}

/** Trim `input` and drop one leading `~` (the "rounded readout" mark), so a
 * value copied from a readout can be typed back in. */
function stripApproxPrefix(input: string): string {
  const t = input.trim()
  return t.startsWith('~') ? t.slice(1).trimStart() : t
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
 *     inches for imperial formats). Bare fractions ("1/2", "3 1/2",
 *     "6-3/4") are accepted as bare numbers in EVERY display mode, so
 *     "3 1/2" is 3.5 m in meters mode and 3.5" in the imperial modes.
 * A leading-dot decimal with no integer part (".75", ".75\"", "-.5",
 * ".5cm") parses exactly as "0.75"/"0.5" would, in every one of the forms
 * above — a plain "." with no digits at all is still not a number.
 * Returns null on empty/invalid input — including a dangling fraction
 * hyphen ("24-"), which is incomplete input and must not silently commit
 * as 24. Sign convention: ONE optional leading '-' negates the WHOLE value
 * ("-5'6\"" is -66", "-1/2\"" is -0.5"); interior signs are rejected.
 */
export function parseLengthToMeters(
  input: string,
  format: LengthFormat = getLengthUnit(),
): number | null {
  const trimmed = stripApproxPrefix(input)
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
    // Bare number — plain decimal OR fraction ("3 1/2" = 3.5) — in the
    // active metric unit.
    const n = parseBareNumber(trimmed)
    if (n === null) return null
    return n * METERS_PER_UNIT[format]
  }

  // Imperial: bare-number fast path first (covers "60", "60.125", "-3.5",
  // "-.5" — including forms the feet/inch regex below would also accept,
  // but parseFloat is simpler and exact for this common case).
  if (/^-?(?:\d+\.?\d*|\.\d+)$/.test(trimmed)) {
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
 * "5' 3\"", "1 cm" — `editDimsBuffer` follows the same rule and never
 * treats a typed space as the separator). A single component is a square.
 * As a fallback, two whitespace-separated components ("3 4", "1cm 2cm") are
 * accepted when the whole string doesn't parse as one length — preserving
 * the legacy space-separated form.
 *
 * Returns null if the string is empty/malformed or any component is not a
 * finite length > 0.
 */
export function parseDimensionsToMeters(
  input: string,
  format: LengthFormat = getLengthUnit(),
): [number, number] | null {
  const trimmed = stripApproxPrefix(input)
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

function broadcastTauri(payload: { lengthUnit?: LengthFormat; precision?: PrecisionMap }): void {
  if (!isTauri) return
  if (tauriEmit !== null) {
    tauriEmit('settings-changed', payload).catch(() => { /* ignore */ })
    return
  }
  import('@tauri-apps/api/event').then(({ emit }) => {
    tauriEmit = emit
    return emit('settings-changed', payload)
  }).catch(() => { /* ignore */ })
}

function applyExternalFormat(next: unknown): void {
  if (!isLengthFormat(next) || next === currentFormat) return
  currentFormat = next
  notify()
}

function applyExternalPrecision(next: unknown): void {
  if (typeof next !== 'object' || next === null) return
  const clean = sanitizePrecisionMap(next)
  if (ALL_FORMATS.every((f) => clean[f] === currentPrecision[f])) return
  currentPrecision = clean
  notify()
}

// Refresh the singleton + notify subscribers when the OTHER window changes
// the format. Two channels:
//   - Tauri global event 'settings-changed' (separate webview windows).
//   - Browser 'storage' event (same-origin web tabs).
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (ev) => {
    if (ev.key === PRECISION_STORAGE_KEY) {
      try {
        applyExternalPrecision(ev.newValue === null ? null : JSON.parse(ev.newValue))
      } catch {
        /* ignore bad JSON */
      }
      return
    }
    if (ev.key !== STORAGE_KEY) return
    if (isLengthFormat(ev.newValue)) {
      applyExternalFormat(ev.newValue)
    } else if (isLegacyLengthUnit(ev.newValue)) {
      applyExternalFormat(migrateLegacy(ev.newValue))
    }
  })

  if (isTauri) {
    import('@tauri-apps/api/event').then(({ listen }) => {
      return listen<{ lengthUnit?: unknown; precision?: unknown }>('settings-changed', (event) => {
        applyExternalFormat(event.payload?.lengthUnit)
        applyExternalPrecision(event.payload?.precision)
      })
    }).catch(() => { /* ignore */ })
  }
}
