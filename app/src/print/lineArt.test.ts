import { describe, expect, it } from 'vitest'
import { PAPER_SIZES } from './paper'
import { layoutPrint } from './layout'
import { lineArtSvgDocument, lineArtSvgForTile, projectAnnotationOverlay, type LineDrawingData } from './lineArt'

/** A 100 mm square (hard) with one hidden diagonal, in view-plane metres. */
function square(): LineDrawingData {
  const coords = new Float64Array([
    -0.05, -0.05, 0.05, -0.05,
    0.05, -0.05, 0.05, 0.05,
    0.05, 0.05, -0.05, 0.05,
    -0.05, 0.05, -0.05, -0.05,
    -0.05, -0.05, 0.05, 0.05,
  ])
  return { coords, kinds: new Uint8Array([0, 0, 0, 0, 4]), sids: new BigUint64Array([1n, 1n, 1n, 1n, 1n]), bounds: [-0.05, -0.05, 0.05, 0.05] }
}

describe('lineArtSvgDocument', () => {
  it('is true size at the scale, y flipped, hidden lines only on request', () => {
    const d = square()
    const svg = lineArtSvgDocument(d, 1, { hiddenDashed: false, marginMm: 0 })
    expect(svg).toContain('width="100mm" height="100mm"')
    expect(svg).toContain('viewBox="-50 -50 100 100"')
    expect(svg).toContain('class="hard"')
    expect(svg).not.toContain('class="hidden"')
    const dashed = lineArtSvgDocument(d, 0.5, { hiddenDashed: true, marginMm: 5 })
    expect(dashed).toContain('width="60mm" height="60mm"')
    expect(dashed).toContain('class="hidden"')
    expect(dashed).toContain('stroke-dasharray')
  })
})

describe('lineArtSvgForTile', () => {
  it('maps the tile model rect to the image rect and keeps strokes at physical weight', () => {
    const layout = layoutPrint({ mode: 'scaled', paper: PAPER_SIZES.letter, orientation: 'portrait', marginMm: 12.7, titleBlockMm: 10, overlapMm: 10, dpi: 300, ratio: 1, extent: { x: -0.05, y: -0.05, w: 0.1, h: 0.1 } })
    const tile = layout.tiles[0]
    const svg = lineArtSvgForTile(square(), tile, layout, 1, { hiddenDashed: true, annotations: { segments: [0, 0, 0.02, 0], labels: [{ x: 0.01, y: 0.005, text: '20 mm', detached: false }] } })
    expect(svg).toContain(`width="${tile.imageRectMm.w}mm"`)
    // viewBox = the model rect (metres), y flipped.
    expect(svg).toContain(`viewBox="${tile.modelRect!.x}`)
    // 0.35 mm hard stroke at 1:1 = 0.00035 user units (metres).
    expect(svg).toMatch(/class="hard" stroke-width="0.00035"/)
    expect(svg).toContain('20 mm')
    expect(svg).toContain('class="annotation"')
  })
})

describe('projectAnnotationOverlay', () => {
  it('projects onto the print frame (top view: x right, y up = world x, y)', () => {
    const cam = { kind: 'ortho' as const, center: [0, 0, 0] as [number, number, number], dir: [0, 0, -1] as [number, number, number], up: [0, 1, 0] as [number, number, number], rect: { x: -1, y: -1, w: 2, h: 2 }, depth: { min: -1, max: 1 } }
    const o = projectAnnotationOverlay({ segments: [1, 2, 3, 4, 5, 6], labels: [{ position: [0.5, 0.25, 9], text: 'x', detached: false }] }, cam)
    expect(o.segments.map((v) => Math.round(v * 1000) / 1000)).toEqual([1, 2, 4, 5])
    expect(o.labels[0].x).toBeCloseTo(0.5, 9)
    expect(o.labels[0].y).toBeCloseTo(0.25, 9)
  })
})
