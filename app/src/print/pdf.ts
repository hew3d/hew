/**
 * Save PDF… (docs/design/printing.md §9b): the same pages the print dialog
 * composes, written by Hew's own PDF writer (`crates/pdfwrite`, reached
 * through wasm `build_pdf`) — exact page sizes, no OS print dialog, identical
 * on desktop, web, and the phone. Vector Line-art pages stay vector (PDF
 * paths); raster pages embed as JPEG (or Flate RGB for PNG); the page
 * furniture goes in as paths/text/rects from the SAME item list the DOM page
 * draws, so the PDF and the print root agree by construction.
 */
import { build_pdf } from '../wasm/pkg/wasm_api.js'
import type { FurnitureItem } from './furniture'
import type { PrintLayout, TileSpec } from './layout'
import { LINE_ART_WEIGHTS_MM, type AnnotationOverlay, type LineDrawingData } from './lineArt'

/** Items in mm from the page's top-left (pdfwrite's contract). */
export type PdfItem =
  | { kind: 'jpeg' | 'rgb'; data: number; w: number; h: number; rect: PdfRect }
  | { kind: 'path'; segs: number[][]; width_mm: number; dash?: number[]; gray: number; clip?: PdfRect }
  | { kind: 'text'; x: number; y: number; size_mm: number; bold: boolean; text: string; gray: number; align: 'left' | 'center' | 'right'; rotate?: number }
  | { kind: 'rect'; rect: PdfRect; stroke_mm?: number; fill_gray?: number; gray: number }
export interface PdfRect {
  x: number
  y: number
  w: number
  h: number
}
export interface PdfPageSpec {
  w_mm: number
  h_mm: number
  items: PdfItem[]
}
export interface PdfSpec {
  title: string
  pages: PdfPageSpec[]
}

/** Furniture items → PDF items (pure). */
export function furnitureToPdfItems(items: FurnitureItem[]): PdfItem[] {
  const out: PdfItem[] = []
  for (const it of items) {
    if (it.kind === 'line') {
      out.push({ kind: 'path', segs: [[it.x1, it.y1, it.x2, it.y2]], width_mm: it.widthMm, dash: it.dash, gray: it.gray })
    } else if (it.kind === 'text') {
      out.push({ kind: 'text', x: it.x, y: it.y, size_mm: it.sizeMm, bold: it.bold === true, text: it.text, gray: it.gray, align: it.align, ...(it.rotate !== undefined && it.rotate !== 0 ? { rotate: it.rotate } : {}) })
    } else {
      out.push({ kind: 'rect', rect: { x: it.x, y: it.y, w: it.w, h: it.h }, stroke_mm: it.strokeMm, fill_gray: it.fillGray, gray: it.gray ?? 0 })
    }
  }
  return out
}

/** A vector tile → PDF paths (mm), clipped to the image rect. */
export function vectorTileToPdfItems(
  drawing: LineDrawingData,
  tile: TileSpec,
  ratio: number,
  opts: { hiddenDashed: boolean; annotations: AnnotationOverlay | null },
): PdfItem[] {
  const r = tile.modelRect
  if (r === null) return []
  const k = ratio * 1000
  const img = tile.imageRectMm
  const toMm = (x: number, y: number): [number, number] => [img.x + (x - r.x) * k, img.y + (r.y + r.h - y) * k]
  const byKind: Record<number, number[][]> = { 0: [], 1: [], 2: [], 3: [], 4: [] }
  const c = drawing.coords
  const slack = 1 / k
  for (let i = 0; i < drawing.kinds.length; i++) {
    const kind = drawing.kinds[i]
    if (kind === 4 && !opts.hiddenDashed) continue
    const o = i * 4
    const minX = Math.min(c[o], c[o + 2])
    const maxX = Math.max(c[o], c[o + 2])
    const minY = Math.min(c[o + 1], c[o + 3])
    const maxY = Math.max(c[o + 1], c[o + 3])
    if (maxX < r.x - slack || minX > r.x + r.w + slack || maxY < r.y - slack || minY > r.y + r.h + slack) continue
    const a = toMm(c[o], c[o + 1])
    const b = toMm(c[o + 2], c[o + 3])
    byKind[kind].push([a[0], a[1], b[0], b[1]])
  }
  const clip: PdfRect = { x: img.x, y: img.y, w: img.w, h: img.h }
  const out: PdfItem[] = []
  const emit = (kind: number, widthMm: number, dash?: number[]): void => {
    if (byKind[kind].length === 0) return
    out.push({ kind: 'path', segs: byKind[kind], width_mm: widthMm, dash, gray: 0, clip })
  }
  emit(4, LINE_ART_WEIGHTS_MM.hidden, [1.5, 1])
  emit(2, LINE_ART_WEIGHTS_MM.soft)
  emit(0, LINE_ART_WEIGHTS_MM.hard)
  emit(1, LINE_ART_WEIGHTS_MM.silhouette)
  emit(3, LINE_ART_WEIGHTS_MM.section)
  if (opts.annotations !== null) {
    const a = opts.annotations
    const segs: number[][] = []
    for (let i = 0; i + 3 < a.segments.length; i += 4) {
      const p = toMm(a.segments[i], a.segments[i + 1])
      const q = toMm(a.segments[i + 2], a.segments[i + 3])
      segs.push([p[0], p[1], q[0], q[1]])
    }
    if (segs.length > 0) out.push({ kind: 'path', segs, width_mm: LINE_ART_WEIGHTS_MM.annotation, gray: 0, clip })
    for (const l of a.labels) {
      const p = toMm(l.x, l.y)
      if (p[0] < img.x || p[0] > img.x + img.w || p[1] < img.y || p[1] > img.y + img.h) continue
      // Baseline ≈ centre + 0.35 em for a 3 mm label.
      out.push({ kind: 'text', x: p[0], y: p[1] + 1.05, size_mm: 3, bold: false, text: l.text, gray: l.detached ? 0.3 : 0, align: 'center' })
    }
  }
  return out
}

/** Decode a page bitmap into raw RGB rows (top first) for the Flate path. */
async function blobToRgb(blob: Blob): Promise<{ data: Uint8Array; w: number; h: number }> {
  const bitmap = await createImageBitmap(blob)
  const canvas = document.createElement('canvas')
  canvas.width = bitmap.width
  canvas.height = bitmap.height
  const ctx = canvas.getContext('2d')
  if (ctx === null) throw new Error('2D canvas unavailable')
  ctx.drawImage(bitmap, 0, 0)
  const img = ctx.getImageData(0, 0, canvas.width, canvas.height)
  bitmap.close?.()
  const rgb = new Uint8Array(canvas.width * canvas.height * 3)
  const d = img.data
  for (let i = 0, j = 0; i < d.length; i += 4, j += 3) {
    rgb[j] = d[i]
    rgb[j + 1] = d[i + 1]
    rgb[j + 2] = d[i + 2]
  }
  canvas.width = 0
  return { data: rgb, w: img.width, h: img.height }
}

export interface PdfPageInput {
  tile: TileSpec
  furniture: FurnitureItem[]
  /** The page's layout (segments of a job may differ). */
  layout: PrintLayout
  /** Raster page bitmap (JPEG or PNG blob), or null for a vector page. */
  raster: Blob | null
  /** Vector drawing for this page (Line art), or null. */
  vector: { drawing: LineDrawingData; ratio: number; hiddenDashed: boolean; annotations: AnnotationOverlay | null } | null
  /** Furniture only (a cut-list page). */
  blank?: boolean
}

/**
 * Compose and build the PDF bytes for a job. Vector pages need `vector`;
 * raster pages need their blobs.
 */
export async function buildPrintPdf(args: { pages: PdfPageInput[]; title: string }): Promise<Uint8Array> {
  const blobs: Uint8Array[] = []
  const pages: PdfPageSpec[] = []
  for (const p of args.pages) {
    const items: PdfItem[] = []
    const r = p.tile.imageRectMm
    if (p.blank === true) {
      // furniture only
    } else if (p.vector !== null && p.raster === null) {
      items.push(...vectorTileToPdfItems(p.vector.drawing, p.tile, p.vector.ratio, { hiddenDashed: p.vector.hiddenDashed, annotations: p.vector.annotations }))
    } else if (p.raster !== null) {
      if (p.raster.type === 'image/jpeg') {
        blobs.push(new Uint8Array(await p.raster.arrayBuffer()))
        items.push({ kind: 'jpeg', data: blobs.length - 1, w: p.tile.imagePx.w, h: p.tile.imagePx.h, rect: { x: r.x, y: r.y, w: r.w, h: r.h } })
      } else {
        const rgb = await blobToRgb(p.raster)
        blobs.push(rgb.data)
        items.push({ kind: 'rgb', data: blobs.length - 1, w: rgb.w, h: rgb.h, rect: { x: r.x, y: r.y, w: r.w, h: r.h } })
      }
    }
    items.push(...furnitureToPdfItems(p.furniture))
    pages.push({ w_mm: p.layout.page.paper.w, h_mm: p.layout.page.paper.h, items })
  }
  const spec: PdfSpec = { title: args.title, pages }
  return build_pdf(JSON.stringify(spec), blobs)
}
