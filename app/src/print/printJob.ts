/**
 * Print job planning (docs/design/printing.md §4–§7): turns the dialog's
 * options + the live viewport into a `PrintPlan` — the page layout, one
 * render request per page, and the furniture for each page. Pure apart from
 * the three `ViewportPrintSource` reads, so the dialog can re-plan on every
 * option change cheaply and the tests can drive it with a fake source.
 */
import { LENGTH_SYSTEM_OF, type LengthFormat } from '../settings/units'
import type { PrintCameraSpec, PrintPageRequest, ViewPlaneExtent } from '../viewport/printPass'
import { pageFurniture, type FurnitureContext, type FurnitureItem } from './furniture'
import {
  DEFAULT_TITLE_BLOCK_MM,
  OVERLAP_MM,
  MANY_PAGES_WARNING,
  PRINT_DPI,
  fitRatioFor,
  layoutPrint,
  type LayoutInput,
  type PrintLayout,
  type PrintMode,
  type TileSpec,
} from './layout'
import { MARGIN_MM, paperSize, type MarginPreset, type Orientation, type PaperSpec } from './paper'
import { fitScale, scaleDisplay, scaleRatio, type PrintScale } from './scale'

export type PrintStyle = 'shaded' | 'lineart'
export type PrintExtentKind = 'model' | 'selection' | 'view'
export type PrintViewKind = 'current' | 'top' | 'bottom' | 'front' | 'back' | 'left' | 'right' | 'iso'
export type PrintPagesMode = 'current' | 'each_scene'
/** Standard mode: print the viewport's own framing, or re-frame the
 * visible model to fill the page. */
export type PrintZoom = 'current' | 'fit'

export interface PrintOptions {
  mode: PrintMode
  paper: PaperSpec
  orientation: Orientation
  margin: MarginPreset
  style: PrintStyle
  includeDimensions: boolean
  includeGuides: boolean
  /** Standard only. */
  includeGridAxes: boolean
  titleBlock: boolean
  /** Scaled only. */
  marks: boolean
  overlap: boolean
  scaleBar: boolean
  /** Line art only: draw hidden lines dashed. */
  hiddenDashed: boolean
  /** Append the cut list (every part with L × W × H) as extra pages. */
  cutList: boolean
  scale: PrintScale
  extent: PrintExtentKind
  view: PrintViewKind
  nudgeMm: { dx: number; dy: number }
  /** Current view, or one page (Standard) / tile set (Scaled) per Scene. */
  pages: PrintPagesMode
  /** Standard only. */
  zoom: PrintZoom
}

export const DEFAULT_SCALE_METRIC: PrintScale = { paperMeters: 1, modelMeters: 10, label: '1:10', kind: 'preset' }
export const DEFAULT_SCALE_IMPERIAL: PrintScale = { paperMeters: 0.0254, modelMeters: 0.3048, label: '1" = 1\'', kind: 'preset' }

export function defaultPrintOptions(format: LengthFormat, paper: PaperSpec): PrintOptions {
  return {
    mode: 'standard',
    paper,
    orientation: 'auto',
    margin: 'normal',
    style: 'shaded',
    includeDimensions: true,
    includeGuides: false,
    includeGridAxes: false,
    titleBlock: true,
    marks: true,
    overlap: true,
    scaleBar: true,
    hiddenDashed: false,
    cutList: false,
    scale: LENGTH_SYSTEM_OF[format] === 'metric' ? DEFAULT_SCALE_METRIC : DEFAULT_SCALE_IMPERIAL,
    extent: 'model',
    view: 'current',
    nudgeMm: { dx: 0, dy: 0 },
    pages: 'current',
    zoom: 'current',
  }
}

/** The subset of `ViewportApi` planning needs. */
export interface ViewportPrintSource {
  getPrintView: () => {
    dir: [number, number, number]
    up: [number, number, number]
    target: [number, number, number]
    projection: 'perspective' | 'parallel'
    orthoSize: { w: number; h: number }
    aspect: number
  }
  computePrintExtent: (
    dir: [number, number, number],
    up: [number, number, number],
    opts: {
      includeSketches: boolean
      includeAnnotations: boolean
      restrictTo: { objects: Set<bigint>; instances: Set<bigint> } | null
      hiddenOverride?: { objects: bigint[]; instances: bigint[] } | null
    },
  ) => ViewPlaneExtent
  getSelectedIds: () => { objects: bigint[]; instances: bigint[] }
}

export interface PrintPlanContext {
  documentName: string
  /** Active Scene name when the view is that Scene (undrifted), else null. */
  sceneName: string | null
  unitFormat: LengthFormat
  /** Local date text for the title block. */
  dateText: string
  /** Name of the single selected part when Extent = Selection names one
   * ("Café table — Tabletop" in the title block); null otherwise. */
  selectionName?: string | null
}

export interface PlannedPage {
  tile: TileSpec
  request: PrintPageRequest
  furniture: FurnitureItem[]
}

export interface PrintPlan {
  layout: PrintLayout
  pages: PlannedPage[]
  /** Nothing visible to print (empty document / empty selection). */
  empty: boolean
  /** Human warnings for the summary line. */
  warnings: string[]
  /** Perspective viewport being printed to scale (parallel print camera). */
  projectionNote: boolean
  jobTitle: string
  viewName: string
  scaleText: string | null
  restrictTo: { objects: Set<bigint>; instances: Set<bigint> } | null
  /** Ratio at which the extent fits one page — for the Fit action. */
  fitRatio: number | null
  /** The Scene this plan prints (Each Scene), or null for the live view. */
  scene: SceneSource | null
}

/** Eye direction (target → camera) per standard view; up is world +Z. Top
 * and bottom are EXACT axis views (a plan must be a true projection); the
 * degenerate up is resolved by the print basis rule (world +X to the right,
 * +Y up on a plan) — no pole tilt here, unlike the interactive viewport. */
const STANDARD_EYE: Record<Exclude<PrintViewKind, 'current'>, [number, number, number]> = {
  top: [0, 0, 1],
  bottom: [0, 0, -1],
  front: [0, -1, 0],
  back: [0, 1, 0],
  right: [1, 0, 0],
  left: [-1, 0, 0],
  iso: [1, -1, 1],
}

export const VIEW_LABEL: Record<PrintViewKind, string> = {
  current: 'Current view',
  top: 'Top',
  bottom: 'Bottom',
  front: 'Front',
  back: 'Back',
  left: 'Left',
  right: 'Right',
  iso: 'Iso',
}

function norm(v: [number, number, number]): [number, number, number] {
  const l = Math.hypot(v[0], v[1], v[2]) || 1
  return [v[0] / l, v[1] / l, v[2] / l]
}

/** Direction + up for the print camera. */
export function printViewDirection(view: PrintViewKind, live: ReturnType<ViewportPrintSource['getPrintView']>): { dir: [number, number, number]; up: [number, number, number] } {
  if (view === 'current') return { dir: live.dir, up: live.up }
  const eye = STANDARD_EYE[view]
  return { dir: norm([-eye[0], -eye[1], -eye[2]]), up: [0, 0, 1] }
}

/** The document's display name for paper: no file extension ("Trestle
 * Table", not "Trestle Table.hew"), "Untitled" when empty. */
export function printDocName(documentName: string): string {
  const stripped = documentName.trim().replace(/\.hew$/i, '')
  return stripped === '' ? 'Untitled' : stripped
}

export function jobTitleFor(ctx: PrintPlanContext, opts: PrintOptions, viewName: string): string {
  const base = printDocName(ctx.documentName)
  return opts.mode === 'scaled' ? `${base} — ${scaleDisplay(opts.scale)}` : `${base} — ${viewName}`
}

/**
 * Plan the print. Reads the viewport (direction, extents, selection) and
 * lays out pages; renders nothing.
 */
/** One Scene as a print segment (Each Scene): its resolved camera pose,
 * hidden sets, and section plane — each optional (not captured). */
export interface SceneSource {
  sid: number
  name: string
  camera: { projection: 'perspective' | 'parallel'; eye: [number, number, number]; target: [number, number, number]; up: [number, number, number]; fovDeg: number } | null
  hidden: { objects: bigint[]; instances: bigint[] } | null
  /** `undefined` = not captured (live plane applies); `null` = captured with no plane. */
  section: { origin: [number, number, number]; normal: [number, number, number]; active: boolean } | null | undefined
}

/** Per-segment overrides threaded through `planPrint`. */
export interface PlanOverrides {
  scene: SceneSource | null
  /** 1-based number of this segment's first page and the job's total. */
  pageNumberOffset: number
  pageTotal: number
}

export function planPrint(opts: PrintOptions, source: ViewportPrintSource, ctx: PrintPlanContext, dpi: number = PRINT_DPI, overrides: PlanOverrides | null = null): PrintPlan {
  const liveView = source.getPrintView()
  const scene = overrides?.scene ?? null
  // A Scene with a captured camera prints from that pose; otherwise the
  // live view (a hidden-only Scene, say).
  const live: ReturnType<ViewportPrintSource['getPrintView']> = scene?.camera
    ? (() => {
        const c = scene.camera
        const d: [number, number, number] = [c.target[0] - c.eye[0], c.target[1] - c.eye[1], c.target[2] - c.eye[2]]
        const l = Math.hypot(d[0], d[1], d[2]) || 1
        const dir: [number, number, number] = [d[0] / l, d[1] / l, d[2] / l]
        const dist = Math.max(l, 1e-6)
        const h = 2 * dist * Math.tan((c.fovDeg * Math.PI) / 360)
        return { dir, up: c.up, target: c.target, projection: c.projection, orthoSize: { w: h * liveView.aspect, h }, aspect: liveView.aspect }
      })()
    : liveView
  const hiddenOverride = scene?.hidden ?? null
  const paper = paperSize(opts.paper)
  const marginMm = MARGIN_MM[opts.margin]
  const titleBlockMm = opts.titleBlock ? DEFAULT_TITLE_BLOCK_MM : 0
  const warnings: string[] = []
  // Standard mode always prints the live view (a Scene's name when one is
  // active and undrifted); the View picker only applies to Scaled mode.
  const viewName = scene !== null ? scene.name : opts.mode === 'standard' || opts.view === 'current' ? (ctx.sceneName ?? VIEW_LABEL.current) : VIEW_LABEL[opts.view]

  let restrictTo: PrintPlan['restrictTo'] = null
  if (opts.mode === 'scaled' && opts.extent === 'selection') {
    const sel = source.getSelectedIds()
    restrictTo = { objects: new Set(sel.objects), instances: new Set(sel.instances) }
  }

  const EXTENT_LABEL: Record<PrintExtentKind, string> = { model: 'Model', selection: 'Selection', view: 'Current view' }
  const docName = printDocName(ctx.documentName)
  // Title-block name: "Café table — Tabletop" when the extent is one named
  // selection; second line: the Scene name (Each Scene), "Top view · Model"
  // (Scaled), or "Perspective view" (Standard).
  const titleName = opts.mode === 'scaled' && opts.extent === 'selection' && ctx.selectionName ? `${docName} — ${ctx.selectionName}` : docName
  const subtitle =
    scene !== null
      ? scene.name
      : opts.mode === 'standard'
        ? (ctx.sceneName ?? (live.projection === 'parallel' ? 'Parallel view' : 'Perspective view'))
        : opts.extent === 'view' && opts.view === 'current'
          ? VIEW_LABEL.current
          : `${opts.view === 'current' ? (ctx.sceneName ?? VIEW_LABEL.current) : `${VIEW_LABEL[opts.view]} view`} · ${EXTENT_LABEL[opts.extent]}`
  const furnitureBase = (scaleText: string | null): FurnitureContext => ({
    documentName: titleName,
    subtitle,
    scaleText,
    dateText: ctx.dateText,
    marks: opts.marks,
    titleBlock: opts.titleBlock,
    scaleBar: opts.scaleBar,
    pageNumberOffset: overrides?.pageNumberOffset,
    pageTotal: overrides?.pageTotal,
  })

  if (opts.mode === 'standard') {
    // Zoom: Fit — the page image takes the drawing area's own aspect (the
    // model fills it), Auto orientation follows the model's projected
    // extent, and the camera re-frames at render time (printPass `fit`).
    const fit = opts.zoom === 'fit'
    let orientation = opts.orientation
    let viewportAspect = live.aspect
    if (fit) {
      const ext = source.computePrintExtent(live.dir, live.up, { includeSketches: true, includeAnnotations: opts.includeDimensions, restrictTo: null, hiddenOverride })
      const modelAspect = ext.empty || !(ext.rect.h > 0) ? 1 : ext.rect.w / ext.rect.h
      if (orientation === 'auto') orientation = modelAspect > 1 ? 'landscape' : 'portrait'
      const probe = layoutPrint({ mode: 'standard', paper, orientation, marginMm, titleBlockMm, overlapMm: 0, dpi, viewportAspect: 1 })
      viewportAspect = probe.page.drawing.w / Math.max(probe.page.drawing.h, 1e-6)
    }
    const layout = layoutPrint({
      mode: 'standard',
      paper,
      orientation,
      marginMm,
      titleBlockMm,
      overlapMm: 0,
      dpi,
      viewportAspect,
    })
    const tile = layout.tiles[0]
    const fc = furnitureBase(null)
    return {
      layout,
      pages: [
        {
          tile,
          request: {
            camera: scene?.camera ? { kind: 'pose', projection: scene.camera.projection, eye: scene.camera.eye, target: scene.camera.target, up: scene.camera.up, fovDeg: scene.camera.fovDeg, fit } : { kind: 'live', fit },
            widthPx: tile.imagePx.w,
            heightPx: tile.imagePx.h,
          },
          furniture: pageFurniture(layout, tile, fc),
        },
      ],
      empty: false,
      warnings,
      projectionNote: false,
      jobTitle: jobTitleFor(ctx, opts, viewName),
      viewName,
      scaleText: null,
      restrictTo,
      fitRatio: null,
      scene,
    }
  }

  // ---- scaled
  const { dir, up } = printViewDirection(opts.view, live)
  const modelExtent = source.computePrintExtent(dir, up, {
    includeSketches: restrictTo === null,
    includeAnnotations: opts.includeDimensions,
    restrictTo,
    hiddenOverride,
  })
  let extent: ViewPlaneExtent = modelExtent
  if (opts.extent === 'view' && opts.view === 'current') {
    // The live parallel frustum, centred on the orbit target; depth range
    // re-expressed relative to the target.
    const dOff =
      (modelExtent.center[0] - live.target[0]) * dir[0] +
      (modelExtent.center[1] - live.target[1]) * dir[1] +
      (modelExtent.center[2] - live.target[2]) * dir[2]
    extent = {
      center: live.target,
      rect: { x: -live.orthoSize.w / 2, y: -live.orthoSize.h / 2, w: live.orthoSize.w, h: live.orthoSize.h },
      depth: { min: modelExtent.depth.min + dOff, max: modelExtent.depth.max + dOff },
      empty: modelExtent.empty,
    }
  }
  const ratio = scaleRatio(opts.scale)
  const layoutInput: LayoutInput = {
    mode: 'scaled',
    paper,
    orientation: opts.orientation,
    marginMm,
    titleBlockMm,
    overlapMm: opts.overlap ? OVERLAP_MM[opts.margin] : 0,
    dpi,
    ratio,
    extent: extent.rect,
    nudgeMm: opts.nudgeMm,
    scaleBarSystem: opts.scaleBar ? LENGTH_SYSTEM_OF[ctx.unitFormat] : null,
  }
  const layout = layoutPrint(layoutInput)
  const fitRatio = fitRatioFor({ ...layoutInput, extent: extent.rect })
  const scaleText = scaleDisplay(opts.scale)
  const fc = furnitureBase(scaleText)
  const pages: PlannedPage[] = layout.tiles.map((tile) => {
    const camera: PrintCameraSpec = {
      kind: 'ortho',
      center: extent.center,
      dir,
      up,
      rect: tile.modelRect!,
      depth: extent.depth,
    }
    return {
      tile,
      request: { camera, widthPx: tile.imagePx.w, heightPx: tile.imagePx.h },
      furniture: pageFurniture(layout, tile, fc),
    }
  })
  if (layout.tiles.length > MANY_PAGES_WARNING) warnings.push(`${layout.tiles.length} pages — check the scale`)
  if (layout.dpi < dpi) warnings.push(`Rendering at ${layout.dpi} dpi for this sheet size`)
  return {
    layout,
    pages,
    empty: extent.empty,
    warnings,
    projectionNote: live.projection === 'perspective',
    jobTitle: jobTitleFor(ctx, opts, viewName),
    viewName,
    scaleText,
    restrictTo,
    fitRatio,
    scene,
  }
}

/** The Fit action's result for the current plan (largest standard scale on
 * one page). */
export function fitScaleFor(plan: PrintPlan, format: LengthFormat): PrintScale | null {
  if (plan.fitRatio === null) return null
  return fitScale(plan.fitRatio, LENGTH_SYSTEM_OF[format])
}

/** A whole print job: one plan (Current view) or one per Scene (Each Scene). */
export interface PrintJob {
  plans: PrintPlan[]
  /** Every page across plans, in order, with its plan index. */
  pages: { plan: number; page: PlannedPage }[]
  totalPages: number
  empty: boolean
  warnings: string[]
  projectionNote: boolean
  jobTitle: string
}

/**
 * Plan the job. With `pages: 'each_scene'` and Scenes present, every Scene
 * becomes a segment (its camera pose, hidden sets, section); Auto
 * orientation is resolved once from the first segment so all pages share a
 * sheet. Page numbers run across the whole job.
 */
export function planPrintJob(opts: PrintOptions, source: ViewportPrintSource, ctx: PrintPlanContext, scenes: SceneSource[], dpi: number = PRINT_DPI, extraPages = 0): PrintJob {
  const segments: (SceneSource | null)[] = opts.pages === 'each_scene' && scenes.length > 0 ? scenes : [null]
  const single = segments.length === 1 && segments[0] === null
  const overridesFor = (sc: SceneSource | null, offset: number, total: number): PlanOverrides | null =>
    single && extraPages === 0 ? null : { scene: sc, pageNumberOffset: offset, pageTotal: total }
  // Pass 1: sizes (and, under Auto, the sheet orientation the first segment
  // picks — every segment shares it so the job is one paper setup).
  let sharedOpts = opts
  const prelim: PrintPlan[] = []
  for (const sc of segments) {
    const plan = planPrint(sharedOpts, source, ctx, dpi, overridesFor(sc, 1, 1))
    if (prelim.length === 0 && opts.orientation === 'auto' && segments.length > 1) {
      sharedOpts = { ...opts, orientation: plan.layout.page.orientation }
    }
    prelim.push(plan)
  }
  const drawingPages = prelim.reduce((a, p) => a + p.pages.length, 0)
  // Appended pages (the cut list) count in "Page x of N".
  const total = drawingPages + extraPages
  // Pass 2: the real plans with global page numbers (a single live-view job
  // with nothing appended is already final).
  let plans: PrintPlan[] = prelim
  if (!single || extraPages > 0) {
    let offset = 1
    plans = segments.map((sc, i) => {
      const plan = planPrint(sharedOpts, source, ctx, dpi, overridesFor(sc, offset, total))
      offset += prelim[i].pages.length
      return plan
    })
  }
  const pages: PrintJob['pages'] = []
  plans.forEach((p, i) => p.pages.forEach((page) => pages.push({ plan: i, page })))
  const warnings = [...new Set(plans.flatMap((p) => p.warnings))]
  if (total > MANY_PAGES_WARNING && !warnings.some((w) => w.includes('pages'))) warnings.push(`${total} pages — check the scale`)
  return {
    plans,
    pages,
    /** Drawing pages only; appended pages are the caller's. */
    totalPages: drawingPages,
    empty: plans.every((p) => p.empty),
    warnings,
    projectionNote: plans.some((p) => p.projectionNote),
    jobTitle: plans[0]?.jobTitle ?? jobTitleFor(ctx, opts, VIEW_LABEL.current),
  }
}
