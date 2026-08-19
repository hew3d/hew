/**
 * Print scale model (docs/design/printing.md §5): a paper:model ratio held
 * with the lengths that produced it so "1 in = 2 ft" prints as typed, plus
 * the preset ladders per unit family, custom-entry parsing, and Fit.
 */
import { LENGTH_SYSTEM_OF, formatLengthIn, parseLengthToMeters, type LengthFormat, type LengthSystem } from '../settings/units'

export interface PrintScale {
  /** Length on paper, meters. */
  paperMeters: number
  /** Model length it represents, meters. */
  modelMeters: number
  /** Human label, printed as typed for custom scales: "1 in = 1 ft", "1:10". */
  label: string
  kind: 'preset' | 'custom' | 'fit'
}

/** paper : model — 0.1 for 1:10, 12 for 12:1. */
export function scaleRatio(s: PrintScale): number {
  return s.paperMeters / s.modelMeters
}

/** Ratio bounds for any accepted scale (custom entry is refused outside). */
export const MIN_RATIO = 1 / 1000
export const MAX_RATIO = 20

const INCH = 0.0254
const FOOT = 0.3048

interface Preset {
  paperMeters: number
  modelMeters: number
  label: string
}

/** Metric ladder, largest scale first. Labels: ratio + a unit hint. */
const METRIC_LADDER: Preset[] = [
  { paperMeters: 2, modelMeters: 1, label: '2:1' },
  { paperMeters: 1, modelMeters: 1, label: '1:1' },
  { paperMeters: 1, modelMeters: 2, label: '1:2' },
  { paperMeters: 1, modelMeters: 5, label: '1:5' },
  { paperMeters: 1, modelMeters: 10, label: '1:10' },
  { paperMeters: 1, modelMeters: 20, label: '1:20' },
  { paperMeters: 1, modelMeters: 25, label: '1:25' },
  { paperMeters: 1, modelMeters: 50, label: '1:50' },
  { paperMeters: 1, modelMeters: 100, label: '1:100' },
  { paperMeters: 1, modelMeters: 200, label: '1:200' },
]

/** Imperial / architectural ladder, largest scale first. */
const IMPERIAL_LADDER: Preset[] = [
  { paperMeters: 2 * INCH, modelMeters: 1 * INCH, label: '2" = 1"' },
  { paperMeters: 1 * INCH, modelMeters: 1 * INCH, label: '1" = 1"' },
  { paperMeters: 6 * INCH, modelMeters: 1 * FOOT, label: '6" = 1\'' },
  { paperMeters: 3 * INCH, modelMeters: 1 * FOOT, label: '3" = 1\'' },
  { paperMeters: 1.5 * INCH, modelMeters: 1 * FOOT, label: '1½" = 1\'' },
  { paperMeters: 1 * INCH, modelMeters: 1 * FOOT, label: '1" = 1\'' },
  { paperMeters: 0.75 * INCH, modelMeters: 1 * FOOT, label: '¾" = 1\'' },
  { paperMeters: 0.5 * INCH, modelMeters: 1 * FOOT, label: '½" = 1\'' },
  { paperMeters: 0.375 * INCH, modelMeters: 1 * FOOT, label: '⅜" = 1\'' },
  { paperMeters: 0.25 * INCH, modelMeters: 1 * FOOT, label: '¼" = 1\'' },
  { paperMeters: 0.125 * INCH, modelMeters: 1 * FOOT, label: '⅛" = 1\'' },
  { paperMeters: 0.0625 * INCH, modelMeters: 1 * FOOT, label: '1/16" = 1\'' },
]

/** The preset ladder for a unit family, largest scale first. */
export function scalePresets(system: LengthSystem): PrintScale[] {
  const ladder = system === 'metric' ? METRIC_LADDER : IMPERIAL_LADDER
  return ladder.map((p) => ({ ...p, kind: 'preset' as const }))
}

export function scalePresetsFor(format: LengthFormat): PrintScale[] {
  return scalePresets(LENGTH_SYSTEM_OF[format])
}

/** "1:24" style ratio text for any scale (integers where they are close). */
export function ratioText(s: PrintScale): string {
  const r = scaleRatio(s)
  const near = (v: number): string => (Math.abs(v - Math.round(v)) < 1e-6 ? String(Math.round(v)) : trim(v))
  return r >= 1 ? `${near(r)}:1` : `1:${near(1 / r)}`
}

function trim(v: number): string {
  // Two significant decimals, no trailing zeros ("3.7", "317.5").
  return String(Math.round(v * 100) / 100)
}

/** The unit hint shown beside a metric preset ("1 cm = 10 cm") in the
 * user's own unit; imperial labels already carry the units. */
export function scaleHint(s: PrintScale, format: LengthFormat): string | null {
  if (LENGTH_SYSTEM_OF[format] !== 'metric') return null
  // One paper unit (1 mm or 1 cm — metres make no sense on paper) and the
  // model length it stands for, in whichever metric unit reads naturally
  // ("1 cm = 5 cm", "1 cm = 1 m", "1 mm = 2 mm").
  const paperFmt: LengthFormat = format === 'mm' ? 'mm' : 'cm'
  const paperLen = paperFmt === 'mm' ? 0.001 : 0.01
  const modelLen = paperLen / scaleRatio(s)
  // mm users stay in mm below a metre; everyone else reads cm below a metre.
  const modelFmt: LengthFormat = modelLen >= 1 ? 'm' : format === 'mm' || modelLen < 0.01 ? 'mm' : 'cm'
  return `${formatLengthIn(paperLen, paperFmt)} = ${formatLengthIn(modelLen, modelFmt)}`
}

/** Full display text: label plus the ratio when the label isn't one. */
export function scaleDisplay(s: PrintScale): string {
  const ratio = ratioText(s)
  if (s.label === ratio) return s.label
  return `${s.label} (${ratio})`
}

/**
 * Parse a custom "paper length = model length" pair in the user's unit
 * format (explicit suffixes always win: "1in", "2 ft", "1'", "25mm").
 * Returns null when either side doesn't parse, is non-positive, or the
 * ratio is outside [MIN_RATIO, MAX_RATIO].
 */
export function parseCustomScale(paperText: string, modelText: string, format: LengthFormat, opts: { unbounded?: boolean } = {}): PrintScale | null {
  const paper = parseLengthToMeters(paperText, format)
  const model = parseLengthToMeters(modelText, format)
  if (paper === null || model === null) return null
  if (!(paper > 0) || !(model > 0)) return null
  const r = paper / model
  if (!opts.unbounded && (r < MIN_RATIO || r > MAX_RATIO)) return null
  return {
    paperMeters: paper,
    modelMeters: model,
    label: `${formatLengthIn(paper, format)} = ${formatLengthIn(model, format)}`,
    kind: 'custom',
  }
}

/**
 * Fit (docs/design/printing.md §4): the EXACT ratio that fills one page to
 * the margins — a custom scale, labelled by its ratio ("1:3.27").
 * The user picks a round scale from the ladder when the job needs one.
 */
export function fitScale(fitRatio: number, _system: LengthSystem): PrintScale {
  if (!(fitRatio > 0) || !isFinite(fitRatio)) {
    return { paperMeters: 1, modelMeters: 1, label: '1:1', kind: 'fit' }
  }
  // Clamp to the same range a typed custom scale gets.
  const r = Math.min(MAX_RATIO, Math.max(MIN_RATIO, fitRatio))
  // Labelled by its ratio alone ("1:8.84") so every display reads cleanly;
  // the dialog's scale menu adds "(fit)" to its own row.
  const exact: PrintScale = { paperMeters: r, modelMeters: 1, label: '', kind: 'fit' }
  exact.label = ratioText(exact)
  return exact
}

/** Two scales are the same if their ratios agree to 1e-9 (labels aside). */
export function sameScale(a: PrintScale, b: PrintScale): boolean {
  return Math.abs(scaleRatio(a) - scaleRatio(b)) < 1e-9
}
