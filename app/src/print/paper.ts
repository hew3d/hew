/**
 * Paper sizes, margins, and orientation for printing (docs/design/printing.md
 * §3, §10). All lengths in millimetres; named sizes are the portrait
 * dimensions and `orient()` swaps them.
 */

export type PaperName = 'letter' | 'legal' | 'tabloid' | 'a5' | 'a4' | 'a3'
export type PaperSpec = PaperName | { w_mm: number; h_mm: number }
export type Orientation = 'auto' | 'portrait' | 'landscape'
export type MarginPreset = 'normal' | 'narrow'

export interface PaperSize {
  /** Portrait width, mm. */
  w: number
  /** Portrait height, mm. */
  h: number
}

/** Named sizes in portrait mm (ISO 216 / ANSI). */
export const PAPER_SIZES: Record<PaperName, PaperSize> = {
  letter: { w: 215.9, h: 279.4 },
  legal: { w: 215.9, h: 355.6 },
  tabloid: { w: 279.4, h: 431.8 },
  a5: { w: 148, h: 210 },
  a4: { w: 210, h: 297 },
  a3: { w: 297, h: 420 },
}

export const PAPER_LABEL: Record<PaperName, string> = {
  letter: 'Letter',
  legal: 'Legal',
  tabloid: 'Tabloid',
  a5: 'A5',
  a4: 'A4',
  a3: 'A3',
}

/** Menu order: the imperial trio first for imperial users, ISO first otherwise
 * — the dialog decides; this is the canonical list. */
export const PAPER_NAMES: readonly PaperName[] = ['letter', 'legal', 'tabloid', 'a5', 'a4', 'a3']

/** Uniform margin per preset (mm). Normal = ½ in, Narrow = ¼ in — both
 * comfortably beyond any desktop printer's hardware unprintable border, so
 * nothing Hew draws is ever clipped by the printer itself. */
export const MARGIN_MM: Record<MarginPreset, number> = { normal: 12.7, narrow: 6.35 }

/** Custom sizes are clamped to what the drivers and layout tolerate. */
export const CUSTOM_PAPER_MIN_MM = 50
export const CUSTOM_PAPER_MAX_MM = 2000

export function paperSize(spec: PaperSpec): PaperSize {
  if (typeof spec === 'string') return PAPER_SIZES[spec]
  const clamp = (v: number): number =>
    Math.min(CUSTOM_PAPER_MAX_MM, Math.max(CUSTOM_PAPER_MIN_MM, isFinite(v) ? v : CUSTOM_PAPER_MIN_MM))
  // Custom sizes are stored portrait (short side = width) so `orient` works
  // the same way for them.
  const a = clamp(spec.w_mm)
  const b = clamp(spec.h_mm)
  return { w: Math.min(a, b), h: Math.max(a, b) }
}

export function paperLabel(spec: PaperSpec): string {
  if (typeof spec === 'string') return PAPER_LABEL[spec]
  const p = paperSize(spec)
  return `Custom ${fmtMm(p.w)} × ${fmtMm(p.h)} mm`
}

function fmtMm(v: number): string {
  return Number.isInteger(v) ? String(v) : v.toFixed(1)
}

/** Paper dimensions in the given (resolved, non-auto) orientation. */
export function orient(size: PaperSize, orientation: 'portrait' | 'landscape'): { w: number; h: number } {
  return orientation === 'portrait' ? { w: size.w, h: size.h } : { w: size.h, h: size.w }
}

/** The named size a `{w,h}` mm pair matches (either orientation, 0.5 mm
 * tolerance), else null — used to map an OS default paper onto the menu. */
export function matchNamedPaper(w_mm: number, h_mm: number): PaperName | null {
  const lo = Math.min(w_mm, h_mm)
  const hi = Math.max(w_mm, h_mm)
  for (const name of PAPER_NAMES) {
    const p = PAPER_SIZES[name]
    if (Math.abs(p.w - lo) <= 0.5 && Math.abs(p.h - hi) <= 0.5) return name
  }
  return null
}

/** Regions whose everyday paper is Letter; everyone else defaults to A4.
 * (Same region parser as Shop Mode's unit seed — `regionFromLocale`.) */
export const LETTER_REGIONS: ReadonlySet<string> = new Set(['US', 'CA', 'MX', 'PH'])

export function defaultPaperForRegion(region: string | null): PaperName {
  return region !== null && LETTER_REGIONS.has(region) ? 'letter' : 'a4'
}

export const MM_PER_INCH = 25.4
export const PT_PER_INCH = 72

export function mmToPt(mm: number): number {
  return (mm / MM_PER_INCH) * PT_PER_INCH
}
