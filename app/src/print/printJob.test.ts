import { describe, expect, it, vi } from 'vitest'
import { defaultPrintOptions, planPrint, printDocName, type ViewportPrintSource } from './printJob'

function source(aspect = 4 / 3, extent = { x: -0.25, y: -0.05, w: 0.5, h: 0.1 }): ViewportPrintSource {
  return {
    getPrintView: () => ({ dir: [0, 0, -1], up: [0, 1, 0], target: [0, 0, 0], projection: 'perspective', orthoSize: { w: 1, h: 1 }, aspect }),
    computePrintExtent: vi.fn(() => ({ center: [0, 0, 0] as [number, number, number], rect: extent, depth: { min: -0.1, max: 0.1 }, empty: false })),
    getSelectedIds: () => ({ objects: [], instances: [] }),
  }
}
const ctx = { documentName: 'Trestle Table.hew', sceneName: null, unitFormat: 'mm' as const, dateText: 'd' }

describe('planPrint — Standard', () => {
  it('Zoom: Current letterboxes the viewport aspect; the camera is the live one, unfitted', () => {
    const plan = planPrint({ ...defaultPrintOptions('mm', 'letter'), orientation: 'portrait' }, source(), ctx)
    const t = plan.layout.tiles[0]
    expect(t.imagePx.w / t.imagePx.h).toBeCloseTo(4 / 3, 2)
    expect(plan.pages[0].request.camera).toEqual({ kind: 'live', fit: false })
  })
  it('Zoom: Fit fills the drawing area, follows the model aspect for Auto orientation, and asks the pass to re-frame', () => {
    // A 5:1 wide extent → Auto picks landscape; the image takes the drawing area's own aspect.
    const plan = planPrint({ ...defaultPrintOptions('mm', 'letter'), zoom: 'fit', orientation: 'auto' }, source(), ctx)
    expect(plan.layout.page.orientation).toBe('landscape')
    const d = plan.layout.page.drawing
    const t = plan.layout.tiles[0]
    expect(t.imageRectMm.w).toBeCloseTo(d.w, 0)
    expect(t.imageRectMm.h).toBeCloseTo(d.h, 0)
    expect(plan.pages[0].request.camera).toEqual({ kind: 'live', fit: true })
  })
})

describe('printDocName', () => {
  it('drops the .hew extension, case-insensitively, and never prints an empty name', () => {
    expect(printDocName('Trestle Table.hew')).toBe('Trestle Table')
    expect(printDocName('bench.HEW')).toBe('bench')
    expect(printDocName('  ')).toBe('Untitled')
    expect(printDocName('.hew')).toBe('Untitled')
    expect(printDocName('a.skp')).toBe('a.skp')
  })
})
