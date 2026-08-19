/**
 * Cut-list page(s) (docs/design/printing.md §16): every whole PART of the
 * document once — objects and component instances, the leaves; a group is
 * a container whose members are the parts — with its overall L × W × H in
 * the user's units, identical parts folded into a Qty, laid out as one
 * flowing table on furniture-only pages appended once per job. A part is
 * listed once however many tags it carries (the Shop Mode parts sheet
 * repeats a part under each of its tags; a printed list must not). Same
 * item model as the title block, so it prints, previews, and lands in the
 * PDF identically.
 */
import type { Scene as WasmScene } from '../wasm/loader'
import { buildPartsSheetSections } from '../shop/partsSheetModel'
import { formatLengthIn, type LengthFormat } from '../settings/units'
import { nodeKey } from '../panels/treeModel'
import { boundsExtents, instanceLocalBounds, instancePoseScale } from '../panels/objectBounds'
import { INK, baseline, textItem, titleBlockItems, type FurnitureItem } from './furniture'
import type { PrintLayout, TileSpec } from './layout'

export interface CutListRow {
  label: string
  /** How many identical parts (same name and size) this row folds. */
  qty: number
  /** Formatted L, W, H in the user's units, unit shown ("440 mm", "1' 6"") —
   * as the parts sheet shows them. */
  l: string
  w: string
  h: string
}

/** Rows from the live scene: each leaf part once, in parts-sheet order,
 * identical parts folded into a Qty. Every instance of one component
 * definition is the same part however it is turned or placed — those fold
 * on the definition (and the instance's own scale, if it was rescaled),
 * with the part's own unrotated L × W × H. */
export function cutListRows(scene: WasmScene, format: LengthFormat): CutListRow[] {
  const sections = buildPartsSheetSections(scene, new Set(), new Set())
  const rows: CutListRow[] = []
  const seen = new Set<string>()
  const index = new Map<string, CutListRow>()
  const fmt = (v: number): string => formatLengthIn(v, format)
  for (const s of sections) {
    for (const r of s.rows) {
      if (r.node.kind === 'group') continue
      const key = nodeKey(r.node)
      if (seen.has(key)) continue
      seen.add(key)
      let extents = r.extentsM
      let fold: string
      if (r.node.kind === 'instance') {
        const def = scene.instance_def(r.node.id)
        const local = instanceLocalBounds(scene, r.node.id)
        if (local !== null) extents = boundsExtents(local)
        // Same definition AND same own scale = the same part; a placement
        // rescaled on its own is a different part with its real size.
        const pose = scene.instance_pose(r.node.id)
        const scaleKey = pose === undefined ? '' : instancePoseScale(pose).map((v) => v.toFixed(4)).join('×')
        fold = def === undefined ? `node ${key}` : `def ${def} @ ${scaleKey}`
      } else {
        const dims0 = extents === null ? ['—', '—', '—'] : [fmt(extents[0]), fmt(extents[1]), fmt(extents[2])]
        fold = `${r.label} ${dims0.join(' ')}`
      }
      const dims = extents === null ? ['—', '—', '—'] : [fmt(extents[0]), fmt(extents[1]), fmt(extents[2])]
      const found = index.get(fold)
      if (found !== undefined) {
        found.qty += 1
        continue
      }
      const row: CutListRow = { label: r.label, qty: 1, l: dims[0], w: dims[1], h: dims[2] }
      index.set(fold, row)
      rows.push(row)
    }
  }
  return rows
}

/** Table geometry (mm). */
export const CUT_LIST = {
  headingMm: 4.2,
  subheadingMm: 2.4,
  /** Heading top, below the drawing-area top. */
  headingTopMm: 7.3,
  subheadingTopMm: 13.7,
  tableTopMm: 25.3,
  headerTextMm: 2.8,
  headerPadMm: 1.6,
  headerRuleMm: 0.3,
  rowMm: 8.5,
  rowRuleMm: 0.15,
  cellMm: 2.8,
  /** Column x-origins from the drawing-area left edge: Part · Qty · L · W ·
   * H (Letter); narrower sheets shrink the Part column. */
  columns: [0, 82, 104, 134, 164] as const,
} as const

/** Column x-origins for a drawing area `w` wide: the Letter numbers, with
 * the Part column giving way on narrower sheets. */
export function cutListColumns(w: number): number[] {
  const c = CUT_LIST.columns
  const spare = w - c[4]
  // Keep ≥ 26 mm for the H column; shrink the Part column first.
  const shift = spare >= 26 ? 0 : 26 - spare
  return [0, c[1] - shift, c[2] - shift, c[3] - shift, c[4] - shift]
}

/** How many rows fit one page's drawing area. */
export function cutListRowsPerPage(layout: PrintLayout): number {
  const d = layout.page.drawing
  const headerBottom = CUT_LIST.tableTopMm + CUT_LIST.headerTextMm + CUT_LIST.headerPadMm + CUT_LIST.headerRuleMm
  return Math.max(1, Math.floor((d.h - headerBottom) / CUT_LIST.rowMm))
}

/**
 * Furniture for cut-list page `pageIndex` (0-based among cut-list pages),
 * given all rows. Continuation pages repeat the heading and header row.
 */
export function cutListPageFurniture(
  layout: PrintLayout,
  rows: CutListRow[],
  pageIndex: number,
  ctx: { documentName: string; pageNumber: number; totalPages: number; dateText: string; titleBlock?: boolean },
): FurnitureItem[] {
  const d = layout.page.drawing
  const C = CUT_LIST
  const per = cutListRowsPerPage(layout)
  const slice = rows.slice(pageIndex * per, (pageIndex + 1) * per)
  const items: FurnitureItem[] = []
  const cols = cutListColumns(d.w).map((x) => d.x + x)
  const pieces = rows.reduce((a, r) => a + r.qty, 0)

  items.push(textItem(d.x, baseline(d.y + C.headingTopMm, C.headingMm), 'Cut list', C.headingMm, 'left', 'table-heading', INK.text, true))
  items.push(textItem(d.x, baseline(d.y + C.subheadingTopMm, C.subheadingMm), ctx.documentName, C.subheadingMm, 'left', 'table-subheading', INK.secondary))
  // Header row: text, then a 0.3 rule under it.
  const headerBase = baseline(d.y + C.tableTopMm, C.headerTextMm)
  const headers = ['Part', 'Qty', 'L', 'W', 'H']
  headers.forEach((h, i) => items.push(textItem(cols[i], headerBase, h, C.headerTextMm, 'left', 'table-header', INK.text, true)))
  const headerRuleY = d.y + C.tableTopMm + C.headerTextMm + C.headerPadMm + C.headerRuleMm / 2
  items.push({ kind: 'line', x1: d.x, y1: headerRuleY, x2: d.x + d.w, y2: headerRuleY, widthMm: C.headerRuleMm, gray: INK.rule, role: 'table-rule' })
  // Rows: 8.5 mm each, text vertically centred, 0.15 rule under each.
  const rowsTop = headerRuleY + C.headerRuleMm / 2
  slice.forEach((r, i) => {
    const top = rowsTop + i * C.rowMm
    const base = top + C.rowMm / 2 + C.cellMm * 0.35
    const cells = [r.label, String(r.qty), r.l, r.w, r.h]
    cells.forEach((c, k) => items.push(textItem(cols[k], base, c, C.cellMm, 'left', 'table-cell', INK.cell)))
    const ruleY = top + C.rowMm - C.rowRuleMm / 2
    items.push({ kind: 'line', x1: d.x, y1: ruleY, x2: d.x + d.w, y2: ruleY, widthMm: C.rowRuleMm, gray: INK.light, role: 'table-rule' })
  })

  if (ctx.titleBlock !== false) {
    items.push(
      ...titleBlockItems(layout, {
        documentName: ctx.documentName,
        subtitle: 'Cut list',
        centerMain: `${rows.length} part${rows.length === 1 ? '' : 's'} · ${pieces} piece${pieces === 1 ? '' : 's'}`,
        centerSub: null,
        showScaleBar: false,
        tileId: null,
        pageText: `Page ${ctx.pageNumber} of ${ctx.totalPages}`,
        dateText: ctx.dateText,
      }),
    )
  }
  return items
}

/** A synthetic tile for a furniture-only page. */
export function blankTile(layout: PrintLayout, page: number, id: string): TileSpec {
  const d = layout.page.drawing
  return { id, row: 0, col: 0, page, imageRectMm: { x: d.x, y: d.y, w: d.w, h: d.h }, imagePx: { w: 0, h: 0 }, modelRect: null, overlapRight: false, overlapBottom: false, neighbors: {} }
}
