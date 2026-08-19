/**
 * Print layout engine (docs/design/printing.md §6) — pure geometry, no DOM,
 * no three.js. Given paper/margins/mode/scale/extent it decides page
 * geometry and the tile grid, px-exactly: image pixel sizes are rounded
 * first and every mm/meter figure is derived from the rounded pixel count,
 * so the bitmap, its CSS size, and the camera frustum agree to the pixel.
 *
 * Coordinate frames:
 *  - Paper: mm, origin top-left of the ORIENTED sheet, y down.
 *  - View plane: meters, origin at the print camera's centre, x right, y UP
 *    (the orthographic camera's own frustum frame — `modelRect` maps
 *    directly onto `left/right/top/bottom`).
 */
import { MM_PER_INCH, orient, type PaperSize } from './paper'

export type PrintMode = 'standard' | 'scaled'

export interface RectMm {
  x: number
  y: number
  w: number
  h: number
}

/** A rectangle in the view plane, meters, y up. */
export interface RectM {
  x: number
  y: number
  w: number
  h: number
}

export interface PageGeometry {
  orientation: 'portrait' | 'landscape'
  /** Oriented paper size, mm. */
  paper: { w: number; h: number }
  marginMm: number
  /** Drawing area (excludes overlap bands and the title block), mm. */
  drawing: RectMm
  /** Title block strip, when on. */
  titleBlock: RectMm | null
}

export interface TileSpec {
  /** "A1", "B3" — row letter, column number. */
  id: string
  row: number
  col: number
  /** Global page index (0-based) across all tile sets. */
  page: number
  /** Where the image sits on the page (drawing area + overlap bands), mm. */
  imageRectMm: RectMm
  /** Bitmap size, whole pixels at `dpi`. */
  imagePx: { w: number; h: number }
  /** View-plane rectangle the image shows (Scaled only; meters, y up). */
  modelRect: RectM | null
  overlapRight: boolean
  overlapBottom: boolean
  neighbors: { left?: string; right?: string; up?: string; down?: string }
}

/**
 * A graphic (architectural) scale: four alternating black/white segments
 * of `segmentMm` each, tick labels in round MODEL units at every boundary
 * ("0 5 10 15 20 cm"), plus the bar's own paper length for the ruler check.
 */
export interface ScaleBarSpec {
  /** Total bar length on paper, mm (= 4 × segmentMm). */
  paperMm: number
  /** One segment on paper, mm. */
  segmentMm: number
  /** Model length of one segment, meters. */
  segmentMeters: number
  /** Five tick labels, 0 … 4 segments; the unit rides on the last one. */
  labels: string[]
  /** The bar's own paper length: "40 mm" or "2 in". */
  paperLabel: string
}

export interface PrintLayout {
  mode: PrintMode
  page: PageGeometry
  dpi: number
  tiles: TileSpec[]
  rows: number
  cols: number
  /** Standard mode: the letterboxed image; Scaled: the extent used. */
  extentM: RectM | null
  scaleBar: ScaleBarSpec | null
  /** Effective nudge after clamping, mm. */
  nudgeMm: { dx: number; dy: number }
  /** The overlap band actually reserved (mm): the requested band on a
   * tiled layout, 0 on a single page (nothing to glue). */
  overlapMm: number
}

export interface LayoutInput {
  mode: PrintMode
  paper: PaperSize
  orientation: 'auto' | 'portrait' | 'landscape'
  marginMm: number
  /** Title block height, mm; 0 when off. */
  titleBlockMm: number
  /** Overlap band, mm; 0 when off (Scaled only). */
  overlapMm: number
  dpi: number
  /** Standard: viewport aspect (w/h) to letterbox. */
  viewportAspect?: number
  /** Scaled: paper/model ratio (0.1 = 1:10). */
  ratio?: number
  /** Scaled: what to print, view-plane meters (y up). */
  extent?: RectM
  /** Scaled: tile-grid offset, mm (clamped; see `applyNudge`). */
  nudgeMm?: { dx: number; dy: number }
  /** Scale-bar unit family; null = no scale bar. */
  scaleBarSystem?: 'metric' | 'imperial' | null
}

/** Default bitmap resolution; §7 (D11). */
export const PRINT_DPI = 300
/** Preview resolution used by the dialog. */
export const PREVIEW_DPI = 40
/** Per-page pixel budget before dpi is stepped down (§7 "Limits"). */
export const MAX_PAGE_PIXELS = 24_000_000
/** Never below this even for a huge sheet. */
export const MIN_DPI = 150

/** Overlap band per margin preset (§6, design decision #12): 10 mm with
 * Normal margins, 5 mm with Narrow. */
export const OVERLAP_MM = { normal: 10, narrow: 5 } as const
export const DEFAULT_OVERLAP_MM = OVERLAP_MM.normal
export const DEFAULT_TITLE_BLOCK_MM = 10
/** Above this the summary turns into a warning (§4). */
export const MANY_PAGES_WARNING = 40

export function mmToPx(mm: number, dpi: number): number {
  return Math.round((mm / MM_PER_INCH) * dpi)
}

export function pxToMm(px: number, dpi: number): number {
  return (px / dpi) * MM_PER_INCH
}

/** Row letter(s): 0 → A, 25 → Z, 26 → AA. */
export function rowLetter(row: number): string {
  let s = ''
  let n = row
  do {
    s = String.fromCharCode(65 + (n % 26)) + s
    n = Math.floor(n / 26) - 1
  } while (n >= 0)
  return s
}

export function tileId(row: number, col: number): string {
  return `${rowLetter(row)}${col + 1}`
}

/** The dpi to use for a page of `w × h` mm: the requested dpi, stepped down
 * (never below MIN_DPI) until the page fits the pixel budget and the
 * renderer's max side. */
export function clampDpi(requestedDpi: number, w_mm: number, h_mm: number, maxSidePx = 16384): number {
  let dpi = requestedDpi
  const fits = (d: number): boolean => {
    const w = mmToPx(w_mm, d)
    const h = mmToPx(h_mm, d)
    return w * h <= MAX_PAGE_PIXELS && w <= maxSidePx && h <= maxSidePx
  }
  while (dpi > MIN_DPI && !fits(dpi)) dpi = Math.max(MIN_DPI, dpi - 50)
  return dpi
}

/**
 * Page geometry. The drawing area is the printable area minus the title
 * block, minus — on a tiled layout — the overlap band, which is reserved
 * INSIDE the printable area on the right and bottom (design decision #12:
 * past the trim line would put the band in the printer's unprintable
 * border). Uniform for every tile; the last column/row leave it empty.
 * `drawing = paper − 2·margin − titleBlock − overlap`.
 */
function pageGeometry(
  paper: PaperSize,
  orientation: 'portrait' | 'landscape',
  marginMm: number,
  titleBlockMm: number,
  overlapMm = 0,
): PageGeometry {
  const o = orient(paper, orientation)
  const inner = { x: marginMm, y: marginMm, w: o.w - 2 * marginMm, h: o.h - 2 * marginMm }
  const titleBlock: RectMm | null =
    titleBlockMm > 0 ? { x: inner.x, y: inner.y + inner.h - titleBlockMm, w: inner.w, h: titleBlockMm } : null
  const drawing: RectMm = {
    x: inner.x,
    y: inner.y,
    w: inner.w - overlapMm,
    h: inner.h - (titleBlock !== null ? titleBlockMm : 0) - overlapMm,
  }
  return { orientation, paper: o, marginMm, drawing, titleBlock }
}

/** Number of tiles along one axis for an extent of `extentMm` on a drawing
 * step of `stepMm`; at least 1. */
/** Hard ceiling on tiles per axis: past this the request is nonsense (a
 * zero drawing area, a scale off by orders of magnitude) and must not
 * allocate. */
export const MAX_TILES_PER_AXIS = 500

function tileCount(extentMm: number, stepMm: number): number {
  if (!(stepMm > 0) || !isFinite(extentMm)) return 1
  return Math.min(MAX_TILES_PER_AXIS, Math.max(1, Math.ceil(extentMm / stepMm - 1e-9)))
}

interface AxisGrid {
  count: number
  /** Extent's start inside the grid (mm from the grid's start). */
  start: number
  /** Effective nudge on this axis after clamping. */
  nudge: number
}

/**
 * Place an extent of `extentMm` on a grid of `stepMm` tiles, centred, then
 * shifted by `nudgeMm` — clamped so the extent stays on the same tiles: the
 * count never changes and no page is ever empty (a seam can move within
 * the slack; past that the model would need another sheet).
 */
function axisGrid(extentMm: number, stepMm: number, nudgeMm: number): AxisGrid {
  if (!(stepMm > 0) || !isFinite(extentMm)) return { count: 1, start: 0, nudge: 0 }
  const count = tileCount(extentMm, stepMm)
  const slack = Math.max(0, count * stepMm - extentMm)
  const maxNudge = slack / 2
  const nudge = Math.max(-maxNudge, Math.min(maxNudge, isFinite(nudgeMm) ? nudgeMm : 0))
  return { count, start: slack / 2 + nudge, nudge }
}

function trimNum(v: number, decimals: number): string {
  const s = v.toFixed(decimals)
  return s.includes('.') ? s.replace(/0+$/, '').replace(/\.$/, '') : s
}

const INCH_FRACTIONS: Record<number, string> = { 0.25: '¼', 0.5: '½', 0.75: '¾' }

/** Inches as "1", "¼", "1½", "2¾" (quarter steps only). */
function inchLabel(inches: number): string {
  const whole = Math.floor(inches + 1e-9)
  const frac = Math.round((inches - whole) * 4) / 4
  const f = INCH_FRACTIONS[frac]
  if (f === undefined) return trimNum(inches, 2)
  return whole === 0 ? f : `${whole}${f}`
}

/**
 * The graphic scale bar for a drawing `drawingWmm` wide at `ratio` (§9b,
 * design §3): the largest ROUND model step — {1, 2, 5} × 10ᵏ mm, or
 * ¼″ ½″ 1″ 2″ 3″ 6″ 1′ 2′ 3′ 6′ 10′ — whose paper length p = step × ratio
 * is ≥ 6 mm with 4·p ≤ min(40 % of the drawing width, 80 mm). Falls back to
 * the largest step that fits at all, then to the smallest step.
 */
export function pickScaleBar(drawingWmm: number, ratio: number, system: 'metric' | 'imperial'): ScaleBarSpec {
  const limit = Math.min(drawingWmm * 0.4, 80)
  // Candidate steps in mm, ascending.
  const stepsMm: number[] = []
  if (system === 'metric') {
    for (let k = -1; k <= 4; k++) for (const m of [1, 2, 5]) stepsMm.push(m * 10 ** k)
  } else {
    for (const inch of [0.25, 0.5, 1, 2, 3, 6, 12, 24, 36, 72, 120]) stepsMm.push(inch * MM_PER_INCH)
  }
  const paperOf = (step: number): number => step * ratio
  let step = [...stepsMm].reverse().find((st) => paperOf(st) >= 6 && 4 * paperOf(st) <= limit)
  if (step === undefined) step = [...stepsMm].reverse().find((st) => 4 * paperOf(st) <= limit)
  if (step === undefined) step = stepsMm[0]
  const segmentMm = paperOf(step)
  const labels: string[] = []
  if (system === 'metric') {
    const [div, unit] = step >= 1000 ? [1000, 'm'] : step >= 10 ? [10, 'cm'] : [1, 'mm']
    for (let i = 0; i <= 4; i++) labels.push(trimNum((i * step) / div, 3))
    labels[4] = `${labels[4]} ${unit}`
  } else {
    const inches = step / MM_PER_INCH
    if (inches >= 12) {
      for (let i = 0; i <= 4; i++) labels.push(trimNum((i * inches) / 12, 2))
      labels[4] = `${labels[4]} ft`
    } else {
      for (let i = 0; i <= 4; i++) labels.push(inchLabel(i * inches))
      labels[4] = `${labels[4]} in`
    }
  }
  const paperMm = 4 * segmentMm
  const paperLabel = system === 'metric' ? `${trimNum(paperMm, 1)} mm` : `${trimNum(paperMm / MM_PER_INCH, 2)} in`
  return { paperMm, segmentMm, segmentMeters: step / 1000, labels, paperLabel }
}

function layoutFor(input: LayoutInput, orientation: 'portrait' | 'landscape'): PrintLayout {
  let page = pageGeometry(input.paper, orientation, input.marginMm, input.titleBlockMm)
  const dpi = clampDpi(input.dpi, page.paper.w, page.paper.h)

  if (input.mode === 'standard') {
    const d = page.drawing
    const aspect = input.viewportAspect !== undefined && input.viewportAspect > 0 ? input.viewportAspect : 4 / 3
    // Letterbox: fit the viewport aspect inside the drawing area, px-exact.
    let wPx = mmToPx(d.w, dpi)
    let hPx = Math.round(wPx / aspect)
    if (pxToMm(hPx, dpi) > d.h) {
      hPx = mmToPx(d.h, dpi)
      wPx = Math.round(hPx * aspect)
    }
    const wMm = pxToMm(wPx, dpi)
    const hMm = pxToMm(hPx, dpi)
    const tile: TileSpec = {
      id: 'A1',
      row: 0,
      col: 0,
      page: 0,
      imageRectMm: { x: d.x + (d.w - wMm) / 2, y: d.y + (d.h - hMm) / 2, w: wMm, h: hMm },
      imagePx: { w: wPx, h: hPx },
      modelRect: null,
      overlapRight: false,
      overlapBottom: false,
      neighbors: {},
    }
    return { mode: 'standard', page, dpi, tiles: [tile], rows: 1, cols: 1, extentM: null, scaleBar: null, nudgeMm: { dx: 0, dy: 0 }, overlapMm: 0 }
  }

  const ratio = input.ratio ?? 1
  const extent = input.extent ?? { x: -0.5, y: -0.5, w: 1, h: 1 }
  const mmPerM = ratio * 1000
  const extentMm = { w: Math.max(extent.w * mmPerM, 1e-6), h: Math.max(extent.h * mmPerM, 1e-6) }
  // First on the full drawing area: one page needs no band. Only a tiled
  // print reserves the overlap inside the printable area (decision #12) —
  // and then every tile steps by the reduced drawing area, uniformly.
  const requested = Math.max(0, input.overlapMm)
  let gx = axisGrid(extentMm.w, page.drawing.w, input.nudgeMm?.dx ?? 0)
  let gy = axisGrid(extentMm.h, page.drawing.h, input.nudgeMm?.dy ?? 0)
  let overlap = 0
  if (requested > 0 && (gx.count > 1 || gy.count > 1)) {
    // Never eat more than half the drawing area (a tiny custom sheet).
    overlap = Math.min(requested, Math.max(0, Math.min(page.drawing.w, page.drawing.h) / 2))
    page = pageGeometry(input.paper, orientation, input.marginMm, input.titleBlockMm, overlap)
    gx = axisGrid(extentMm.w, page.drawing.w, input.nudgeMm?.dx ?? 0)
    gy = axisGrid(extentMm.h, page.drawing.h, input.nudgeMm?.dy ?? 0)
  }
  const d = page.drawing

  const tiles: TileSpec[] = []
  let pageIdx = 0
  for (let r = 0; r < gy.count; r++) {
    for (let c = 0; c < gx.count; c++) {
      const overlapRight = overlap > 0 && c < gx.count - 1
      const overlapBottom = overlap > 0 && r < gy.count - 1
      // The image covers the drawing area plus the band toward a neighbour;
      // the last column/row leave their band empty.
      const bandR = overlapRight ? overlap : 0
      const bandB = overlapBottom ? overlap : 0
      const wPx = mmToPx(d.w + bandR, dpi)
      const hPx = mmToPx(d.h + bandB, dpi)
      const wMm = pxToMm(wPx, dpi)
      const hMm = pxToMm(hPx, dpi)
      // Grid coordinates (mm) of this tile's top-left, relative to the grid.
      const gridX = c * d.w
      const gridY = r * d.h
      // View plane: grid x → model x; grid y (down) → model y (up).
      const modelX = extent.x + (gridX - gx.start) / mmPerM
      const modelTop = extent.y + extent.h - (gridY - gy.start) / mmPerM
      const modelW = wMm / mmPerM
      const modelH = hMm / mmPerM
      tiles.push({
        id: tileId(r, c),
        row: r,
        col: c,
        page: pageIdx++,
        imageRectMm: { x: d.x, y: d.y, w: wMm, h: hMm },
        imagePx: { w: wPx, h: hPx },
        modelRect: { x: modelX, y: modelTop - modelH, w: modelW, h: modelH },
        overlapRight,
        overlapBottom,
        neighbors: {
          left: c > 0 ? tileId(r, c - 1) : undefined,
          right: c < gx.count - 1 ? tileId(r, c + 1) : undefined,
          up: r > 0 ? tileId(r - 1, c) : undefined,
          down: r < gy.count - 1 ? tileId(r + 1, c) : undefined,
        },
      })
    }
  }
  const scaleBar =
    input.scaleBarSystem !== undefined && input.scaleBarSystem !== null ? pickScaleBar(d.w, ratio, input.scaleBarSystem) : null
  return {
    mode: 'scaled',
    page,
    dpi,
    tiles,
    rows: gy.count,
    cols: gx.count,
    extentM: extent,
    scaleBar,
    nudgeMm: { dx: gx.nudge, dy: gy.nudge },
    overlapMm: overlap,
  }
}

/**
 * Lay out one print job. Orientation `auto` picks landscape for a Standard
 * viewport wider than tall, and for Scaled whichever orientation needs
 * fewer tiles (ties → portrait).
 */
export function layoutPrint(input: LayoutInput): PrintLayout {
  if (input.orientation !== 'auto') return layoutFor(input, input.orientation)
  if (input.mode === 'standard') {
    const aspect = input.viewportAspect ?? 4 / 3
    return layoutFor(input, aspect > 1 ? 'landscape' : 'portrait')
  }
  const p = layoutFor(input, 'portrait')
  const l = layoutFor(input, 'landscape')
  return l.tiles.length < p.tiles.length ? l : p
}

/**
 * The exact ratio at which `extent` fills one page's drawing area in the
 * best orientation (input to `fitScale`). Ignores overlap (a one-page print
 * has none).
 */
export function fitRatioFor(input: Omit<LayoutInput, 'ratio' | 'mode' | 'nudgeMm'> & { extent: RectM }): number {
  const orientations: ('portrait' | 'landscape')[] =
    input.orientation === 'auto' ? ['portrait', 'landscape'] : [input.orientation]
  let best = 0
  for (const o of orientations) {
    const page = pageGeometry(input.paper, o, input.marginMm, input.titleBlockMm)
    const r = Math.min(page.drawing.w / 1000 / Math.max(input.extent.w, 1e-9), page.drawing.h / 1000 / Math.max(input.extent.h, 1e-9))
    best = Math.max(best, r)
  }
  return best
}

/** Page count summary text used by the dialog and tests: "6 pages (3 × 2)". */
export function pageCountText(layout: PrintLayout): string {
  const n = layout.tiles.length
  if (n === 1) return '1 page'
  return `${n} pages (${layout.cols} × ${layout.rows})`
}
