/**
 * The Print dialog's brain (docs/design/printing.md §4), shared by the
 * desktop dialog (`panels/PrintDialog.tsx`) and the phone sheet
 * (`shop/PrintSheet.tsx`): options + preferences, the planned job, the
 * debounced preview renders, vector Line-art drawings, cut-list pages, and
 * the two actions (Print… → the shell's PrintHost, Save PDF… → Hew's own
 * PDF). Pure state and effects — no markup — so the two surfaces can look
 * entirely different and behave identically.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { ViewportApi } from '../viewport/Viewport'
import type { Scene as WasmScene } from '../wasm/loader'
import { lineArtSvgForTile, projectAnnotationOverlay, requestLineDrawing, type AnnotationOverlay, type LineDrawingData } from './lineArt'
import { buildPrintPdf } from './pdf'
import { blankTile, cutListPageFurniture, cutListRows, cutListRowsPerPage } from './cutList'
import { waitForPrintImages, type PrintPageModel } from './PrintDocument'
import { MANY_PAGES_WARNING, PREVIEW_DPI, pageCountText, type PrintLayout } from './layout'
import { matchNamedPaper, paperLabel, paperSize, orient, type PaperSpec } from './paper'
import { defaultPrintOptions, fitScaleFor, planPrintJob, printDocName, type PrintJob, type PrintOptions, type PrintPlan, type PrintPlanContext, type SceneSource } from './printJob'
import { parseCustomScale, sameScale, scaleDisplay, scalePresetsFor, scaleRatio, type PrintScale } from './scale'
import type { PrintHost } from './printHost'
import { getPrintPrefs, setPrintPrefs, type PrintPrefs } from '../settings/print'
import { getLengthUnit, type LengthFormat } from '../settings/units'
import { regionFromLocale } from '../shop/localeUnits'
import { defaultPaperForRegion } from './paper'

export interface PrintControllerProps {
  getViewportApi: () => ViewportApi | null
  /** The live kernel scene (vector Line art asks it for the hidden-line drawing). */
  getScene: () => WasmScene | null
  /** The document's Scenes, resolved (for Pages: Each Scene); empty when none. */
  getScenes: () => SceneSource[]
  documentName: string
  /** Active, undrifted Scene name, or null. */
  sceneName: string | null
  printHost: PrintHost
  /** Save PDF…: write the bytes through the shell's file host. Resolves true
   * when saved, false when the user cancelled. */
  savePdf: (bytes: Uint8Array, suggestedName: string) => Promise<boolean>
  /** Test seam: called with the final page models right before the host is
   * invoked (the harness records these). */
  onPagesReady?: (pages: PrintPageModel[], plan: PrintPlan) => void
  /** Phone: no Selection extent (the phone is read-only; isolate is what's
   * visible), no Each Scene, no custom paper. */
  compact?: boolean
}

export type PrintStatus = 'idle' | 'rendering' | 'sent' | 'saved' | 'error'

/** One page in the preview, in job order: drawing pages then cut-list pages. */
export interface PreviewPage {
  /** Global 0-based index. */
  index: number
  layout: PrintLayout
  /** Scene name under Pages = Each Scene, else null. */
  group: string | null
  page: PrintPageModel
}

export interface PrintController {
  opts: PrintOptions
  setOpts: (update: (o: PrintOptions) => PrintOptions) => void
  format: LengthFormat
  compact: boolean
  /** The whole planned job (null before the viewport is ready). */
  job: PrintJob | null
  /** The first (or only) segment — what single-plan controls read. */
  plan: PrintPlan | null
  scenes: SceneSource[]
  hasSelection: boolean
  status: PrintStatus
  progress: { done: number; total: number } | null
  error: string | null
  busy: boolean
  canPrint: boolean
  /** Every page of the job for the preview, drawing pages then cut list. */
  previewPages: PreviewPage[]
  /** Number of appended cut-list pages. */
  cutPageCount: number
  /** Whole-job vector Line art (else raster). */
  vectorPages: boolean
  /** Why a Line-art job fell back to raster, if it did. */
  vectorNote: string | null
  /** The footer summary for the idle state ("6 pages (3 × 2) · Letter portrait · 1:1 (1 cm = 1 cm) · print at 100 %"). */
  summary: string
  /** Job title = default PDF file name ("Café table — 1:10"). */
  jobTitle: string
  /** The name the last Save PDF… wrote (for "Saved “….pdf”"). */
  savedName: string | null
  /** Print… : render at print resolution, mount the print root, hand off. */
  doPrint: () => Promise<void>
  /** Save PDF… : Hew's own PDF through the shell's file host. */
  doSavePdf: () => Promise<void>
  /** Largest ladder scale that fits one page; resets the nudge. */
  fit: () => void
  /** Move the tile grid by paper mm (Scaled, tiled). */
  nudge: (dx: number, dy: number) => void
  centerNudge: () => void
  isNudged: boolean
  /** Apply a custom scale from two typed lengths; returns an error string
   * or null when applied. */
  applyCustomScale: (paperText: string, modelText: string) => string | null
  /** Set a custom paper size (mm); returns an error string or null. */
  applyCustomPaper: (wMm: number, hMm: number) => string | null
  switchMode: (mode: PrintOptions['mode']) => void
  presets: PrintScale[]
  /** Index of the current scale in `presets`, or -1 (custom). */
  presetIndex: number
  /** The print root's content once a job was rendered (mount `PrintRoot`). */
  printed: { pages: PrintPageModel[]; layout: PrintLayout; title: string } | null
  /** Set by the desktop dialog when the OS print dialog isn't reachable
   * (`printHost.fallback`) so the view can offer the browser dialog. */
  hasPrintFallback: boolean
  /** Print through the fallback host (browser dialog), when there is one. */
  doPrintFallback: () => Promise<void>
  /** CSS scale that makes 1 paper mm one real millimetre on THIS display
   * (1 when the platform can't say — the CSS 96 dpi rule). */
  trueSizeScale: number
  /** Ask for preview bitmaps at about this many dots per paper inch (the
   * view derives it from the on-screen page size); clamped and rounded. */
  setPreviewDpi: (dpi: number) => void
  /** Ask for ONE page's bitmap at a sharper dpi (the 100 % view) — rendered
   * on its own so the rest of the grid is left alone; null clears it. */
  setInspect: (req: { index: number; dpi: number } | null) => void
  /** The inspect bitmap once it lands (job page index → object URL). */
  inspectImage: { index: number; url: string } | null
}

/** Preview bitmaps are rendered for at most this many drawing pages per
 * job; later pages keep their furniture but show no image. */
export const PREVIEW_IMAGE_BUDGET = 24
/** Sharpest preview bitmap (the 100 % view on a dense display). */
export const PREVIEW_DPI_MAX = 220
const CUSTOM_PAPER_MIN_MM = 50
const CUSTOM_PAPER_MAX_MM = 1200
export const CUSTOM_PAPER_ERROR = 'Paper must be between 50 mm and 1200 mm.'
export const CUSTOM_PAPER_PARSE_ERROR = 'Enter sizes like “216 mm” or “11 in”.'
export const CUSTOM_SCALE_PARSE_ERROR = 'Enter a length like “1 cm”, “1in” or “3/8″”.'
export const CUSTOM_SCALE_RANGE_ERROR = 'Scale must be between 1:1000 and 20:1.'

function prefsToOptions(prefs: PrintPrefs, format: LengthFormat): PrintOptions {
  const base = defaultPrintOptions(format, prefs.paper)
  const mode = prefs.lastMode
  const scaled = prefs.scaled
  return {
    ...base,
    mode,
    paper: prefs.paper,
    orientation: prefs.orientation,
    margin: prefs.margin,
    style: mode === 'standard' ? prefs.standard.style : scaled.style,
    includeGuides: mode === 'standard' ? prefs.standard.includeGuides : scaled.includeGuides,
    includeGridAxes: prefs.standard.includeGridAxes,
    zoom: prefs.standard.zoom,
    titleBlock: mode === 'standard' ? prefs.standard.titleBlock : scaled.titleBlock,
    marks: scaled.marks,
    overlap: scaled.overlap,
    scaleBar: scaled.scaleBar,
    hiddenDashed: scaled.hiddenDashed,
    cutList: prefs.cutList,
    // A stored preset carries its label from whenever it was saved; show
    // it as the ladder labels it today (the ratio is what is stored).
    scale: relabelPreset(scaled.scale ?? base.scale, format),
    extent: scaled.extent,
    view: scaled.view,
  }
}

function relabelPreset(scale: PrintScale, format: LengthFormat): PrintScale {
  if (scale.kind !== 'preset') return scale
  const match = scalePresetsFor(format).find((p) => sameScale(p, scale))
  return match ?? scale
}

function optionsToPrefs(prev: PrintPrefs, o: PrintOptions): PrintPrefs {
  const std = o.mode === 'standard'
  return {
    ...prev,
    paper: o.paper,
    // A hand-picked paper ends the one-time OS/locale seeding.
    paperSeeded: prev.paperSeeded || JSON.stringify(o.paper) !== JSON.stringify(prev.paper),
    orientation: o.orientation,
    margin: o.margin,
    lastMode: o.mode,
    cutList: o.cutList,
    standard: std ? { style: o.style, includeGuides: o.includeGuides, includeGridAxes: o.includeGridAxes, titleBlock: o.titleBlock, zoom: o.zoom } : { ...prev.standard, includeGridAxes: o.includeGridAxes, zoom: o.zoom },
    scaled: std
      ? { ...prev.scaled, scale: o.scale, extent: o.extent, view: o.view, marks: o.marks, overlap: o.overlap, scaleBar: o.scaleBar, hiddenDashed: o.hiddenDashed }
      : { style: o.style, includeGuides: o.includeGuides, hiddenDashed: o.hiddenDashed, overlap: o.overlap, marks: o.marks, scaleBar: o.scaleBar, titleBlock: o.titleBlock, scale: o.scale, extent: o.extent, view: o.view },
  }
}

/** Seed the paper from the locale the first time; on desktop the OS default
 * (`PrintHost.defaults`) replaces that seed asynchronously below, still only
 * while nothing was chosen by hand. */
function seededPrefs(): PrintPrefs {
  const p = getPrintPrefs()
  if (p.paperSeeded) return p
  const region = typeof navigator === 'undefined' ? null : regionFromLocale(navigator.language)
  const next: PrintPrefs = { ...p, paper: defaultPaperForRegion(region), paperSeeded: false }
  setPrintPrefs(next)
  return next
}

function todayText(): string {
  try {
    return new Date().toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' })
  } catch {
    return new Date().toISOString().slice(0, 10)
  }
}

function switchModeOpts(o: PrintOptions, mode: PrintOptions['mode']): PrintOptions {
  if (o.mode === mode) return o
  const prefs = getPrintPrefs()
  const src = mode === 'standard' ? prefs.standard : prefs.scaled
  return { ...o, mode, style: src.style, includeGuides: src.includeGuides, titleBlock: src.titleBlock }
}

function renderOptions(opts: PrintOptions, plan: PrintPlan, dpi: number, format: 'image/png' | 'image/jpeg', quality: number) {
  return {
    hiddenOverride: plan.scene?.hidden ?? null,
    sectionOverride: plan.scene !== null && plan.scene.section !== undefined ? plan.scene.section : undefined,
    style: opts.style,
    includeGuides: opts.includeGuides,
    includeGridAxes: opts.mode === 'standard' && opts.includeGridAxes,
    includeAnnotations: opts.includeDimensions,
    restrictTo: plan.restrictTo,
    dpi,
    format,
    quality,
  }
}

/** "6 pages (3 × 2) · Letter portrait · 1:1 (1 cm = 1 cm) · print at 100 %". */
export function summaryText(job: PrintJob, plan: PrintPlan, opts: PrintOptions, extraPages = 0): string {
  const count = job.plans.length === 1 ? pageCountText(plan.layout) : `${job.totalPages} pages · ${job.plans.length} Scenes`
  const parts = [extraPages > 0 ? `${count} + ${extraPages} cut list` : count, `${paperLabel(opts.paper)} ${plan.layout.page.orientation}`]
  if (plan.scaleText !== null) parts.push(plan.scaleText, 'print at 100 %')
  return parts.join(' · ')
}

/** Name of the one selected part, for the title block; null when the
 * selection is empty, plural, or unnamed. */
function selectionName(scene: WasmScene | null, sel: { objects: bigint[]; instances: bigint[] }): string | null {
  if (scene === null) return null
  const n = sel.objects.length + sel.instances.length
  if (n !== 1) return null
  try {
    if (sel.objects.length === 1) return scene.object_name(sel.objects[0]) ?? null
    const id = sel.instances[0]
    const own = scene.instance_name(id)
    if (own !== undefined && own !== '') return own
    const def = scene.instance_def(id)
    return def === undefined ? null : (scene.component_name(def) ?? null)
  } catch {
    return null
  }
}

export function usePrintController({ getViewportApi, getScene, getScenes, documentName, sceneName, printHost, savePdf, onPagesReady, compact = false }: PrintControllerProps): PrintController {
  const format = getLengthUnit()
  const [opts, setOptsState] = useState<PrintOptions>(() => {
    const o = prefsToOptions(seededPrefs(), format)
    // The phone has no Selection extent (read-only; isolate is what is
    // visible): a desktop-persisted 'selection' falls back to Model here.
    return compact && o.extent === 'selection' ? { ...o, extent: 'model' } : o
  })
  const [revision, setRevision] = useState(0)
  const [previewUrls, setPreviewUrls] = useState<(string | null)[]>([])
  const [status, setStatus] = useState<PrintStatus>('idle')
  const statusRef = useRef(status)
  statusRef.current = status
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [savedName, setSavedName] = useState<string | null>(null)
  /** The print root's content, captured when the job was rendered — never
   * the live plan, which may have moved on. */
  const [printed, setPrinted] = useState<{ pages: PrintPageModel[]; layout: PrintLayout; title: string } | null>(null)
  const [trueSizeScale, setTrueSizeScale] = useState(1)
  useEffect(() => {
    if (printHost.screenMmPerPx === undefined) return
    let cancelled = false
    void printHost.screenMmPerPx().then((mmPerPx) => {
      if (cancelled || mmPerPx === null) return
      // CSS lays out 1 mm as 96/25.4 px; the display really shows 1/mmPerPx px per mm.
      setTrueSizeScale(1 / mmPerPx / (96 / 25.4))
    })
    return () => {
      cancelled = true
    }
  }, [printHost])
  const printUrlsRef = useRef<string[]>([])
  const previewUrlsRef = useRef<string[]>([])
  const mountedRef = useRef(true)

  const api = getViewportApi()
  const selection = useMemo(() => api?.getSelectedIds() ?? { objects: [], instances: [] }, [api, revision])
  const hasSelection = selection.objects.length + selection.instances.length > 0
  // The dialog is modal, but the selection can still change under it (⌘A
  // reaches the app's global handler; a Scene switch elsewhere). Re-read it
  // after any key or pointer release and re-plan when it differs.
  const selectionKeyRef = useRef('')
  selectionKeyRef.current = `${selection.objects.join(',')}|${selection.instances.join(',')}`
  useEffect(() => {
    if (api === null) return
    const check = (): void => {
      // The job is frozen while pages render (see setOpts).
      if (statusRef.current === 'rendering') return
      const s = api.getSelectedIds()
      const key = `${s.objects.join(',')}|${s.instances.join(',')}`
      if (key !== selectionKeyRef.current) setRevision((r) => r + 1)
    }
    window.addEventListener('keyup', check, true)
    window.addEventListener('pointerup', check, true)
    return () => {
      window.removeEventListener('keyup', check, true)
      window.removeEventListener('pointerup', check, true)
    }
  }, [api])
  const selName = useMemo(() => selectionName(getScene(), selection), [getScene, selection])

  const ctx = useMemo<PrintPlanContext>(
    () => ({ documentName, sceneName, unitFormat: format, dateText: todayText(), selectionName: selName }),
    [documentName, sceneName, format, selName],
  )

  const setOpts = useCallback((update: (o: PrintOptions) => PrintOptions) => {
    // While pages are rendering the job is frozen: an option change would
    // re-plan under the running pass (and re-measure the extent through the
    // scene's print-pass seams). Controls are disabled then too; this is the
    // belt to that brace (keyboard shortcuts, arrow-key nudge).
    if (statusRef.current === 'rendering') return
    setOptsState((o) => {
      const next = update(o)
      setPrintPrefs(optionsToPrefs(getPrintPrefs(), next))
      return next
    })
    // A change after a handoff starts a fresh job: back to the summary line.
    setStatus((st) => (st === 'sent' || st === 'saved' || st === 'error' ? 'idle' : st))
    // Re-read the viewport (selection, hidden set, section plane, Scenes) on
    // every option change — the dialog is modal, so this is when it matters.
    setRevision((r) => r + 1)
  }, [])

  // ---- plan (pure, cheap): every option change and every viewport read.
  // Resolved once per open (and per `revision`), NOT per render — App
  // re-renders often and a fresh array would re-plan and re-render previews.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const scenes = useMemo(() => (compact ? [] : getScenes()), [revision])
  // Losing the selection falls Extent back to Model (§4).
  useEffect(() => {
    if (opts.mode === 'scaled' && opts.extent === 'selection' && !hasSelection) setOptsState((o) => ({ ...o, extent: 'model' }))
  }, [hasSelection, opts.mode, opts.extent])
  // Cut-list rows (when on) — their page count feeds the job's numbering.
  const cutRows = useMemo(() => {
    if (!opts.cutList) return []
    const scene = getScene()
    return scene === null ? [] : cutListRows(scene, format)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [opts.cutList, format, revision])
  // Planning can fail transiently: a Selection extent or a Scene's hidden
  // set measures through a print pass on the scene, and a preview render's
  // own pass may be mid-flight at that instant (passes are exclusive). Such
  // a failure is retried shortly, not left as a dead dialog.
  const planFailedRef = useRef(false)
  const job = useMemo<PrintJob | null>(() => {
    if (api === null) return null
    try {
      // Plan once to learn the real page geometry (a tiled Scaled job's
      // drawing area is shorter by the overlap band), size the cut list on
      // THAT layout, then re-plan with the appended pages counted into
      // "Page x of y" — so the reservation and the rendered cut-list pages
      // agree by construction.
      const first = planPrintJob(opts, api, ctx, scenes, undefined, 0)
      planFailedRef.current = false
      if (cutRows.length === 0 || first.plans.length === 0) return first
      const n = Math.ceil(cutRows.length / cutListRowsPerPage(first.plans[0].layout))
      return planPrintJob(opts, api, ctx, scenes, undefined, n)
    } catch {
      planFailedRef.current = true
      return null
    }
    // `revision` forces a re-plan when the viewport moved without an option change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, opts, ctx, scenes, cutRows, revision])
  useEffect(() => {
    if (job !== null || api === null || !planFailedRef.current) return
    const t = setTimeout(() => setRevision((r) => r + 1), 150)
    return () => clearTimeout(t)
  }, [job, api, revision])
  const plan: PrintPlan | null = job?.plans[0] ?? null

  // Vector Line art (Scaled + Line art): one hidden-line drawing per plan
  // (per Scene under Each Scene), sliced per tile into SVG. Falls back to
  // raster (with a note) when the kernel refuses the model as too complex.
  type VectorSeg = { svgs: (string | null)[]; drawing: LineDrawingData | null; annotations: AnnotationOverlay | null; fallbackNote: string | null }
  const vectors = useMemo<(VectorSeg | null)[]>(() => {
    if (job === null || api === null || opts.mode !== 'scaled' || opts.style !== 'lineart' || job.empty) return []
    const scene = getScene()
    if (scene === null) return []
    return job.plans.map((plan): VectorSeg | null => {
      const first = plan.pages[0]?.request.camera
      if (first === undefined || first.kind !== 'ortho' || plan.empty) return null
      const sc = plan.scene
      const liveSection = api.getSectionState()
      const sectionState = sc !== null && sc.section !== undefined ? sc.section : liveSection
      const hidden = sc?.hidden ?? api.getHiddenIds()
      const res = requestLineDrawing({
        scene,
        camera: first,
        section: sectionState !== null && sectionState.active ? { origin: sectionState.origin, normal: sectionState.normal } : null,
        hidden,
        hiddenAuthoritative: sc?.hidden !== null && sc?.hidden !== undefined,
        only: plan.restrictTo === null ? null : { objects: [...plan.restrictTo.objects], instances: [...plan.restrictTo.instances] },
        includeHidden: opts.hiddenDashed,
      })
      if (res.kind !== 'ok') {
        return { svgs: [], drawing: null, annotations: null, fallbackNote: res.kind === 'too-complex' ? 'Line art printed as a bitmap — the model is too complex for vector line work.' : `Line art printed as a bitmap (${res.message})` }
      }
      const annotations = opts.includeDimensions ? projectAnnotationOverlay(api.collectAnnotationDrawing(), first) : null
      const ratio = scaleRatio(opts.scale)
      const svgs = plan.pages.map((p) => lineArtSvgForTile(res.drawing, p.tile, plan.layout, ratio, { hiddenDashed: opts.hiddenDashed, annotations }))
      return { svgs, drawing: res.drawing, annotations, fallbackNote: null }
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [job, api, opts.mode, opts.style, opts.hiddenDashed, opts.includeDimensions, revision])
  /** True when every segment rendered vector (else the whole job is raster). */
  const vectorPages = vectors.length > 0 && vectors.every((v) => v !== null && v.drawing !== null)
  const vectorNote = vectors.find((v) => v !== null && v.fallbackNote !== null)?.fallbackNote ?? null
  /** SVG for global page index `i` (vector jobs). */
  const vectorSvgAt = useCallback(
    (i: number): string | null => {
      if (job === null || !vectorPages) return null
      let k = i
      for (let p = 0; p < job.plans.length; p++) {
        const n = job.plans[p].pages.length
        if (k < n) return vectors[p]?.svgs[k] ?? null
        k -= n
      }
      return null
    },
    [job, vectorPages, vectors],
  )

  // Cut-list pages appended after the drawing pages (furniture only).
  const cutPages = useMemo<PrintPageModel[]>(() => {
    if (job === null || plan === null || cutRows.length === 0) return []
    const per = cutListRowsPerPage(plan.layout)
    const n = Math.ceil(cutRows.length / per)
    const total = job.totalPages + n
    return Array.from({ length: n }, (_, i) => {
      const pageIndex = job.totalPages + i
      const tile = blankTile(plan.layout, pageIndex, `CL${i + 1}`)
      const furniture = cutListPageFurniture(plan.layout, cutRows, i, {
        documentName: printDocName(documentName),
        pageNumber: pageIndex + 1,
        totalPages: total,
        dateText: ctx.dateText,
        titleBlock: opts.titleBlock,
      })
      return { tile, furniture, imageUrl: null, blank: true }
    })
  }, [job, plan, cutRows, documentName, ctx.dateText, format, opts.titleBlock])

  // ---- preview: debounced re-render at `previewDpi` (the view sets it from
  // how large the pages are on screen — a big dialog or the 100 % view
  // wants a sharper bitmap than a thumbnail), capped at 24 tiles; the
  // previous images stay until the new ones land.
  const [previewDpi, setPreviewDpiState] = useState(PREVIEW_DPI)
  const setPreviewDpi = useCallback((dpi: number) => {
    const d = Math.max(PREVIEW_DPI, Math.min(PREVIEW_DPI_MAX, Math.round(dpi / 10) * 10))
    setPreviewDpiState((cur) => (cur === d ? cur : d))
  }, [])
  useEffect(() => {
    if (api === null || job === null || job.empty || status === 'rendering') return
    if (vectorPages) {
      setPreviewUrls([])
      return
    }
    let cancelled = false
    const t = setTimeout(() => {
      void (async () => {
        const urls: (string | null)[] = []
        let budget = PREVIEW_IMAGE_BUDGET
        try {
          for (const plan of job.plans) {
            const scale = previewDpi / plan.layout.dpi
            const take = Math.min(plan.pages.length, Math.max(0, budget))
            budget -= take
            const requests = plan.pages.slice(0, take).map((p) => ({
              camera: p.request.camera,
              widthPx: Math.max(1, Math.round(p.request.widthPx * scale)),
              heightPx: Math.max(1, Math.round(p.request.heightPx * scale)),
            }))
            const blobs = requests.length === 0 ? [] : await api.renderPrintPages(requests, renderOptions(opts, plan, previewDpi, 'image/jpeg', 0.88))
            if (cancelled || !mountedRef.current) {
              for (const u of urls) if (u !== null) URL.revokeObjectURL(u)
              return
            }
            for (let i = 0; i < plan.pages.length; i++) urls.push(blobs[i] !== undefined ? URL.createObjectURL(blobs[i]) : null)
          }
          for (const u of previewUrlsRef.current) URL.revokeObjectURL(u)
          previewUrlsRef.current = urls.filter((u): u is string => u !== null)
          setPreviewUrls(urls)
        } catch {
          if (!cancelled && mountedRef.current) setPreviewUrls([])
        }
      })()
    }, 150)
    return () => {
      cancelled = true
      clearTimeout(t)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, job, status, vectorPages, previewDpi])

  // ---- inspect: one page at a sharper dpi, rendered on its own.
  const [inspectReq, setInspectReqState] = useState<{ index: number; dpi: number } | null>(null)
  const setInspect = useCallback((req: { index: number; dpi: number } | null) => {
    setInspectReqState((cur) => {
      if (req === null) return cur === null ? cur : null
      const dpi = Math.max(PREVIEW_DPI, Math.min(PREVIEW_DPI_MAX, Math.round(req.dpi / 10) * 10))
      return cur !== null && cur.index === req.index && cur.dpi === dpi ? cur : { index: req.index, dpi }
    })
  }, [])
  const [inspectImage, setInspectImage] = useState<{ index: number; url: string } | null>(null)
  const inspectUrlRef = useRef<string | null>(null)
  useEffect(() => {
    if (api === null || job === null || job.empty || status === 'rendering' || inspectReq === null || vectorPages) {
      if (inspectReq === null || vectorPages) {
        if (inspectUrlRef.current !== null) URL.revokeObjectURL(inspectUrlRef.current)
        inspectUrlRef.current = null
        setInspectImage(null)
      }
      return
    }
    const jp = job.pages[inspectReq.index]
    if (jp === undefined) return
    const plan = job.plans[jp.plan]
    let cancelled = false
    const t = setTimeout(() => {
      void (async () => {
        try {
          const scale = inspectReq.dpi / plan.layout.dpi
          const req = { camera: jp.page.request.camera, widthPx: Math.max(1, Math.round(jp.page.request.widthPx * scale)), heightPx: Math.max(1, Math.round(jp.page.request.heightPx * scale)) }
          const blobs = await api.renderPrintPages([req], renderOptions(opts, plan, inspectReq.dpi, 'image/jpeg', 0.9))
          if (cancelled || !mountedRef.current || blobs[0] === undefined) return
          const url = URL.createObjectURL(blobs[0])
          if (inspectUrlRef.current !== null) URL.revokeObjectURL(inspectUrlRef.current)
          inspectUrlRef.current = url
          setInspectImage({ index: inspectReq.index, url })
        } catch {
          /* the grid bitmap stands */
        }
      })()
    }, 100)
    return () => {
      cancelled = true
      clearTimeout(t)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, job, status, vectorPages, inspectReq])
  useEffect(
    () => () => {
      if (inspectUrlRef.current !== null) URL.revokeObjectURL(inspectUrlRef.current)
      inspectUrlRef.current = null
    },
    [],
  )

  // Desktop: the OS default paper seeds the preference once (never over a
  // hand-picked paper; the locale seed stands until this answers).
  useEffect(() => {
    if (getPrintPrefs().paperSeeded) return
    if (printHost.defaults === undefined) {
      setPrintPrefs({ ...getPrintPrefs(), paperSeeded: true })
      return
    }
    let cancelled = false
    void printHost.defaults().then((d) => {
      if (cancelled || getPrintPrefs().paperSeeded) return
      const prefs = getPrintPrefs()
      if (d !== null) {
        const named = matchNamedPaper(d.paperWmm, d.paperHmm)
        const paper: PaperSpec = named ?? { w_mm: Math.min(d.paperWmm, d.paperHmm), h_mm: Math.max(d.paperWmm, d.paperHmm) }
        setPrintPrefs({ ...prefs, paper, paperSeeded: true })
        setOptsState((o) => ({ ...o, paper }))
      } else {
        setPrintPrefs({ ...prefs, paperSeeded: true })
      }
    })
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
      for (const u of previewUrlsRef.current) URL.revokeObjectURL(u)
      for (const u of printUrlsRef.current) URL.revokeObjectURL(u)
      previewUrlsRef.current = []
      printUrlsRef.current = []
    }
  }, [])

  const busy = status === 'rendering'
  const canPrint = job !== null && !job.empty && !busy

  /** Render every drawing page of the job at print resolution (per segment,
   * with that Scene's hidden/section), returning blobs in job page order. */
  const renderJobRasters = useCallback(
    async (j: PrintJob, onProgress: (done: number, total: number) => void): Promise<Blob[]> => {
      if (api === null) throw new Error('viewport not ready')
      const out: Blob[] = []
      let done = 0
      for (const plan of j.plans) {
        const blobs = await api.renderPrintPages(
          plan.pages.map((p) => p.request),
          renderOptions(opts, plan, plan.layout.dpi, opts.style === 'lineart' ? 'image/png' : 'image/jpeg', 0.92),
          (d) => onProgress(done + d, j.totalPages),
        )
        done += plan.pages.length
        out.push(...blobs)
      }
      return out
    },
    [api, opts],
  )

  const printWith = useCallback(
    async (host: PrintHost) => {
      if (api === null || job === null || job.empty || busy) return
      setStatus('rendering')
      setError(null)
      setProgress({ done: 0, total: job.totalPages })
      try {
        let models: PrintPageModel[]
        if (vectorPages) {
          models = job.pages.map((jp, i) => ({ tile: jp.page.tile, furniture: jp.page.furniture, imageUrl: null, vectorSvg: vectorSvgAt(i) }))
        } else {
          const blobs = await renderJobRasters(job, (done, total) => setProgress({ done, total }))
          for (const u of printUrlsRef.current) URL.revokeObjectURL(u)
          const urls = blobs.map((b) => URL.createObjectURL(b))
          printUrlsRef.current = urls
          models = job.pages.map((jp, i) => ({ tile: jp.page.tile, furniture: jp.page.furniture, imageUrl: urls[i] ?? null }))
        }
        models = [...models, ...cutPages]
        if (!mountedRef.current) {
          // Closed mid-render: nothing goes to the OS dialog (it would print
          // the live DOM, not the pages), and the bitmaps are released.
          for (const u of printUrlsRef.current) URL.revokeObjectURL(u)
          printUrlsRef.current = []
          return
        }
        const layout = job.plans[0].layout
        setPrinted({ pages: models, layout, title: job.jobTitle })
        onPagesReady?.(models, job.plans[0])
        // Let React commit the print root, then wait for the images to decode.
        await new Promise<void>((r) => setTimeout(r, 0))
        await waitForPrintImages()
        if (!mountedRef.current) return
        const paper = orient(paperSize(opts.paper), layout.page.orientation)
        await host.print({ paperWmm: paper.w, paperHmm: paper.h, landscape: layout.page.orientation === 'landscape', jobTitle: job.jobTitle })
        if (mountedRef.current) setStatus('sent')
      } catch (err) {
        console.error('[print] failed:', err)
        if (mountedRef.current) {
          setStatus('error')
          setError(err instanceof Error ? err.message : String(err))
        }
      } finally {
        if (mountedRef.current) setProgress(null)
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [api, job, busy, opts, onPagesReady, vectors, vectorPages, cutPages, renderJobRasters, vectorSvgAt],
  )
  const doPrint = useCallback(() => printWith(printHost), [printWith, printHost])
  const doPrintFallback = useCallback(async () => {
    if (printHost.fallback !== undefined) await printWith(printHost.fallback)
  }, [printWith, printHost])

  const doSavePdf = useCallback(async () => {
    if (api === null || job === null || job.empty || busy) return
    setStatus('rendering')
    setError(null)
    setProgress({ done: 0, total: job.totalPages })
    try {
      const rasters: (Blob | null)[] = vectorPages ? job.pages.map(() => null) : await renderJobRasters(job, (done, total) => setProgress({ done, total }))
      const ratio = scaleRatio(opts.scale)
      const bytes = await buildPrintPdf({
        pages: [
          ...job.pages.map((jp, i) => {
            const v = vectorPages ? vectors[jp.plan] : null
            return {
              tile: jp.page.tile,
              furniture: jp.page.furniture,
              layout: job.plans[jp.plan].layout,
              raster: rasters[i] ?? null,
              vector: v !== null && v !== undefined && v.drawing !== null ? { drawing: v.drawing, ratio, hiddenDashed: opts.hiddenDashed, annotations: v.annotations } : null,
            }
          }),
          ...cutPages.map((p) => ({ tile: p.tile, furniture: p.furniture, layout: job.plans[0].layout, raster: null, vector: null, blank: true })),
        ],
        title: job.jobTitle,
      })
      if (!mountedRef.current) return // closed mid-render: no file appears unasked
      const name = job.jobTitle.replace(/[\\/:*?"<>|]/g, '-')
      const ok = await savePdf(bytes, name)
      if (mountedRef.current) {
        setSavedName(ok ? name : null)
        setStatus(ok ? 'saved' : 'idle')
      }
    } catch (err) {
      console.error('[print] save pdf failed:', err)
      if (mountedRef.current) {
        setStatus('error')
        setError(err instanceof Error ? err.message : String(err))
      }
    } finally {
      if (mountedRef.current) setProgress(null)
    }
  }, [api, job, busy, opts, savePdf, vectors, vectorPages, cutPages, renderJobRasters])

  const presets = useMemo(() => scalePresetsFor(format), [format])
  const presetIndex = presets.findIndex((p) => sameScale(p, opts.scale))

  const applyCustomScale = useCallback(
    (paperText: string, modelText: string): string | null => {
      const s = parseCustomScale(paperText, modelText, format)
      if (s === null) {
        // Distinguish "unparseable" from "out of range" for the message.
        const probe = parseCustomScale(paperText, modelText, format, { unbounded: true })
        return probe === null ? CUSTOM_SCALE_PARSE_ERROR : CUSTOM_SCALE_RANGE_ERROR
      }
      setOpts((o) => ({ ...o, scale: s }))
      return null
    },
    [format, setOpts],
  )

  const applyCustomPaper = useCallback(
    (wMm: number, hMm: number): string | null => {
      if (!isFinite(wMm) || !isFinite(hMm)) return CUSTOM_PAPER_PARSE_ERROR
      if (wMm < CUSTOM_PAPER_MIN_MM || hMm < CUSTOM_PAPER_MIN_MM || wMm > CUSTOM_PAPER_MAX_MM || hMm > CUSTOM_PAPER_MAX_MM) return CUSTOM_PAPER_ERROR
      setOpts((o) => ({ ...o, paper: { w_mm: Math.min(wMm, hMm), h_mm: Math.max(wMm, hMm) } }))
      return null
    },
    [setOpts],
  )

  const fit = useCallback((): void => {
    if (plan === null) return
    const s = fitScaleFor(plan, format)
    if (s !== null) setOpts((o) => ({ ...o, scale: s, nudgeMm: { dx: 0, dy: 0 } }))
  }, [plan, format, setOpts])

  const nudge = useCallback((dx: number, dy: number) => setOpts((o) => ({ ...o, nudgeMm: { dx: o.nudgeMm.dx + dx, dy: o.nudgeMm.dy + dy } })), [setOpts])
  const centerNudge = useCallback(() => setOpts((o) => ({ ...o, nudgeMm: { dx: 0, dy: 0 } })), [setOpts])
  const switchMode = useCallback((mode: PrintOptions['mode']) => setOpts((o) => switchModeOpts(o, mode)), [setOpts])

  const previewPages = useMemo<PreviewPage[]>(() => {
    if (job === null || plan === null || job.empty) return []
    const out: PreviewPage[] = job.pages.map((jp, i) => ({
      index: i,
      layout: job.plans[jp.plan].layout,
      group: job.plans[jp.plan].scene?.name ?? null,
      page: { tile: jp.page.tile, furniture: jp.page.furniture, imageUrl: previewUrls[i] ?? null, vectorSvg: vectorSvgAt(i) },
    }))
    cutPages.forEach((page, k) => out.push({ index: job.pages.length + k, layout: plan.layout, group: null, page }))
    return out
  }, [job, plan, previewUrls, vectorSvgAt, cutPages])

  const summary = job === null || plan === null ? '' : summaryText(job, plan, opts, cutPages.length)
  const isNudged = plan !== null && (plan.layout.nudgeMm.dx !== 0 || plan.layout.nudgeMm.dy !== 0)

  return {
    opts,
    setOpts,
    format,
    compact,
    job,
    plan,
    scenes,
    hasSelection,
    status,
    progress,
    error,
    busy,
    canPrint,
    previewPages,
    cutPageCount: cutPages.length,
    vectorPages,
    vectorNote,
    summary,
    jobTitle: job?.jobTitle ?? '',
    savedName,
    doPrint,
    doSavePdf,
    fit,
    nudge,
    centerNudge,
    isNudged,
    applyCustomScale,
    applyCustomPaper,
    switchMode,
    presets,
    presetIndex,
    printed,
    hasPrintFallback: printHost.fallback !== undefined,
    doPrintFallback,
    trueSizeScale,
    setPreviewDpi,
    setInspect,
    inspectImage,
  }
}

export { MANY_PAGES_WARNING, scaleDisplay }
