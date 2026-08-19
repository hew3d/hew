/**
 * Page furniture as data (docs/design/printing.md §9b; visual design in
 * docs/design/design-spec/printing SPEC §3): title block, graphic scale
 * bar, crop/trim marks, neighbor labels — a list of primitive items in
 * paper millimetres. Two renderers consume it: the DOM/SVG print page
 * (`PrintDocument.tsx`, also the dialog preview) and the PDF spec builder.
 * One source of numbers, two outputs — parity by construction. The Rust
 * mirror for headless prints is `crates/api/src/print_layout.rs`
 * (`page_furniture`); keep the two in step.
 *
 * Ink is ≥ 30 % gray everywhere (laser-safe): #111 for text and bars,
 * #1a1a1a for rules and marks, #444/#555 for secondary text, #8c8c8c for
 * the lightest table rules.
 */
import type { PrintLayout, TileSpec } from './layout'

export type FurnitureItem =
  | {
      kind: 'line'
      x1: number
      y1: number
      x2: number
      y2: number
      /** Stroke width, mm. */
      widthMm: number
      /** 0 = black … 1 = white. */
      gray: number
      /** Dash pattern in mm, if dashed. */
      dash?: number[]
      role: FurnitureRole
    }
  | {
      kind: 'text'
      /** Anchor x (per `align`) and baseline y, mm. */
      x: number
      y: number
      /** Font size, mm. */
      sizeMm: number
      text: string
      gray: number
      bold?: boolean
      align: 'left' | 'center' | 'right'
      /** Clockwise rotation about the anchor, degrees (90 = reads top to
       * bottom, glyph tops to the right — CSS `writing-mode: vertical-rl`). */
      rotate?: number
      role: FurnitureRole
    }
  | {
      kind: 'rect'
      x: number
      y: number
      w: number
      h: number
      strokeMm?: number
      fillGray?: number
      gray?: number
      role: FurnitureRole
    }

export type FurnitureRole =
  | 'crop-mark'
  | 'trim-line'
  | 'trim-caption'
  | 'neighbor-label'
  | 'title-rule'
  | 'title-doc'
  | 'title-view'
  | 'title-scale'
  | 'title-scale-sub'
  | 'title-page'
  | 'title-tile'
  | 'title-date'
  | 'title-brand'
  | 'scale-bar'
  | 'scale-bar-label'
  | 'scale-bar-caption'
  | 'table-heading'
  | 'table-subheading'
  | 'table-header'
  | 'table-rule'
  | 'table-cell'

/** Ink levels (0 = black … 1 = white). */
export const INK = {
  /** #111 — primary text, bar fills. */
  text: 0.067,
  /** #1a1a1a — rules and marks. */
  rule: 0.1,
  /** #222 — table cells. */
  cell: 0.133,
  /** #444 — secondary text. */
  secondary: 0.267,
  /** #555 — neighbor labels. */
  tertiary: 0.333,
  /** #8c8c8c — the lightest table rules (≥ 30 % gray). */
  light: 0.55,
} as const

export interface FurnitureContext {
  /** Document name for the title block ("Café table — Tabletop" when the
   * extent is a single named selection). */
  documentName: string
  /** Second line under the name: "Top view · Model", the Scene name (Pages
   * = Each Scene), "Perspective view", or "Cut list". */
  subtitle: string
  /** Scale text ("1 in = 1 ft (1:12)"); null in Standard mode. */
  scaleText: string | null
  /** Already-formatted local date. */
  dateText: string
  marks: boolean
  titleBlock: boolean
  scaleBar: boolean
  /** Global page numbering when this layout is one segment of a larger job
   * (Each Scene, appended cut list): the first page's 1-based number and
   * the job total. */
  pageNumberOffset?: number
  pageTotal?: number
}

/** Font sizes (mm) and geometry — the SPEC's numbers, named. */
export const FURNITURE = {
  titleRuleMm: 0.25,
  titleTopPadMm: 0.9,
  docNameMm: 3.2,
  subtitleMm: 2.3,
  scaleTextMm: 2.6,
  notToScaleMm: 2.8,
  subNoteMm: 2.1,
  tileIdMm: 4.6,
  pageLineMm: 2.2,
  pageLineSingleMm: 2.3,
  barHeightMm: 1.8,
  barStrokeMm: 0.25,
  barLabelMm: 2.0,
  barCaptionMm: 2.1,
  cropLenMm: 5,
  cropGapMm: 1.5,
  cropWidthMm: 0.3,
  trimWidthMm: 0.3,
  trimDash: [2, 1.5],
  trimCaptionMm: 2.3,
  neighborLabelMm: 2.6,
  neighborInsetMm: 2,
} as const

/** Baseline y for a text box whose top is `top`, at `sizeMm` (ascender ≈
 * 0.78 em for the system/Helvetica family). */
export function baseline(top: number, sizeMm: number): number {
  return top + sizeMm * 0.78
}

/** Estimated advance of "HEW" at 700 weight in ems (Helvetica-Bold
 * H+E+W = 2.333 em) — right-anchors the brand mark beside its line. */
const BRAND_EM = 2.333

/** Content of the title block, already resolved to strings. */
export interface TitleBlockContent {
  documentName: string
  subtitle: string
  /** Centre slot: main line + optional sub-line, and the scale bar or not. */
  centerMain: string | null
  centerSub: string | null
  showScaleBar: boolean
  /** Big tile id at the right end (tiled Scaled prints only). */
  tileId: string | null
  /** "Page 5 of 6 · Tile B2" / "Page 1 of 1". */
  pageText: string
  dateText: string
  /** Horizontal inset of the strip's content at both ends, mm — room for
   * the bottom crop marks, which run down into the strip at its ends. */
  insetMm?: number
}

/** The title block strip: rule, three columns, brand mark. */
export function titleBlockItems(layout: PrintLayout, content: TitleBlockContent): FurnitureItem[] {
  const tb = layout.page.titleBlock
  if (tb === null) return []
  const F = FURNITURE
  const items: FurnitureItem[] = []
  const top = tb.y + F.titleTopPadMm
  const inset = content.insetMm ?? 0
  const left = tb.x + inset
  const right = tb.x + tb.w - inset
  items.push({ kind: 'line', x1: tb.x, y1: tb.y, x2: tb.x + tb.w, y2: tb.y, widthMm: F.titleRuleMm, gray: INK.rule, role: 'title-rule' })
  // Left: name over subtitle.
  items.push(text(left, baseline(top, F.docNameMm), content.documentName, F.docNameMm, 'left', 'title-doc', INK.text, true))
  items.push(text(left, baseline(top + F.docNameMm * 1.2 + 0.8, F.subtitleMm), content.subtitle, F.subtitleMm, 'left', 'title-view', INK.secondary))
  // Centre: scale (+ graphic bar) or the cut-list count; nothing on a Standard page.
  const cx = tb.x + tb.w / 2
  const sb = layout.scaleBar
  if (content.showScaleBar && sb !== null && content.centerMain !== null) {
    items.push(text(cx, baseline(top, F.scaleTextMm), content.centerMain, F.scaleTextMm, 'center', 'title-scale', INK.text, true))
    // Bar block: labels row (2.0) then the bar 2.7 below the block's top;
    // bar + caption centred as a group under the scale text. Text widths
    // are estimated (0.5 em per character) — the two renderers use the same
    // estimate, so they agree; the bar's own position is exact by
    // construction either way.
    // The bar's paper length, for the ruler check — the labels above the
    // ticks already say what it means in the model.
    const caption = sb.paperLabel
    const captionW = caption.length * 0.5 * F.barCaptionMm
    const lastLabelW = sb.labels[4].length * 0.5 * F.barLabelMm
    const gap = Math.max(2.5, lastLabelW / 2 + 1)
    const blockTop = top + F.scaleTextMm * 1.15 + 0.5
    const x0 = cx - (sb.paperMm + gap + captionW) / 2
    const labelBase = baseline(blockTop, F.barLabelMm)
    for (let i = 0; i <= 4; i++) {
      items.push(text(x0 + i * sb.segmentMm, labelBase, sb.labels[i], F.barLabelMm, 'center', 'scale-bar-label', INK.text))
    }
    const barY = blockTop + 2.7
    for (let i = 0; i < 4; i += 2) {
      items.push({ kind: 'rect', x: x0 + i * sb.segmentMm, y: barY, w: sb.segmentMm, h: F.barHeightMm, fillGray: INK.text, role: 'scale-bar' })
    }
    items.push({ kind: 'rect', x: x0, y: barY, w: sb.paperMm, h: F.barHeightMm, strokeMm: F.barStrokeMm, gray: INK.text, role: 'scale-bar' })
    items.push(text(x0 + sb.paperMm + gap, barY + F.barHeightMm - 0.2, caption, F.barCaptionMm, 'left', 'scale-bar-caption', INK.secondary))
  } else if (content.centerMain !== null) {
    const mainSize = content.centerSub !== null ? F.notToScaleMm : F.scaleTextMm
    items.push(text(cx, baseline(top, mainSize), content.centerMain, mainSize, 'center', 'title-scale', INK.text, true))
    if (content.centerSub !== null) {
      items.push(text(cx, baseline(top + mainSize * 1.2 + 0.8, F.subNoteMm), content.centerSub, F.subNoteMm, 'center', 'title-scale-sub', INK.secondary))
    }
  }
  // Right: tile id over the page line, or page over date; "HEW" in bold,
  // the rest of its line ("Page 5 of 6 · Tile B2 · 17 Aug 2026 ·") anchored
  // just left of it. An empty date (headless) simply drops out.
  const brandW = BRAND_EM * F.pageLineMm
  const dated = (parts: string[]): string => [...parts, ...(content.dateText !== '' ? [content.dateText] : [])].join(' · ')
  if (content.tileId !== null) {
    items.push(text(right, top + F.tileIdMm * 0.78, content.tileId, F.tileIdMm, 'right', 'title-tile', INK.text, true))
    const lineBase = baseline(top + F.tileIdMm + 0.9, F.pageLineMm)
    items.push(text(right, lineBase, 'HEW', F.pageLineMm, 'right', 'title-brand', INK.rule, true))
    items.push(text(right - brandW - 0.6, lineBase, `${dated([content.pageText])} ·`, F.pageLineMm, 'right', 'title-page', INK.secondary))
  } else {
    items.push(text(right, baseline(top, F.pageLineSingleMm), content.pageText, F.pageLineSingleMm, 'right', 'title-page', INK.secondary))
    const lineBase = baseline(top + F.pageLineSingleMm * 1.2 + 0.8, F.pageLineMm)
    items.push(text(right, lineBase, 'HEW', F.pageLineMm, 'right', 'title-brand', INK.rule, true))
    if (content.dateText !== '') items.push(text(right - brandW - 0.6, lineBase, `${content.dateText} ·`, F.pageLineMm, 'right', 'title-date', INK.secondary))
  }
  return items
}

/** Everything drawn on one page besides the drawing itself. */
export function pageFurniture(layout: PrintLayout, tile: TileSpec, ctx: FurnitureContext): FurnitureItem[] {
  const items: FurnitureItem[] = []
  const F = FURNITURE
  const d = layout.page.drawing
  const total = ctx.pageTotal ?? layout.tiles.length
  const pageNumber = (ctx.pageNumberOffset ?? 1) + tile.page
  const tiled = layout.tiles.length > 1
  const tb = layout.page.titleBlock
  const titleOn = ctx.titleBlock && tb !== null

  // Marks are a Scaled-mode thing: a single scaled sheet gets corner crop
  // marks (a squareness check); Standard is a picture and gets none.
  if (ctx.marks && layout.mode === 'scaled') {
    const ov = layout.overlapMm
    const X2 = d.x + d.w
    const Y2 = d.y + d.h
    const bandR = tile.overlapRight ? ov : 0
    const bandB = tile.overlapBottom ? ov : 0
    // Crop marks at the drawing-area corners: 5 × 0.3 bars, 1.5 gap; a bar
    // is skipped where it would run into an overlap band. The bottom
    // vertical bars run down into the title-block strip's ends (its
    // content is inset to leave them room) so a corner always shows both.
    // Bars are clipped to the sheet: with Narrow margins the 6.5 mm reach
    // (gap + length) exceeds the 6.35 mm margin by a hair.
    const pw = layout.page.paper.w
    const ph = layout.page.paper.h
    const hbar = (x: number, y: number): void => {
      const x0 = Math.max(0, x)
      const x1 = Math.min(pw, x + F.cropLenMm)
      if (x1 - x0 > 0.5) items.push({ kind: 'rect', x: x0, y: y - F.cropWidthMm / 2, w: x1 - x0, h: F.cropWidthMm, fillGray: INK.rule, role: 'crop-mark' })
    }
    const vbar = (x: number, y: number): void => {
      const y0 = Math.max(0, y)
      const y1 = Math.min(ph, y + F.cropLenMm)
      if (y1 - y0 > 0.5) items.push({ kind: 'rect', x: x - F.cropWidthMm / 2, y: y0, w: F.cropWidthMm, h: y1 - y0, fillGray: INK.rule, role: 'crop-mark' })
    }
    const outL = d.x - F.cropGapMm - F.cropLenMm
    const outT = d.y - F.cropGapMm - F.cropLenMm
    const outR = X2 + F.cropGapMm
    const outB = Y2 + F.cropGapMm
    hbar(outL, d.y)
    vbar(d.x, outT)
    vbar(X2, outT)
    if (!tile.overlapRight) hbar(outR, d.y)
    hbar(outL, Y2)
    if (!tile.overlapBottom) vbar(d.x, outB)
    if (!tile.overlapRight) hbar(outR, Y2)
    // With a right band the dashed trim line already defines that edge, and
    // a bar there would land in the title strip's page line.
    if (!tile.overlapBottom && !tile.overlapRight) vbar(X2, outB)
    // Trim lines at the drawing edge on sides with a neighbour, spanning the
    // drawing plus the band; "✂ trim" at each line's start.
    const n = tile.neighbors
    if (n.right !== undefined) {
      items.push({ kind: 'line', x1: X2, y1: d.y, x2: X2, y2: Y2 + bandB, widthMm: F.trimWidthMm, gray: INK.rule, dash: [...F.trimDash], role: 'trim-line' })
      // Rotated 90° the baseline runs down x = anchor and glyph tops point
      // right, so the run's box starts ~0.25 em left of the anchor.
      items.push({ ...text(X2 + 0.8 + F.trimCaptionMm * 0.25, d.y + 1.3, '✂ trim', F.trimCaptionMm, 'left', 'trim-caption', INK.secondary), rotate: 90 })
    }
    if (n.down !== undefined) {
      items.push({ kind: 'line', x1: d.x, y1: Y2, x2: X2 + bandR, y2: Y2, widthMm: F.trimWidthMm, gray: INK.rule, dash: [...F.trimDash], role: 'trim-line' })
      items.push(text(d.x + 1.3, baseline(Y2 + 0.8, F.trimCaptionMm), '✂ trim', F.trimCaptionMm, 'left', 'trim-caption', INK.secondary))
    }
    // Neighbour labels 2 mm inside the drawing edge, mid-edge — they survive
    // trimming the band.
    const ls = F.neighborLabelMm
    const midY = d.y + d.h / 2 + ls * 0.35
    const midX = d.x + d.w / 2
    if (n.right !== undefined) items.push(text(X2 - F.neighborInsetMm, midY, `→ ${n.right}`, ls, 'right', 'neighbor-label', INK.tertiary))
    if (n.left !== undefined) items.push(text(d.x + F.neighborInsetMm, midY, `← ${n.left}`, ls, 'left', 'neighbor-label', INK.tertiary))
    if (n.up !== undefined) items.push(text(midX, baseline(d.y + F.neighborInsetMm - 0.7, ls), `↑ ${n.up}`, ls, 'center', 'neighbor-label', INK.tertiary))
    if (n.down !== undefined) items.push(text(midX, Y2 - F.neighborInsetMm - 0.3, `↓ ${n.down}`, ls, 'center', 'neighbor-label', INK.tertiary))
  }

  if (titleOn) {
    const pageText = tiled ? `Page ${pageNumber} of ${total} · Tile ${tile.id}` : total > 1 ? `Page ${pageNumber} of ${total}` : 'Page 1 of 1'
    const scaled = layout.mode === 'scaled'
    items.push(
      ...titleBlockItems(layout, {
        documentName: ctx.documentName,
        subtitle: ctx.subtitle,
        // Standard pages have no scale and say nothing about it.
        centerMain: scaled ? ctx.scaleText : null,
        centerSub: null,
        showScaleBar: scaled && ctx.scaleBar,
        tileId: scaled && tiled ? tile.id : null,
        pageText,
        dateText: ctx.dateText,
        insetMm: ctx.marks && scaled ? F.cropWidthMm + 1.5 : 0,
      }),
    )
  }

  return items
}

function text(
  x: number,
  y: number,
  t: string,
  sizeMm: number,
  align: 'left' | 'center' | 'right',
  role: FurnitureRole,
  gray: number,
  bold = false,
): Extract<FurnitureItem, { kind: 'text' }> {
  return { kind: 'text', x, y, sizeMm, text: t, gray, align, role, ...(bold ? { bold: true } : {}) }
}

/** A plain text item for other furniture builders (the cut list). */
export const textItem = text
