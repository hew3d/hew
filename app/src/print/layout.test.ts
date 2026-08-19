import { describe, expect, it } from 'vitest'
import { PAPER_SIZES, matchNamedPaper, paperSize, defaultPaperForRegion } from './paper'
import {
  clampDpi,
  fitRatioFor,
  layoutPrint,
  mmToPx,
  pageCountText,
  pickScaleBar,
  pxToMm,
  rowLetter,
  tileId,
  type LayoutInput,
} from './layout'

const base: LayoutInput = {
  mode: 'scaled',
  paper: PAPER_SIZES.letter,
  orientation: 'portrait',
  marginMm: 12.7,
  titleBlockMm: 10,
  overlapMm: 10,
  dpi: 300,
  ratio: 1,
  scaleBarSystem: 'imperial',
}

describe('paper', () => {
  it('names the standard sizes and matches either orientation', () => {
    expect(matchNamedPaper(215.9, 279.4)).toBe('letter')
    expect(matchNamedPaper(297, 210)).toBe('a4')
    expect(matchNamedPaper(200, 200)).toBeNull()
  })
  it('stores custom sizes portrait and clamps', () => {
    expect(paperSize({ w_mm: 400, h_mm: 300 })).toEqual({ w: 300, h: 400 })
    expect(paperSize({ w_mm: 1, h_mm: 5000 })).toEqual({ w: 50, h: 2000 })
  })
  it('defaults Letter for Letter regions and A4 elsewhere', () => {
    expect(defaultPaperForRegion('US')).toBe('letter')
    expect(defaultPaperForRegion('DE')).toBe('a4')
    expect(defaultPaperForRegion(null)).toBe('a4')
  })
})

describe('layout helpers', () => {
  it('px/mm round trip is exact after rounding', () => {
    const px = mmToPx(200.5, 300)
    expect(pxToMm(px, 300)).toBeCloseTo(200.5, 0)
    expect(mmToPx(pxToMm(px, 300), 300)).toBe(px)
  })
  it('row letters and tile ids', () => {
    expect(rowLetter(0)).toBe('A')
    expect(rowLetter(25)).toBe('Z')
    expect(rowLetter(26)).toBe('AA')
    expect(tileId(1, 2)).toBe('B3')
  })
  it('clamps dpi to the pixel budget but never below the floor', () => {
    expect(clampDpi(300, 215.9, 279.4)).toBe(300)
    // A1-ish sheet: 594×841 mm at 300 dpi = 69 Mpx → steps down.
    const d = clampDpi(300, 594, 841)
    expect(d).toBeLessThan(300)
    expect(d).toBeGreaterThanOrEqual(150)
    expect(mmToPx(594, d) * mmToPx(841, d)).toBeLessThanOrEqual(24_000_000)
  })
})

describe('standard mode', () => {
  it('is one letterboxed page keeping the viewport aspect; auto orientation follows the aspect', () => {
    const l = layoutPrint({ ...base, mode: 'standard', orientation: 'auto', viewportAspect: 16 / 9 })
    expect(l.tiles).toHaveLength(1)
    expect(l.page.orientation).toBe('landscape')
    const t = l.tiles[0]
    expect(t.imagePx.w / t.imagePx.h).toBeCloseTo(16 / 9, 2)
    // Fills the drawing width, centred vertically.
    expect(t.imageRectMm.w).toBeCloseTo(l.page.drawing.w, 0)
    expect(t.imageRectMm.y).toBeGreaterThan(l.page.drawing.y)
    expect(pageCountText(l)).toBe('1 page')
  })
  it('a tall viewport on landscape paper fits the height', () => {
    const l = layoutPrint({ ...base, mode: 'standard', orientation: 'landscape', viewportAspect: 0.5 })
    const t = l.tiles[0]
    expect(t.imageRectMm.h).toBeCloseTo(l.page.drawing.h, 0)
    expect(t.imageRectMm.x).toBeGreaterThan(l.page.drawing.x)
  })
})

describe('scaled mode — the three stories', () => {
  it('a 100 mm cube top view at 1:1 fits one Letter page and maps 100 mm to 1181 px', () => {
    const l = layoutPrint({ ...base, extent: { x: -0.05, y: -0.05, w: 0.1, h: 0.1 } })
    expect(l.tiles).toHaveLength(1)
    const t = l.tiles[0]
    // The tile's model rect covers the whole drawing area at 1:1: its width
    // in meters is the drawing width in meters.
    expect(t.modelRect!.w).toBeCloseTo(t.imageRectMm.w / 1000, 6)
    // 100 mm of model = 100/25.4*300 px.
    const pxPerMeter = t.imagePx.w / t.modelRect!.w
    expect(pxPerMeter * 0.1).toBeCloseTo(1181.1, 0)
    // The extent is centred in the tile.
    const cx = t.modelRect!.x + t.modelRect!.w / 2
    expect(cx).toBeCloseTo(0, 6)
  })
  it('a 500 × 300 mm slab at 1:1 tiles 3 × 2 either way with the band reserved; auto keeps portrait on the tie', () => {
    // Letter portrait drawing area is 190.5 × 244 mm (½ in margins, 10 mm
    // title block); tiled with the 10 mm band reserved inside (decision
    // #12) it steps 180.5 × 234. Landscape: 254 × 180.5 → 244 × 170.5.
    const extent = { x: -0.25, y: -0.15, w: 0.5, h: 0.3 }
    const p = layoutPrint({ ...base, orientation: 'portrait', extent })
    expect([p.cols, p.rows]).toEqual([3, 2])
    expect(p.page.drawing.w).toBeCloseTo(180.5, 6)
    expect(p.page.drawing.h).toBeCloseTo(234, 6)
    const land = layoutPrint({ ...base, orientation: 'landscape', extent })
    expect([land.cols, land.rows]).toEqual([3, 2])
    expect(land.page.drawing.w).toBeCloseTo(244, 6)
    const auto = layoutPrint({ ...base, orientation: 'auto', extent })
    expect(auto.page.orientation).toBe('portrait')
    expect(pageCountText(auto)).toBe('6 pages (3 × 2)')
    expect(auto.tiles.map((t) => t.id)).toEqual(['A1', 'A2', 'A3', 'B1', 'B2', 'B3'])
    // Overlap off: the full drawing area steps, and landscape wins 2 × 2.
    const noBand = layoutPrint({ ...base, orientation: 'auto', extent, overlapMm: 0 })
    expect(noBand.page.orientation).toBe('landscape')
    expect([noBand.cols, noBand.rows]).toEqual([2, 2])
  })
  it('measure on paper: a 2.4 m table at 1:10 fits one A4 page', () => {
    const l = layoutPrint({
      ...base,
      paper: PAPER_SIZES.a4,
      ratio: 0.1,
      scaleBarSystem: 'metric',
      extent: { x: -1.2, y: -0.5, w: 2.4, h: 1.0 },
      orientation: 'auto',
    })
    expect(l.tiles).toHaveLength(1)
    expect(l.page.orientation).toBe('landscape')
    expect(l.scaleBar).not.toBeNull()
    // Graphic scale at 1:10 on a 261.6 mm drawing: 4·p ≤ 80 → 20 cm steps
    // of 20 mm, labelled 0 … 80 cm; the bar is 80 mm on paper.
    expect(l.scaleBar!.paperMm).toBeCloseTo(80, 9)
    expect(l.scaleBar!.segmentMeters).toBeCloseTo(0.2, 9)
    expect(l.scaleBar!.labels).toEqual(['0', '20', '40', '60', '80 cm'])
    expect(l.scaleBar!.paperLabel).toBe('80 mm')
    // A one-page scaled print reserves no band, whatever was asked.
    expect(l.overlapMm).toBe(0)
    expect(l.page.drawing.w).toBeCloseTo(271.6, 6)
  })
})

describe('tiling geometry', () => {
  // 500 × 450 mm on Letter portrait (180.5 × 234 mm tiled drawing area) → 3 × 2.
  const extent = { x: 0, y: 0, w: 0.5, h: 0.45 }
  it('tiles subdivide the extent px-exactly and only inner edges overlap', () => {
    const l = layoutPrint({ ...base, extent, orientation: 'portrait' })
    expect([l.cols, l.rows]).toEqual([3, 2])
    for (const t of l.tiles) {
      expect(t.overlapRight).toBe(t.col < l.cols - 1)
      expect(t.overlapBottom).toBe(t.row < l.rows - 1)
      // Overlapping tiles are wider than the drawing area by the band.
      const band = t.overlapRight ? 10 : 0
      expect(t.imageRectMm.w).toBeCloseTo(l.page.drawing.w + band, 0)
      // Image px ↔ mm ↔ model agree exactly.
      expect(pxToMm(t.imagePx.w, l.dpi)).toBeCloseTo(t.imageRectMm.w, 9)
      expect(t.modelRect!.w * 1000).toBeCloseTo(t.imageRectMm.w, 9)
    }
    // Adjacent tiles' model rects abut at the trim line: A2 starts one
    // drawing-width right of A1.
    const a1 = l.tiles[0].modelRect!
    const a2 = l.tiles[1].modelRect!
    expect(a2.x - a1.x).toBeCloseTo(l.page.drawing.w / 1000, 9)
    // Row B is one drawing-height LOWER (y up): B1.top = A1.top - d.h.
    const b1 = l.tiles[l.cols].modelRect!
    expect(a1.y + a1.h - (b1.y + b1.h)).toBeCloseTo(l.page.drawing.h / 1000, 9)
    // The union of trim cells covers the extent, centred.
    const gridW = (l.cols * l.page.drawing.w) / 1000
    const left = a1.x
    expect(left + gridW / 2).toBeCloseTo(extent.x + extent.w / 2, 9)
    // Neighbors.
    expect(l.tiles[0].neighbors).toEqual({ right: 'A2', down: 'B1' })
    expect(l.tiles[4].neighbors).toEqual({ left: 'B1', right: 'B3', up: 'A2' })
  })
  it('overlap off means no bands, equal-size tiles, and the full drawing area as the step', () => {
    const l = layoutPrint({ ...base, extent, overlapMm: 0 })
    expect(l.page.drawing.w).toBeCloseTo(190.5, 6)
    for (const t of l.tiles) {
      expect(t.overlapRight).toBe(false)
      expect(t.imageRectMm.w).toBeCloseTo(l.page.drawing.w, 0)
    }
  })
  it('the band is reserved inside the printable area, uniformly: every tile steps by the reduced area and the last column leaves its band empty', () => {
    const l = layoutPrint({ ...base, extent })
    const d = l.page.drawing
    // paper − 2·margin − overlap = 215.9 − 25.4 − 10; height also minus the title block.
    expect(d.w).toBeCloseTo(180.5, 6)
    expect(d.h).toBeCloseTo(234, 6)
    // The trim line (drawing right edge) + band stays inside the margin.
    expect(d.x + d.w + l.overlapMm).toBeCloseTo(l.page.paper.w - l.page.marginMm, 6)
    const last = l.tiles[l.cols - 1]
    expect(last.overlapRight).toBe(false)
    expect(last.imageRectMm.w).toBeCloseTo(d.w, 0)
    // Narrow margins: 5 mm band, never clipped.
    const n = layoutPrint({ ...base, marginMm: 6.35, overlapMm: 5, extent })
    expect(n.overlapMm).toBe(5)
    expect(n.page.drawing.w).toBeCloseTo(215.9 - 12.7 - 5, 6)
  })
  it('nudge shifts the extent within the grid, never adds a tile, and reports the clamped value', () => {
    const l0 = layoutPrint({ ...base, extent })
    const a1 = l0.tiles[0].modelRect!.x
    const l1 = layoutPrint({ ...base, extent, nudgeMm: { dx: 20, dy: 0 } })
    // The extent moved right by 20 mm = tiles' model x moved LEFT by 0.02 m.
    expect(l1.tiles[0].modelRect!.x).toBeCloseTo(a1 - 0.02, 9)
    expect(l1.nudgeMm.dx).toBeCloseTo(20, 9)
    expect(l1.cols).toBe(l0.cols)
    // A big nudge is clamped to the slack: the count is unchanged and no
    // page can be empty (500 mm on 3 × 180.5 → slack 41.5 → ±20.75).
    const l2 = layoutPrint({ ...base, extent, nudgeMm: { dx: 500, dy: 0 } })
    expect(l2.cols).toBe(l0.cols)
    expect(l2.nudgeMm.dx).toBeCloseTo((3 * 180.5 - 500) / 2, 6)
    const l3 = layoutPrint({ ...base, extent, nudgeMm: { dx: -500, dy: 0 } })
    expect(l3.cols).toBe(l0.cols)
    expect(l3.nudgeMm.dx).toBeCloseTo(-(3 * 180.5 - 500) / 2, 6)
    // The extent still starts inside the first tile.
    expect(l3.tiles[0].modelRect!.x).toBeLessThanOrEqual(extent.x + 1e-9)
  })
  it('the fit ratio puts the extent on exactly one page', () => {
    const r = fitRatioFor({ ...base, extent, orientation: 'auto' })
    const l = layoutPrint({ ...base, extent, ratio: r, orientation: 'auto', overlapMm: 0 })
    expect(l.tiles).toHaveLength(1)
    const l2 = layoutPrint({ ...base, extent, ratio: r * 1.01, orientation: 'auto', overlapMm: 0 })
    expect(l2.tiles.length).toBeGreaterThan(1)
  })
  it('graphic scale bar: four round-unit segments, ≥ 6 mm each, ≤ min(40 % of the drawing, 80 mm) overall', () => {
    // 1:1 on Letter → 1 cm segments (10 mm; 4·20 = 80 > 76.2).
    expect(pickScaleBar(190.5, 1, 'metric')).toMatchObject({ segmentMm: 10, paperMm: 40, labels: ['0', '1', '2', '3', '4 cm'], paperLabel: '40 mm' })
    // 1:5 → 5 cm segments of 10 mm, "0 5 10 15 20 cm".
    expect(pickScaleBar(190.5, 0.2, 'metric').labels).toEqual(['0', '5', '10', '15', '20 cm'])
    // 1:100 → 1 m segments of 10 mm.
    expect(pickScaleBar(190.5, 0.01, 'metric').labels).toEqual(['0', '1', '2', '3', '4 m'])
    // 1:2 → 2 cm segments of 10 mm (5 cm would be 25 mm ×4 = 100 > 76.2).
    expect(pickScaleBar(190.5, 0.5, 'metric')).toMatchObject({ segmentMm: 10, labels: ['0', '2', '4', '6', '8 cm'] })
    // Imperial 1:1 → ½″ segments of 12.7 mm (1″ would be 101.6 > 76.2), bar 2 in.
    expect(pickScaleBar(190.5, 1, 'imperial')).toMatchObject({ labels: ['0', '½', '1', '1½', '2 in'], paperLabel: '2 in' })
    // 1″ = 1′ → 6″ segments of 12.7 mm, "0 6 12 18 24 in"? No — 6″ steps read in inches.
    expect(pickScaleBar(190.5, 1 / 12, 'imperial').labels).toEqual(['0', '6', '12', '18', '24 in'])
    // 1:48 (¼″ = 1′) → 3′ segments (36 in × 25.4 / 48 = 19.05 mm; 4 × 19.05 = 76.2 fits exactly), labelled in feet.
    expect(pickScaleBar(190.5, 1 / 48, 'imperial').labels).toEqual(['0', '3', '6', '9', '12 ft'])
    // A narrow drawing shrinks the bar: 40 % of 70 = 28 → 5 mm segments (the
    // ≥ 6 mm wish yields to the fit; the bar never overruns its slot).
    const narrow = pickScaleBar(70, 1, 'metric')
    expect(narrow.segmentMm).toBe(5)
    expect(narrow.paperMm).toBeLessThanOrEqual(28)
    // 4 mm segments would fit a 190 mm drawing at 1:1 too but 1 cm is rounder — the largest round step wins.
    expect(pickScaleBar(120, 1, 'metric').labels).toEqual(['0', '1', '2', '3', '4 cm'])
  })
})

describe('degenerate inputs stay bounded', () => {
  it('a margin that leaves no drawing area yields one tile, not a runaway grid', () => {
    const l = layoutPrint({ ...base, paper: { w: 60, h: 100 }, marginMm: 30, extent: { x: 0, y: 0, w: 5, h: 5 } })
    // Zero drawing width → one column (no division by zero, no Infinity).
    expect(l.cols).toBe(1)
    expect(l.tiles.length).toBeLessThanOrEqual(500)
    const l2 = layoutPrint({ ...base, ratio: 20, extent: { x: 0, y: 0, w: 100, h: 100 } })
    expect(l2.cols).toBeLessThanOrEqual(500)
    expect(l2.rows).toBeLessThanOrEqual(500)
  })
  it('a band bigger than a tiny sheet can hold is capped at half the drawing area', () => {
    const l = layoutPrint({ ...base, paper: { w: 60, h: 80 }, marginMm: 6, titleBlockMm: 0, overlapMm: 40, extent: { x: 0, y: 0, w: 0.5, h: 0.45 } })
    expect(l.overlapMm).toBeCloseTo(24, 6)
    expect(l.page.drawing.w).toBeCloseTo(24, 6)
    expect(l.tiles.length).toBeLessThanOrEqual(500)
    const n = layoutPrint({ ...base, extent: { x: 0, y: 0, w: 0.5, h: 0.45 } })
    expect(n.overlapMm).toBe(10)
  })
})
