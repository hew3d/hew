/**
 * Vector Line art (docs/design/printing.md §7b): the kernel-side hidden-line
 * drawing (`Scene.line_drawing`, crates/hlr) composed into per-page SVG at
 * drawing scale, plus the app's annotation overlay. Used by the print dialog
 * for Line-art pages (raster stays the fallback), by SVG export, and by Save
 * PDF… (segments straight into PDF paths).
 */
import type { Scene as WasmScene } from '../wasm/loader'
import type { PrintCameraSpec } from '../viewport/printPass'
import type { PrintLayout, TileSpec } from './layout'
import type { AnnotationDrawing } from '../viewport/SceneRenderer'

export type LineKind = 0 | 1 | 2 | 3 | 4 // hard, silhouette, soft, section, hidden

export interface LineDrawingData {
  /** [ax, ay, bx, by, …] view-plane metres, y up, origin at the print centre. */
  coords: Float64Array
  kinds: Uint8Array
  sids: BigUint64Array
  bounds: number[] | null
}

/** Annotation overlay in the same view-plane frame. */
export interface AnnotationOverlay {
  /** [ax, ay, bx, by, …] */
  segments: number[]
  labels: {
    x: number
    y: number
    text: string
    detached: boolean
    /** Radial labels: unit direction in the view plane to push the label
     * along, past its anchor, so it clears its leader (see `labelAnchor`). */
    push?: { dx: number; dy: number }
  }[]
  /** Arrowheads in the view plane: tip, unit direction back along the line,
   * and the longest the head may be (model metres, after foreshortening).
   * Sized at emit time from the paper label size. */
  arrows: { x: number; y: number; dx: number; dy: number; maxLen: number }[]
}

export interface LineArtRequest {
  scene: WasmScene
  camera: Extract<PrintCameraSpec, { kind: 'ortho' }>
  section: { origin: [number, number, number]; normal: [number, number, number] } | null
  hidden: { objects: bigint[]; instances: bigint[] }
  /** `hidden` REPLACES the document's hidden state (a Scene's sets) instead of adding to it. */
  hiddenAuthoritative?: boolean
  only: { objects: bigint[]; instances: bigint[] } | null
  includeHidden: boolean
}

export type LineArtResult = { kind: 'ok'; drawing: LineDrawingData } | { kind: 'too-complex'; message: string } | { kind: 'error'; message: string }

/** Ask the kernel for the drawing. Never throws: complexity and errors come
 * back typed so the caller can fall back to raster. */
export function requestLineDrawing(req: LineArtRequest): LineArtResult {
  const { camera } = req
  const range = Math.max(camera.depth.max - camera.depth.min, 1e-3)
  const pad = Math.max(range * 0.1, 0.05)
  const dir = camera.dir
  const eye: [number, number, number] = [
    camera.center[0] + dir[0] * (camera.depth.min - pad),
    camera.center[1] + dir[1] * (camera.depth.min - pad),
    camera.center[2] + dir[2] * (camera.depth.min - pad),
  ]
  const camJson = JSON.stringify({ eye, target: camera.center, up: camera.up, projection: 'parallel' })
  const optsJson = JSON.stringify({
    section: req.section === null ? null : { origin: req.section.origin, normal: req.section.normal },
    include_hidden: req.includeHidden,
    include_soft: false,
    only: req.only !== null,
    hidden_authoritative: req.hiddenAuthoritative === true,
  })
  try {
    const d = req.scene.line_drawing(
      camJson,
      optsJson,
      new BigUint64Array(req.hidden.objects),
      new BigUint64Array(req.hidden.instances),
      new BigUint64Array(req.only?.objects ?? []),
      new BigUint64Array(req.only?.instances ?? []),
    )
    const coords = d.coords()
    const kinds = d.kinds()
    const sids = d.sids()
    const bounds = d.bounds()
    d.free?.()
    return { kind: 'ok', drawing: { coords, kinds, sids, bounds: bounds.length === 4 ? Array.from(bounds) : null } }
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e)
    if (message.startsWith('TooComplex')) return { kind: 'too-complex', message }
    return { kind: 'error', message }
  }
}

/** Stroke weights, mm (matches the raster pass's edge weights). */
export const LINE_ART_WEIGHTS_MM = { hard: 0.35, silhouette: 0.35, section: 0.5, soft: 0.18, hidden: 0.25, annotation: 0.25 } as const

const LABEL_MM = 3.0
/** Arrowhead length on paper: 0.9 of the label height, as on screen. */
const ARROW_MM = 0.9 * LABEL_MM
const ARROW_WIDTH_FRAC = 0.35

/** The two wing segments (model units) of each arrowhead at `lenPerMm` model
 * units per paper millimeter, as `M..L..` path text through `fmt`. */
export function arrowheadPath(
  arrows: NonNullable<AnnotationOverlay['arrows']>,
  lenPerMm: number,
  fmt: (x: number, y: number) => string,
): string {
  let d = ''
  for (const a of arrows) {
    const len = Math.min(ARROW_MM * lenPerMm, a.maxLen)
    if (!(len > 0)) continue
    const w = len * ARROW_WIDTH_FRAC
    const bx = a.x + a.dx * len
    const by = a.y + a.dy * len
    d += `M${fmt(a.x, a.y)}L${fmt(bx - a.dy * w, by + a.dx * w)}M${fmt(a.x, a.y)}L${fmt(bx + a.dy * w, by - a.dx * w)}`
  }
  return d
}

/** Rough paper width of a label: an average glyph is about 0.55 of the
 * font size wide. The page has no text metrics, so this stands in. */
function labelWidthMm(text: string): number {
  return text.length * 0.55 * LABEL_MM
}

/**
 * Where a label is drawn, in view-plane model units. A pushed (radial) label
 * moves past its anchor along `push` until its upright paper box clears the
 * anchor, plus the same 0.4-of-height padding the screen uses, so a printed
 * radius reads beside its leader end rather than on top of it.
 */
export function labelAnchor(l: AnnotationOverlay['labels'][number], lenPerMm: number): { x: number; y: number } {
  if (l.push === undefined) return { x: l.x, y: l.y }
  const halfW = labelWidthMm(l.text) / 2
  const halfH = LABEL_MM / 2
  // Distance from the box centre to its edge along the push direction.
  const toEdge = Math.min(
    Math.abs(l.push.dx) > 1e-9 ? halfW / Math.abs(l.push.dx) : Infinity,
    Math.abs(l.push.dy) > 1e-9 ? halfH / Math.abs(l.push.dy) : Infinity,
  )
  const mmPush = toEdge + 0.4 * LABEL_MM
  return { x: l.x + l.push.dx * mmPush * lenPerMm, y: l.y + l.push.dy * mmPush * lenPerMm }
}

function num(v: number): string {
  // Six decimals: a micron when the unit is metres (tile SVGs), a nanometre
  // when it is millimetres (the standalone document) — never a visible drift.
  const s = v.toFixed(6)
  const t = s.replace(/\.?0+$/, '')
  return t === '' || t === '-0' ? '0' : t
}

function esc(t: string): string {
  return t.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

/**
 * The SVG for one tile: an image-rect-sized document (mm) whose viewBox is
 * the tile's model rectangle, y flipped so drawing coordinates (metres, y up)
 * land the right way up. Paths group by kind; strokes are given in metres
 * (mm ÷ scale) so they print at the intended weight.
 */
// oxlint-disable-next-line lingui/no-unlocalized-strings -- CSS font stack
const ANNOTATION_FONT_FAMILY = "-apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, 'IBM Plex Sans Arabic', Cairo, 'Noto Sans Arabic', sans-serif"

export function lineArtSvgForTile(
  drawing: LineDrawingData,
  tile: TileSpec,
  layout: PrintLayout,
  ratio: number,
  opts: { hiddenDashed: boolean; annotations: AnnotationOverlay | null },
): string {
  const r = tile.modelRect
  if (r === null) return ''
  const wMm = tile.imageRectMm.w
  const hMm = tile.imageRectMm.h
  const k = ratio * 1000 // mm per metre
  const mm = (v: number): number => v / k // stroke mm → metres in user units
  const parts: string[] = []
  parts.push(
    `<svg xmlns="http://www.w3.org/2000/svg" width="${num(wMm)}mm" height="${num(hMm)}mm" viewBox="${num(r.x)} ${num(-(r.y + r.h))} ${num(r.w)} ${num(r.h)}" preserveAspectRatio="none">`,
  )
  parts.push(`<g transform="scale(1,-1)" fill="none" stroke="#000" stroke-linecap="round" stroke-linejoin="round">`)
  const byKind: Record<number, string[]> = { 0: [], 1: [], 2: [], 3: [], 4: [] }
  const c = drawing.coords
  for (let i = 0; i < drawing.kinds.length; i++) {
    const kind = drawing.kinds[i]
    if (kind === 4 && !opts.hiddenDashed) continue
    const o = i * 4
    // Cheap culling: skip segments entirely outside the tile (with slack).
    const minX = Math.min(c[o], c[o + 2])
    const maxX = Math.max(c[o], c[o + 2])
    const minY = Math.min(c[o + 1], c[o + 3])
    const maxY = Math.max(c[o + 1], c[o + 3])
    const slack = mm(1)
    if (maxX < r.x - slack || minX > r.x + r.w + slack || maxY < r.y - slack || minY > r.y + r.h + slack) continue
    // oxlint-disable-next-line lingui/no-unlocalized-strings -- SVG markup, not copy
    byKind[kind].push(`M${num(c[o])} ${num(c[o + 1])}L${num(c[o + 2])} ${num(c[o + 3])}`)
  }
  const emit = (kind: number, cls: string, widthMm: number, dash?: string): void => {
    const d = byKind[kind]
    if (d.length === 0) return
    parts.push(
      // oxlint-disable-next-line lingui/no-unlocalized-strings -- SVG markup, not copy
      `<path class="${cls}" stroke-width="${num(mm(widthMm))}"${dash !== undefined ? ` stroke-dasharray="${dash}"` : ''} d="${d.join('')}"/>`,
    )
  }
  emit(4, 'hidden', LINE_ART_WEIGHTS_MM.hidden, `${num(mm(1.5))} ${num(mm(1))}`)
  emit(2, 'soft', LINE_ART_WEIGHTS_MM.soft)
  emit(0, 'hard', LINE_ART_WEIGHTS_MM.hard)
  emit(1, 'silhouette', LINE_ART_WEIGHTS_MM.silhouette)
  emit(3, 'section', LINE_ART_WEIGHTS_MM.section)
  if (opts.annotations !== null) {
    const a = opts.annotations
    if (a.segments.length >= 4) {
      let d = ''
      for (let i = 0; i + 3 < a.segments.length; i += 4) {
        d += `M${num(a.segments[i])} ${num(a.segments[i + 1])}L${num(a.segments[i + 2])} ${num(a.segments[i + 3])}`
      }
      parts.push(`<path class="annotation" stroke-width="${num(mm(LINE_ART_WEIGHTS_MM.annotation))}" d="${d}"/>`)
    }
    if (a.arrows !== undefined && a.arrows.length > 0) {
      const d = arrowheadPath(a.arrows, mm(1), (x, y) => `${num(x)} ${num(y)}`)
      if (d !== '') parts.push(`<path class="annotation-arrow" stroke-width="${num(mm(LINE_ART_WEIGHTS_MM.annotation))}" d="${d}"/>`)
    }
    for (const l of a.labels) {
      // Labels are drawn upright in paper space: undo the y flip locally.
      const at = labelAnchor(l, mm(1))
      parts.push(
        `<text class="annotation-label" transform="translate(${num(at.x)} ${num(at.y)}) scale(1,-1)" font-family="${ANNOTATION_FONT_FAMILY}" font-size="${num(mm(LABEL_MM))}" text-anchor="middle" dominant-baseline="middle" fill="${l.detached ? '#b3261e' : '#000'}" stroke="#fff" stroke-width="${num(mm(0.6))}" paint-order="stroke" stroke-linejoin="round">${esc(l.text)}</text>`,
      )
    }
  }
  parts.push('</g></svg>')
  return parts.join('')
}

/** Project world annotation drawing into the view-plane frame of an ortho print camera. */
export function projectAnnotationOverlay(
  world: AnnotationDrawing,
  camera: Extract<PrintCameraSpec, { kind: 'ortho' }>,
): AnnotationOverlay {
  const dir = norm(camera.dir)
  const up = norm(camera.up)
  // Same basis as printPass.viewPlaneBasis / hlr::Frame.
  const back: [number, number, number] = [-dir[0], -dir[1], -dir[2]]
  let right = cross(up, back)
  if (len(right) < 1e-9) {
    const seed: [number, number, number] = Math.abs(dir[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0]
    const k = dot(seed, dir)
    right = [seed[0] - dir[0] * k, seed[1] - dir[1] * k, seed[2] - dir[2] * k]
  }
  right = norm(right)
  const upV = norm(cross(back, right))
  const c = camera.center
  const proj = (p: [number, number, number]): [number, number] => {
    const rel: [number, number, number] = [p[0] - c[0], p[1] - c[1], p[2] - c[2]]
    return [dot(rel, right), dot(rel, upV)]
  }
  const segments: number[] = []
  for (let i = 0; i + 5 < world.segments.length; i += 6) {
    const a = proj([world.segments[i], world.segments[i + 1], world.segments[i + 2]])
    const b = proj([world.segments[i + 3], world.segments[i + 4], world.segments[i + 5]])
    segments.push(a[0], a[1], b[0], b[1])
  }
  const labels = world.labels.map((l) => {
    const p = proj(l.position)
    const out: AnnotationOverlay['labels'][number] = { x: p[0], y: p[1], text: l.text, detached: l.detached }
    if (l.pushDir !== undefined) {
      const q = proj([l.position[0] + l.pushDir[0], l.position[1] + l.pushDir[1], l.position[2] + l.pushDir[2]])
      const vx = q[0] - p[0]
      const vy = q[1] - p[1]
      const n = Math.hypot(vx, vy)
      // Seen end-on there is no screen direction to push along.
      if (n > 1e-9) out.push = { dx: vx / n, dy: vy / n }
    }
    return out
  })
  const arrows: NonNullable<AnnotationOverlay['arrows']> = []
  for (const a of world.arrows) {
    const t = proj(a.tip)
    const e = proj([a.tip[0] + a.dir[0] * a.maxLen, a.tip[1] + a.dir[1] * a.maxLen, a.tip[2] + a.dir[2] * a.maxLen])
    const vx = e[0] - t[0]
    const vy = e[1] - t[1]
    const l = Math.hypot(vx, vy)
    if (l < 1e-9) continue // seen end-on: no readable head
    arrows.push({ x: t[0], y: t[1], dx: vx / l, dy: vy / l, maxLen: l })
  }
  return { segments, labels, arrows }
}

function cross(a: [number, number, number], b: [number, number, number]): [number, number, number] {
  return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
function dot(a: [number, number, number], b: [number, number, number]): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
function len(a: [number, number, number]): number {
  return Math.hypot(a[0], a[1], a[2])
}
function norm(a: [number, number, number]): [number, number, number] {
  const l = len(a) || 1
  return [a[0] / l, a[1] / l, a[2] / l]
}

/**
 * A standalone SVG document of the whole drawing at scale `ratio`
 * (paper/model), true size in millimetres, y down, with a margin — the
 * `File ▸ Export… ▸ SVG` payload (docs/design/printing.md §7b). Same stroke
 * weights and classes as the print page.
 */
export function lineArtSvgDocument(drawing: LineDrawingData, ratio: number, opts: { hiddenDashed: boolean; marginMm?: number; annotations?: AnnotationOverlay | null }): string {
  const k = ratio * 1000
  const margin = opts.marginMm ?? 5
  const b = drawing.bounds ?? [0, 0, 0, 0]
  const x0 = b[0] * k - margin
  const y0 = -b[3] * k - margin
  const w = (b[2] - b[0]) * k + 2 * margin
  const h = (b[3] - b[1]) * k + 2 * margin
  const parts: string[] = []
  parts.push(`<svg xmlns="http://www.w3.org/2000/svg" width="${num(w)}mm" height="${num(h)}mm" viewBox="${num(x0)} ${num(y0)} ${num(w)} ${num(h)}">`)
  parts.push(`<g fill="none" stroke="#000" stroke-linecap="round" stroke-linejoin="round">`)
  const byKind: Record<number, string[]> = { 0: [], 1: [], 2: [], 3: [], 4: [] }
  const c = drawing.coords
  for (let i = 0; i < drawing.kinds.length; i++) {
    const kind = drawing.kinds[i]
    if (kind === 4 && !opts.hiddenDashed) continue
    const o = i * 4
    // oxlint-disable-next-line lingui/no-unlocalized-strings -- SVG markup, not copy
    byKind[kind].push(`M${num(c[o] * k)} ${num(-c[o + 1] * k)}L${num(c[o + 2] * k)} ${num(-c[o + 3] * k)}`)
  }
  const emit = (kind: number, cls: string, widthMm: number, dash?: string): void => {
    const d = byKind[kind]
    if (d.length === 0) return
    // oxlint-disable-next-line lingui/no-unlocalized-strings -- SVG markup, not copy
    parts.push(`<path class="${cls}" stroke-width="${num(widthMm)}"${dash !== undefined ? ` stroke-dasharray="${dash}"` : ''} d="${d.join('')}"/>`)
  }
  emit(4, 'hidden', LINE_ART_WEIGHTS_MM.hidden, '1.5 1')
  emit(2, 'soft', LINE_ART_WEIGHTS_MM.soft)
  emit(0, 'hard', LINE_ART_WEIGHTS_MM.hard)
  emit(1, 'silhouette', LINE_ART_WEIGHTS_MM.silhouette)
  emit(3, 'section', LINE_ART_WEIGHTS_MM.section)
  const a = opts.annotations ?? null
  if (a !== null) {
    if (a.segments.length >= 4) {
      let d = ''
      for (let i = 0; i + 3 < a.segments.length; i += 4) d += `M${num(a.segments[i] * k)} ${num(-a.segments[i + 1] * k)}L${num(a.segments[i + 2] * k)} ${num(-a.segments[i + 3] * k)}`
      parts.push(`<path class="annotation" stroke-width="${num(LINE_ART_WEIGHTS_MM.annotation)}" d="${d}"/>`)
    }
    if (a.arrows !== undefined && a.arrows.length > 0) {
      // Document units are paper millimeters; model metres scale by k.
      const d = arrowheadPath(a.arrows.map((r) => ({ ...r, x: r.x * k, y: -r.y * k, dy: -r.dy, maxLen: r.maxLen * k })), 1, (x, y) => `${num(x)} ${num(y)}`)
      if (d !== '') parts.push(`<path class="annotation-arrow" stroke-width="${num(LINE_ART_WEIGHTS_MM.annotation)}" d="${d}"/>`)
    }
    for (const l of a.labels) {
      const at = labelAnchor(l, 1 / k)
      parts.push(`<text class="annotation-label" x="${num(at.x * k)}" y="${num(-at.y * k)}" font-family="${ANNOTATION_FONT_FAMILY}" font-size="${num(LABEL_MM)}" text-anchor="middle" dominant-baseline="middle" fill="${l.detached ? '#b3261e' : '#000'}" stroke="#fff" stroke-width="0.6" paint-order="stroke">${esc(l.text)}</text>`)
    }
  }
  parts.push('</g></svg>')
  return parts.join('\n')
}
