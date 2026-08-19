import { describe, expect, it } from 'vitest'
import { PAPER_SIZES } from './paper'
import { layoutPrint } from './layout'
import { FURNITURE, INK, pageFurniture, type FurnitureContext, type FurnitureItem } from './furniture'
import { cutListPageFurniture, cutListRowsPerPage, type CutListRow } from './cutList'

const ctx: FurnitureContext = {
  documentName: 'Cafe table',
  subtitle: 'Top view · Model',
  scaleText: '1:1 (1 cm = 1 cm)',
  dateText: '17 Aug 2026',
  marks: true,
  titleBlock: true,
  scaleBar: true,
}

type TextItem = Extract<FurnitureItem, { kind: 'text' }>
type LineItem = Extract<FurnitureItem, { kind: 'line' }>
type RectItem = Extract<FurnitureItem, { kind: 'rect' }>

const texts = (items: FurnitureItem[], role?: string): string[] =>
  items.filter((i): i is TextItem => i.kind === 'text' && (role === undefined || i.role === role)).map((i) => i.text)

describe('page furniture', () => {
  // 500 × 450 mm at 1:1 on Letter portrait → 3 × 2 tiles, 10 mm band reserved.
  const layout = layoutPrint({
    mode: 'scaled',
    paper: PAPER_SIZES.letter,
    orientation: 'portrait',
    marginMm: 12.7,
    titleBlockMm: 10,
    overlapMm: 10,
    dpi: 300,
    ratio: 1,
    extent: { x: 0, y: 0, w: 0.5, h: 0.45 },
    scaleBarSystem: 'metric',
  })

  it('a first tile carries the crop marks that fit, both trim lines with captions, inside neighbor labels, and the graphic scale bar', () => {
    const items = pageFurniture(layout, layout.tiles[0], ctx)
    const roles = items.map((i) => i.role)
    // 8 corner bars minus the four that would run into the right band or the
    // bottom band: TR-h, BR-h, BL-v, BR-v → 4 remain.
    expect(roles.filter((r) => r === 'crop-mark')).toHaveLength(4)
    expect(roles.filter((r) => r === 'trim-line')).toHaveLength(2)
    const captions = items.filter((i): i is TextItem => i.kind === 'text' && i.role === 'trim-caption')
    expect(captions).toHaveLength(2)
    expect(captions.every((c) => c.text === '✂ trim')).toBe(true)
    expect(captions.some((c) => c.rotate === 90)).toBe(true)
    // Trim lines sit at the drawing edge and span the band.
    const d = layout.page.drawing
    const vertical = items.find((i): i is LineItem => i.kind === 'line' && i.role === 'trim-line' && i.x1 === i.x2)!
    expect(vertical.x1).toBeCloseTo(d.x + d.w, 9)
    expect(vertical.y2).toBeCloseTo(d.y + d.h + 10, 9)
    expect(vertical.widthMm).toBe(FURNITURE.trimWidthMm)
    // Neighbor labels are INSIDE the drawing edge (arrow first).
    const right = items.find((i): i is TextItem => i.kind === 'text' && i.role === 'neighbor-label' && i.text === '→ A2')!
    expect(right.x).toBeCloseTo(d.x + d.w - 2, 9)
    expect(texts(items, 'neighbor-label')).toEqual(expect.arrayContaining(['→ A2', '↓ B1']))
    // Graphic scale bar: two black segments + outline, five labels, caption.
    const bar = items.filter((i): i is RectItem => i.role === 'scale-bar' && i.kind === 'rect')
    expect(bar).toHaveLength(3)
    expect(bar[2]).toMatchObject({ w: layout.scaleBar!.paperMm, strokeMm: FURNITURE.barStrokeMm })
    expect(texts(items, 'scale-bar-label')).toEqual(['0', '1', '2', '3', '4 cm'])
    expect(texts(items, 'scale-bar-caption')).toEqual(['40 mm'])
    // Title block: big tile id, page line with the tile, bold HEW.
    expect(texts(items, 'title-tile')).toEqual(['A1'])
    expect(texts(items, 'title-page')[0]).toBe('Page 1 of 6 · Tile A1 · 17 Aug 2026 ·')
    expect(items.find((i) => i.role === 'title-brand')).toMatchObject({ text: 'HEW', bold: true })
    expect(texts(items, 'title-scale')).toEqual(['1:1 (1 cm = 1 cm)'])
    expect(texts(items, 'title-view')).toEqual(['Top view · Model'])
    // The title rule is 0.25 mm at the drawing's bottom edge.
    const rule = items.find((i): i is LineItem => i.role === 'title-rule')!
    expect(rule.widthMm).toBe(0.25)
    expect(rule.y1).toBeCloseTo(layout.page.titleBlock!.y, 9)
    // No hairline frame in either mode.
    expect(roles.some((r) => (r as string) === 'frame')).toBe(false)
  })

  it('the last tile has no trim lines and no right/down labels, and gets its right-side crop marks back', () => {
    const last = layout.tiles[layout.tiles.length - 1]
    const items = pageFurniture(layout, last, ctx)
    expect(items.filter((i) => i.role === 'trim-line')).toHaveLength(0)
    expect(items.filter((i) => i.role === 'trim-caption')).toHaveLength(0)
    const labels = texts(items, 'neighbor-label')
    expect(labels).toEqual(expect.arrayContaining(['← B2', '↑ A3']))
    expect(labels.some((t) => t.includes('→'))).toBe(false)
    // No band on the right/bottom: all eight bars — the two bottom vertical
    // ones run down into the title strip's ends, whose content is inset.
    expect(items.filter((i) => i.role === 'crop-mark')).toHaveLength(8)
    const doc = items.find((i): i is TextItem => i.kind === 'text' && i.role === 'title-doc')!
    expect(doc.x).toBeCloseTo(layout.page.titleBlock!.x + 1.8, 9)
  })

  it('a single scaled sheet gets corner crop marks only; with no title block all eight bars fit', () => {
    const one = layoutPrint({ mode: 'scaled', paper: PAPER_SIZES.letter, orientation: 'portrait', marginMm: 12.7, titleBlockMm: 0, overlapMm: 10, dpi: 300, ratio: 1, extent: { x: 0, y: 0, w: 0.1, h: 0.1 }, scaleBarSystem: 'metric' })
    const items = pageFurniture(one, one.tiles[0], { ...ctx, titleBlock: false })
    expect(items.filter((i) => i.role === 'crop-mark')).toHaveLength(8)
    expect(items.filter((i) => i.role === 'trim-line')).toHaveLength(0)
    expect(items.filter((i) => i.role === 'neighbor-label')).toHaveLength(0)
    expect(items.filter((i) => i.role === 'title-rule')).toHaveLength(0)
    // Every crop bar is 5 × 0.3 mm in rule ink.
    for (const b of items.filter((i): i is RectItem => i.role === 'crop-mark' && i.kind === 'rect')) {
      expect(Math.max(b.w, b.h)).toBe(FURNITURE.cropLenMm)
      expect(Math.min(b.w, b.h)).toBe(FURNITURE.cropWidthMm)
      expect(b.fillGray).toBe(INK.rule)
    }
  })

  it('everything stays on the sheet and no ink is lighter than 30 % gray', () => {
    // Narrow margins too: the 6.5 mm crop-mark reach is clipped to the 6.35 mm margin.
    const narrow = layoutPrint({ mode: 'scaled', paper: PAPER_SIZES.letter, orientation: 'portrait', marginMm: 6.35, titleBlockMm: 10, overlapMm: 5, dpi: 300, ratio: 1, extent: { x: 0, y: 0, w: 0.5, h: 0.45 }, scaleBarSystem: 'metric' })
    for (const tile of [...layout.tiles, ...narrow.tiles]) {
      const lay = layout.tiles.includes(tile) ? layout : narrow
      for (const it of pageFurniture(lay, tile, ctx)) {
        const xs = it.kind === 'line' ? [it.x1, it.x2] : it.kind === 'rect' ? [it.x, it.x + it.w] : [it.x]
        const ys = it.kind === 'line' ? [it.y1, it.y2] : it.kind === 'rect' ? [it.y, it.y + it.h] : [it.y]
        for (const x of xs) expect(x).toBeGreaterThanOrEqual(0)
        for (const x of xs) expect(x).toBeLessThanOrEqual(lay.page.paper.w)
        for (const y of ys) expect(y).toBeGreaterThanOrEqual(0)
        for (const y of ys) expect(y).toBeLessThanOrEqual(lay.page.paper.h)
        const gray = it.kind === 'rect' ? (it.fillGray ?? it.gray ?? 0) : it.gray
        expect(gray).toBeLessThanOrEqual(0.7)
      }
    }
  })

  it('standard mode has no marks and no scale slot at all', () => {
    const std = layoutPrint({ mode: 'standard', paper: PAPER_SIZES.a4, orientation: 'auto', marginMm: 12.7, titleBlockMm: 10, overlapMm: 0, dpi: 300, viewportAspect: 1.5 })
    const items = pageFurniture(std, std.tiles[0], { ...ctx, scaleText: null, subtitle: 'Perspective view' })
    expect(items.some((i) => i.role === 'crop-mark')).toBe(false)
    expect(items.some((i) => i.role === 'trim-line')).toBe(false)
    expect(texts(items, 'title-scale')).toEqual([])
    expect(texts(items, 'title-scale-sub')).toEqual([])
    expect(texts(items, 'title-view')).toEqual(['Perspective view'])
    expect(texts(items, 'title-page')).toEqual(['Page 1 of 1'])
    expect(texts(items, 'title-date')).toEqual(['17 Aug 2026 ·'])
    expect(items.some((i) => i.role === 'scale-bar')).toBe(false)
    expect(items.some((i) => i.role === 'title-tile')).toBe(false)
  })

  it('marks off leaves only the title block; title block off leaves only marks', () => {
    const noMarks = pageFurniture(layout, layout.tiles[0], { ...ctx, marks: false })
    expect(noMarks.every((i) => i.role.startsWith('title-') || i.role.startsWith('scale-bar'))).toBe(true)
    const noTitle = pageFurniture(layout, layout.tiles[0], { ...ctx, titleBlock: false })
    expect(noTitle.some((i) => i.role.startsWith('title-'))).toBe(false)
    expect(noTitle.some((i) => i.role === 'scale-bar')).toBe(false)
    expect(noTitle.some((i) => i.role === 'crop-mark')).toBe(true)
  })
})

describe('cut-list page', () => {
  const layout = layoutPrint({ mode: 'standard', paper: PAPER_SIZES.letter, orientation: 'portrait', marginMm: 12.7, titleBlockMm: 10, overlapMm: 0, dpi: 300, viewportAspect: 1 })
  const rows: CutListRow[] = [
    { label: 'Tabletop', qty: 1, l: '440 mm', w: '440 mm', h: '30 mm' },
    { label: 'Leg', qty: 4, l: '680 mm', w: '34 mm', h: '34 mm' },
    { label: 'Stretcher', qty: 2, l: '420 mm', w: '40 mm', h: '12 mm' },
  ]
  it('lays out heading, header row, one 8.5 mm row per part, and a "parts · pieces" title block', () => {
    const items = cutListPageFurniture(layout, rows, 0, { documentName: 'Café table', pageNumber: 7, totalPages: 7, dateText: '17 Aug 2026' })
    expect(texts(items, 'table-heading')).toEqual(['Cut list'])
    // No "lengths in …" anywhere: the cells carry their units.
    expect(texts(items, 'table-subheading')).toEqual(['Café table'])
    expect(texts(items, 'table-header')).toEqual(['Part', 'Qty', 'L', 'W', 'H'])
    expect(texts(items, 'table-cell')).toEqual(['Tabletop', '1', '440 mm', '440 mm', '30 mm', 'Leg', '4', '680 mm', '34 mm', '34 mm', 'Stretcher', '2', '420 mm', '40 mm', '12 mm'])
    expect(texts(items).some((t) => t.includes('lengths in'))).toBe(false)
    // Column origins from the drawing-area left edge (Letter): 0/82/104/134/164.
    const d = layout.page.drawing
    const headerXs = items.filter((i): i is TextItem => i.kind === 'text' && i.role === 'table-header').map((i) => i.x - d.x)
    ;[0, 82, 104, 134, 164].forEach((x, i) => expect(headerXs[i]).toBeCloseTo(x, 9))
    // Header rule 0.3 mm dark; row rules 0.15 mm at 55 % gray, 8.5 mm apart.
    const rules = items.filter((i): i is LineItem => i.kind === 'line' && i.role === 'table-rule')
    expect(rules[0].widthMm).toBe(0.3)
    expect(rules.slice(1).every((r) => r.widthMm === 0.15 && r.gray === INK.light)).toBe(true)
    expect(rules[2].y1 - rules[1].y1).toBeCloseTo(8.5, 9)
    expect(texts(items, 'title-scale')).toEqual(['3 parts · 7 pieces'])
    expect(texts(items, 'title-scale-sub')).toEqual([])
    expect(texts(items, 'title-view')).toEqual(['Cut list'])
    expect(texts(items, 'title-page')).toEqual(['Page 7 of 7'])
    // Rows per Letter page: (244 − 25.3 − 2.8 − 1.6 − 0.3) / 8.5 = 25.
    expect(cutListRowsPerPage(layout)).toBe(25)
  })
  it('narrower sheets shrink the Part column (A4)', () => {
    const a4 = layoutPrint({ mode: 'standard', paper: PAPER_SIZES.a4, orientation: 'portrait', marginMm: 12.7, titleBlockMm: 10, overlapMm: 0, dpi: 300, viewportAspect: 1 })
    const items = cutListPageFurniture(a4, rows, 0, { documentName: 'T', pageNumber: 1, totalPages: 1, dateText: 'd' })
    const headerXs = items.filter((i): i is TextItem => i.kind === 'text' && i.role === 'table-header').map((i) => i.x - a4.page.drawing.x)
    // A4 drawing 184.6: 184.6 − 164 = 20.6 < 26 → every column left of H shifts by 5.4.
    expect(headerXs[1]).toBeCloseTo(82 - 5.4, 6)
    expect(headerXs[4]).toBeCloseTo(a4.page.drawing.w - 26, 6)
  })
})
