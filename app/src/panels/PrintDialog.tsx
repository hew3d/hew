/**
 * File ▸ Print… — the desktop Print dialog (docs/design/printing.md §4,
 * design_handoff_printing/SPEC.md §2), a thin view over the controller hook
 * (`usePrintController`) it shares with the phone Shop Mode sheet
 * (`shop/PrintSheet.tsx`, `compact: true`) — that sheet calls the hook
 * directly rather than through this component, so this file is desktop-only
 * (no `compact` branch).
 */
import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { PrintPage, PrintRoot } from '../print/PrintDocument'
import { mmToPx } from '../print/layout'
import { PAPER_LABEL, PAPER_NAMES, paperSize, type PaperName, type PaperSpec } from '../print/paper'
import { VIEW_LABEL, type PrintOptions, type PrintViewKind } from '../print/printJob'
import { ratioText, scaleHint, type PrintScale } from '../print/scale'
import {
  MANY_PAGES_WARNING,
  PREVIEW_IMAGE_BUDGET,
  usePrintController,
  type PreviewPage,
  type PrintController,
  type PrintControllerProps,
} from '../print/usePrintController'
import { formatLengthIn, LENGTH_SYSTEM_OF, parseLengthToMeters, type LengthFormat } from '../settings/units'

export interface PrintDialogProps extends PrintControllerProps {
  onClose: () => void
}

/** Escape closes; Return prints (when focus isn't in a field) — shared by
 * both surfaces. */
function usePrintDialogKeys(onClose: () => void, doPrint: () => void): void {
  useEffect(() => {
    const onKey = (ev: KeyboardEvent): void => {
      if (ev.key === 'Escape') {
        ev.preventDefault()
        onClose()
      } else if (ev.key === 'Enter' && !(ev.target instanceof HTMLInputElement) && !(ev.target instanceof HTMLSelectElement) && !(ev.target instanceof HTMLTextAreaElement)) {
        ev.preventDefault()
        doPrint()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, doPrint])
}

export function PrintDialog({ onClose, ...controllerProps }: PrintDialogProps) {
  const c = usePrintController(controllerProps)
  const doPrint = c.doPrint
  usePrintDialogKeys(onClose, () => void doPrint())
  return <DesktopPrintDialog c={c} onClose={onClose} />
}

/* =========================================================================
 * Shared formatting helpers
 * ====================================================================== */

/** Scale `<select>` rows: "1:10 — 1 cm = 10 cm" (metric) / "1″ = 1′-0″ —
 * 1:12" (imperial, label already carries the reading so the ratio trails). */
function scaleMenuRows(presets: PrintScale[], format: LengthFormat, scale: PrintScale, presetIndex: number): { value: string; label: string }[] {
  const rows = presets.map((p, i) => {
    const hint = scaleHint(p, format)
    return { value: String(i), label: `${p.label} — ${hint ?? ratioText(p)}` }
  })
  if (presetIndex < 0 && scale.kind === 'fit') rows.push({ value: 'fit', label: `${scale.label} (fit)` })
  rows.push({ value: 'custom', label: 'Custom…' })
  return rows
}

/* =========================================================================
 * Desktop dialog (SPEC.md §2) — small presentational helpers
 * ====================================================================== */

interface SegOption {
  value: string
  label: string
  disabled?: boolean
  title?: string
}

/** A segmented control: `role="group"`, each chip a real `<button
 * aria-pressed>` (not `role="radio"`) so `getByRole('button', {name})`
 * keeps working in both the component tests and Playwright. */
function SegControl({ testId, ariaLabel, options, value, onChange, mode }: { testId: string; ariaLabel: string; options: SegOption[]; value: string; onChange: (v: string) => void; mode?: boolean }) {
  return (
    <div className={`hwprint__seg${mode === true ? ' hwprint__seg--mode' : ''}`} role="group" aria-label={ariaLabel} data-testid={testId}>
      {options.map((o) => (
        <button key={o.value} type="button" className="hwprint__seg-btn" aria-pressed={value === o.value} disabled={o.disabled === true} title={o.title} onClick={() => onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  )
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="hwprint__section">
      <div className="hwprint__section-head">{title}</div>
      <div className="hwprint__grid">{children}</div>
    </div>
  )
}

/** One label+control pair inside a `Section`'s two-column grid. */
function Row({ label, htmlFor, top, children }: { label: string; htmlFor?: string; top?: boolean; children: React.ReactNode }) {
  return (
    <>
      <label className={`hwprint__label${top === true ? ' hwprint__label--top' : ''}`} htmlFor={htmlFor}>{label}</label>
      <div className="hwprint__control">{children}</div>
    </>
  )
}

/** A grid item with no paired label (custom-value fields, inline errors,
 * hints) — occupies the control column only. */
function Solo({ children, className }: { children: React.ReactNode; className?: string }) {
  return <div className={`hwprint__solo${className !== undefined ? ' ' + className : ''}`}>{children}</div>
}

/** One preview tile: the real, CSS-scaled `PrintPage` when it has content
 * (a rendered raster/vector page, or a furniture-only cut-list page), else a
 * dashed placeholder outline (SPEC.md §2 "24 pages rendered, rest dashed"). */
/** `data-index` (the flat `previewPages` index) is read back by the groups
 * container's own pointer handling — inspect-toggling is resolved from
 * pointerup, not a native `click`, so it survives right after a drag
 * gesture on the same container (see the long comment on
 * `onGroupsPointerDown`). No `onClick` here as a result. */
function PreviewTile({ pp, scale }: { pp: PreviewPage; scale: number }) {
  // Every tile draws its real page (furniture, and the drawing once its
  // preview bitmap lands — a light pending box until then). Only pages past
  // the image budget never get a bitmap; they read as dashed outlines.
  const beyondBudget = pp.page.blank !== true && pp.page.imageUrl === null && (pp.page.vectorSvg === undefined || pp.page.vectorSvg === null) && pp.index >= PREVIEW_IMAGE_BUDGET
  const wPx = mmToPx(pp.layout.page.paper.w, 96) * scale
  const hPx = mmToPx(pp.layout.page.paper.h, 96) * scale
  return (
    <div className="hwprint__tile-col">
      <div
        className={`hwprint__tile${beyondBudget ? ' hwprint__tile--pending' : ''}`}
        style={{ width: `${wPx}px`, height: `${hPx}px` }}
        data-testid="print-preview-page"
        data-tile={pp.page.tile.id}
        data-scene={pp.group ?? undefined}
        data-index={pp.index}
      >
        <PrintPage layout={pp.layout} page={pp.page} scale={scale} />
      </div>
      {pp.layout.tiles.length > 1 && <div className="hwprint__tile-id">{pp.page.tile.id}</div>}
    </div>
  )
}

const PREVIEW_GRID_CAP = 60
const GAP_W = 10
const GAP_H = 12

interface PreviewGroup {
  label: string | null
  /** Every page in the group (for the "+N more" count). */
  total: PreviewPage[]
  /** Capped to `PREVIEW_GRID_CAP` for actual rendering. */
  shown: PreviewPage[]
}

/** Group consecutive preview pages by Scene name (Pages = Each Scene); a
 * single ungrouped run (Pages = Current view) yields one group with a null
 * label. Cut-list pages (group null) trail as their own group whenever a
 * Scene grouping precedes them. */
function groupPreviewPages(pages: PreviewPage[]): PreviewGroup[] {
  const groups: { label: string | null; pages: PreviewPage[] }[] = []
  for (const p of pages) {
    const last = groups[groups.length - 1]
    if (last !== undefined && last.label === p.group) last.pages.push(p)
    else groups.push({ label: p.group, pages: [p] })
  }
  return groups.map((g) => ({ label: g.label, total: g.pages, shown: g.pages.slice(0, PREVIEW_GRID_CAP) }))
}

/** The uniform CSS `scale` (unitless multiplier on `PrintPage`'s own
 * mm-as-CSS-length rendering) that fits every group's tile grid into the
 * measured preview area. */
/** Height of the "A1" caption row under each tile (10px text + 3px gap),
 * budgeted per grid row so it never clips against the container edge. */
const TILE_ID_H = 16
/** Per-group label ("SCENE NAME") + its own gap, budgeted once per group in
 * the multi-group (Each Scene) layout. */
const GROUP_LABEL_H = 22

function computePreviewScale(groups: PreviewGroup[], availW: number, availH: number): number {
  if (availW <= 0 || availH <= 0 || groups.length === 0) return 0.4
  const multi = groups.length > 1
  let s = 2
  for (const g of groups) {
    const layout = g.shown[0]?.layout
    if (layout === undefined) continue
    const cols = layout.cols
    const pw = mmToPx(layout.page.paper.w, 96)
    const ph = mmToPx(layout.page.paper.h, 96)
    s = Math.min(s, (availW - (cols - 1) * GAP_W) / (cols * pw))
    if (!multi) {
      const dispRows = Math.max(1, Math.ceil(g.shown.length / cols))
      s = Math.min(s, (availH - (dispRows - 1) * GAP_H - dispRows * TILE_ID_H) / (dispRows * ph))
    } else {
      const rows = Math.max(1, Math.ceil(g.shown.length / cols))
      const perGroupH = availH / groups.length - GROUP_LABEL_H
      s = Math.min(s, (perGroupH - (rows - 1) * GAP_H - rows * TILE_ID_H) / (rows * ph))
    }
  }
  return Math.max(0.04, Math.min(s, 1.4))
}

/* =========================================================================
 * Desktop dialog (SPEC.md §2)
 * ====================================================================== */

function DesktopPrintDialog({ c, onClose }: { c: PrintController; onClose: () => void }) {
  const { opts, setOpts, format, job, plan, scenes, hasSelection, status, progress, error, busy, canPrint, previewPages, vectorNote, summary, printed, presets, presetIndex, isNudged, hasPrintFallback } = c

  // ---- custom paper (typed in the current unit family: mm metric, in imperial)
  const paperIsCustom = typeof opts.paper !== 'string'
  const paperFmt: LengthFormat = LENGTH_SYSTEM_OF[format] === 'metric' ? 'mm' : 'dec_in'
  const [paperWText, setPaperWText] = useState('')
  const [paperHText, setPaperHText] = useState('')
  const [paperErr, setPaperErr] = useState<string | null>(null)
  useEffect(() => {
    if (paperIsCustom) {
      const p = opts.paper as { w_mm: number; h_mm: number }
      setPaperWText(formatLengthIn(p.w_mm / 1000, paperFmt))
      setPaperHText(formatLengthIn(p.h_mm / 1000, paperFmt))
    } else {
      setPaperWText('')
      setPaperHText('')
      setPaperErr(null)
    }
    // Only re-seed on the custom/named transition, not every keystroke.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paperIsCustom])
  const onPaperSelect = (v: string): void => {
    if (v === 'custom') {
      const cur = paperSize(opts.paper)
      setOpts((o) => ({ ...o, paper: { w_mm: cur.w, h_mm: cur.h } }))
    } else {
      setPaperErr(null)
      setOpts((o) => ({ ...o, paper: v as PaperSpec }))
    }
  }
  const commitCustomPaper = (): void => {
    const wM = parseLengthToMeters(paperWText, paperFmt)
    const hM = parseLengthToMeters(paperHText, paperFmt)
    setPaperErr(c.applyCustomPaper(wM === null ? NaN : wM * 1000, hM === null ? NaN : hM * 1000))
  }

  // ---- custom scale
  const [customScalePaper, setCustomScalePaper] = useState('')
  const [customScaleModel, setCustomScaleModel] = useState('')
  const [customScaleErr, setCustomScaleErr] = useState<string | null>(null)
  const [showCustomScale, setShowCustomScale] = useState(opts.scale.kind === 'custom')
  // Edited since the last commit: only a fresh pair commits on blur — an
  // empty or half-filled row is not an error, and a stale row (typed
  // earlier, then a preset or Fit chosen) must never overwrite that choice.
  const [customScaleDirty, setCustomScaleDirty] = useState(false)
  const editCustomPaper = (v: string): void => {
    setCustomScalePaper(v)
    setCustomScaleDirty(true)
  }
  const editCustomModel = (v: string): void => {
    setCustomScaleModel(v)
    setCustomScaleDirty(true)
  }
  const commitCustomScale = (): void => {
    if (!customScaleDirty) return
    if (customScalePaper.trim() === '' || customScaleModel.trim() === '') return
    const err = c.applyCustomScale(customScalePaper, customScaleModel)
    setCustomScaleErr(err)
    if (err === null) setCustomScaleDirty(false)
  }
  const fitScale = (): void => {
    setShowCustomScale(false)
    setCustomScaleErr(null)
    c.fit()
  }
  const scaleRows = useMemo(() => scaleMenuRows(presets, format, opts.scale, presetIndex), [presets, format, opts.scale, presetIndex])
  const scaleSelectValue = showCustomScale ? 'custom' : presetIndex >= 0 ? String(presetIndex) : opts.scale.kind === 'fit' ? 'fit' : 'custom'
  const onScaleSelect = (v: string): void => {
    if (v === 'custom') {
      // A fresh row: nothing typed yet, nothing to commit on blur.
      setCustomScalePaper('')
      setCustomScaleModel('')
      setCustomScaleDirty(false)
      setCustomScaleErr(null)
      setShowCustomScale(true)
      return
    }
    if (v === 'fit') return // already the current scale; nothing to change
    setShowCustomScale(false)
    setOpts((o) => ({ ...o, scale: presets[Number(v)] }))
  }
  const showCustomScaleUi = showCustomScale || (presetIndex < 0 && opts.scale.kind === 'custom')

  // ---- preview: measure the scroll area, group pages, compute a shared scale
  const previewScrollRef = useRef<HTMLDivElement>(null)
  const [previewSize, setPreviewSize] = useState({ w: 0, h: 0 })
  useEffect(() => {
    const el = previewScrollRef.current
    if (el === null) return
    const update = (): void => setPreviewSize({ w: el.clientWidth, h: el.clientHeight })
    update()
    if (typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(update)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])
  const groups = useMemo(() => groupPreviewPages(previewPages), [previewPages])
  // `previewSize` is the groups container's own clientWidth/Height, which
  // includes its `14px 16px` padding — subtract that (plus a small safety
  // margin) to get the box actually available to the tile grids.
  const gridScale = useMemo(() => computePreviewScale(groups, previewSize.w - 36, previewSize.h - 32), [groups, previewSize])
  // The drag handler reads the LIVE scale through this ref: a nudge that
  // adds a tile column re-scales the grid mid-gesture, and the px→mm
  // conversion must follow it or the drawing runs away from the cursor.
  const gridScaleRef = useRef(gridScale)
  gridScaleRef.current = gridScale
  const multiGroup = groups.length > 1
  const canNudge = opts.mode === 'scaled' && plan !== null && plan.layout.tiles.length > 1

  // ---- inspect (click a page to zoom; click again, or the caption, to return)
  // Inspect shows the page at 100 % print scale (1 CSS mm per paper mm —
  // far larger than the pane), scrollable and drag-to-pan, opened centred on
  // the point that was clicked; a click (no drag) closes it again.
  const [inspect, setInspect] = useState<{ index: number; fx: number; fy: number } | null>(null)
  useEffect(() => setInspect(null), [opts.mode, opts.pages, previewPages.length])
  const inspectPage = inspect !== null ? previewPages[inspect.index] : undefined
  // 100 % = one real millimetre per paper millimetre on this display where
  // the shell can measure it (desktop), else the CSS 96 dpi rule.
  const inspectScale = c.trueSizeScale
  // Preview bitmaps sharp enough for how large a page is on screen: CSS px
  // per paper inch × device pixel ratio. The grid asks at its own scale; the
  // 100 % view asks for just its one page at the display's density.
  const setPreviewDpi = c.setPreviewDpi
  const setInspectReq = c.setInspect
  useEffect(() => {
    const dpr = typeof window === 'undefined' ? 1 : window.devicePixelRatio || 1
    setPreviewDpi(96 * gridScale * dpr)
  }, [gridScale, setPreviewDpi])
  useEffect(() => {
    const dpr = typeof window === 'undefined' ? 1 : window.devicePixelRatio || 1
    setInspectReq(inspect === null ? null : { index: inspect.index, dpi: 96 * inspectScale * dpr })
  }, [inspect, inspectScale, setInspectReq])
  const inspectPageModel = useMemo(() => {
    if (inspectPage === undefined) return undefined
    const im = c.inspectImage
    return im !== null && im.index === inspectPage.index && inspectPage.page.blank !== true ? { ...inspectPage.page, imageUrl: im.url } : inspectPage.page
  }, [inspectPage, c.inspectImage])
  useLayoutEffect(() => {
    const el = previewScrollRef.current
    if (el === null || inspect === null || inspectPage === undefined) return
    const pw = mmToPx(inspectPage.layout.page.paper.w, 96) * inspectScale
    const ph = mmToPx(inspectPage.layout.page.paper.h, 96) * inspectScale
    // The scroll box's own padding sits before the page in scroll space.
    const cs = getComputedStyle(el)
    const padL = parseFloat(cs.paddingLeft) || 0
    const padT = parseFloat(cs.paddingTop) || 0
    el.scrollLeft = Math.max(0, padL + inspect.fx * pw - el.clientWidth / 2)
    el.scrollTop = Math.max(0, padT + inspect.fy * ph - el.clientHeight / 2)
    // Re-centre if the display's true-size figure arrives after opening.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [inspect, inspectScale])

  // ---- nudge + inspect toggle: one pointer gesture on the groups
  // container resolves to either. Tracked with `window`-level listeners
  // (never `setPointerCapture`, which retargets the eventual `click` to the
  // capturing element and would defeat a tile's own click handler) and
  // resolved entirely from `pointerup` rather than a native `click` — in
  // testing, a `click` immediately following an unrelated prior drag on the
  // same container was silently swallowed by the browser (no `click` event
  // at all, pointer events notwithstanding); pointerup doesn't have that
  // failure mode, so it — not `onClick` — is what opens/closes inspect.
  const [dragging, setDragging] = useState(false)
  const onGroupsPointerDown = (e: React.PointerEvent<HTMLDivElement>): void => {
    // Focus the preview so the arrow keys nudge right after a click.
    e.currentTarget.focus({ preventScroll: true })
    const target = e.target as HTMLElement
    const tileEl = target.closest<HTMLElement>('[data-testid="print-preview-page"]')
    const tileIndex = tileEl !== null ? Number(tileEl.dataset.index) : null
    const onInspectOverlay = target.closest('.hwprint__inspect') !== null
    const nudgeable = canNudge && inspect === null
    // Where on the tile the press landed (fractions), so inspect can open there.
    let fx = 0.5
    let fy = 0.5
    if (tileEl !== null) {
      const r = tileEl.getBoundingClientRect()
      fx = r.width > 0 ? (e.clientX - r.left) / r.width : 0.5
      fy = r.height > 0 ? (e.clientY - r.top) / r.height : 0.5
    }
    const scroller = previewScrollRef.current
    const pan = inspect !== null && scroller !== null ? { left: scroller.scrollLeft, top: scroller.scrollTop } : null
    const d = { x: e.clientX, y: e.clientY, moved: false, lastDxMm: 0, lastDyMm: 0 }
    if (nudgeable || pan !== null) setDragging(true)
    const onMove = (ev: PointerEvent): void => {
      const totalDx = ev.clientX - d.x
      const totalDy = ev.clientY - d.y
      if (Math.abs(totalDx) > 3 || Math.abs(totalDy) > 3) d.moved = true
      if (!d.moved) return
      if (pan !== null && scroller !== null) {
        // Inspecting: drag pans the 100 % page.
        scroller.scrollLeft = pan.left - totalDx
        scroller.scrollTop = pan.top - totalDy
        return
      }
      if (!nudgeable) return
      const pxPerMm = mmToPx(1, 96) * gridScaleRef.current
      if (pxPerMm <= 0) return
      const totalDxMm = totalDx / pxPerMm
      const totalDyMm = totalDy / pxPerMm
      c.nudge(totalDxMm - d.lastDxMm, totalDyMm - d.lastDyMm)
      d.lastDxMm = totalDxMm
      d.lastDyMm = totalDyMm
    }
    const onUp = (): void => {
      setDragging(false)
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onUp)
      if (d.moved) return
      if (onInspectOverlay) setInspect(null)
      else if (tileIndex !== null && !Number.isNaN(tileIndex)) setInspect({ index: tileIndex, fx, fy })
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onUp)
  }
  const onPreviewKeyDown = (e: React.KeyboardEvent<HTMLDivElement>): void => {
    if (!canNudge || inspect !== null) return
    const step = e.shiftKey ? 10 : 1
    const d: Record<string, [number, number]> = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }
    const v = d[e.key]
    if (v === undefined) return
    e.preventDefault()
    c.nudge(v[0], v[1])
  }
  const nudgeMm = plan?.layout.nudgeMm ?? { dx: 0, dy: 0 }

  const previewCaption =
    inspect !== null ? '100 % — drag to pan · click to go back' : opts.mode === 'scaled' && canNudge ? 'Drag the model to reposition · click a page to see it at 100 %' : 'Click a page to see it at 100 %'

  // The footer is quiet at rest — the preview and the controls already say
  // everything; it speaks only for progress, an error, or a warning.
  const summaryLine =
    status === 'rendering' && progress !== null
      ? `Rendering page ${Math.min(progress.done + 1, progress.total)} of ${progress.total}…`
      : status === 'error'
        ? 'Couldn’t open the system print dialog.'
        : ''
  const isWarning = status === 'idle' && job !== null && job.totalPages > MANY_PAGES_WARNING
  const isError = status === 'error'

  return (
    <div className="hwprint__overlay" onClick={onClose} data-testid="print-dialog-overlay">
      <style>{PRINT_CSS}</style>
      <div
        ref={(el) => {
          if (el !== null && !el.contains(document.activeElement)) el.querySelector<HTMLElement>('[data-testid="print-mode"] button[aria-pressed="true"]')?.focus()
        }}
        role="dialog"
        aria-modal="true"
        aria-label="Print Layout"
        className="hwprint"
        data-status={status}
        data-pages={job === null ? undefined : job.totalPages + c.cutPageCount}
        data-orientation={plan?.layout.page.orientation}
        data-scale={plan?.scaleText ?? undefined}
        data-summary={summary}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="hwprint__header">
          <div className="hwprint__title">Print Layout</div>
          <SegControl
            testId="print-mode"
            ariaLabel="Mode"
            mode
            value={opts.mode}
            onChange={(v) => c.switchMode(v as PrintOptions['mode'])}
            options={[
              { value: 'standard', label: 'Standard' },
              { value: 'scaled', label: 'Scaled' },
            ]}
          />
        </div>

        <div className="hwprint__body">
          {/* ---- preview (left) */}
          <div className="hwprint__preview-pane">
            {opts.mode === 'scaled' && isNudged && (
              <button
                type="button"
                className="hwprint__center-btn"
                onClick={c.centerNudge}
                title={`Offset ${Math.round(nudgeMm.dx)} mm, ${Math.round(nudgeMm.dy)} mm — click to center`}
                data-testid="print-nudge-readout"
              >
                Center
              </button>
            )}
            <div
              ref={previewScrollRef}
              data-testid="print-preview"
              tabIndex={0}
              aria-label="Page preview — arrow keys nudge the tile grid in Scaled mode"
              className={`hwprint__groups${inspect !== null ? ' hwprint__groups--inspect' : ''}`}
              style={{ justifyContent: multiGroup ? 'flex-start' : 'center', cursor: inspect !== null ? (dragging ? 'grabbing' : 'zoom-out') : canNudge ? (dragging ? 'grabbing' : 'grab') : 'default' }}
              onPointerDown={onGroupsPointerDown}
              onKeyDown={onPreviewKeyDown}
            >
              {inspectPage !== undefined ? (
                <div className="hwprint__inspect">
                  <div className="hwprint__inspect-page" style={{ width: mmToPx(inspectPage.layout.page.paper.w, 96) * inspectScale, height: mmToPx(inspectPage.layout.page.paper.h, 96) * inspectScale }}>
                    <PrintPage layout={inspectPage.layout} page={inspectPageModel ?? inspectPage.page} scale={inspectScale} />
                  </div>
                </div>
              ) : job === null || plan === null || job.empty ? (
                <div className="hwprint__empty">{job?.empty ? 'Nothing visible to print.' : 'Preparing…'}</div>
              ) : (
                groups.map((g, gi) => (
                  <div key={`${g.label ?? ''}#${gi}`} className="hwprint__group">
                    {g.label !== null && <div className="hwprint__group-label">{g.label}</div>}
                    <div className="hwprint__tile-grid" style={{ gridTemplateColumns: `repeat(${g.shown[0]?.layout.cols ?? 1}, auto)` }}>
                      {g.shown.map((pp) => (
                        <PreviewTile key={`${pp.group ?? ''}#${pp.page.tile.id}#${pp.index}`} pp={pp} scale={gridScale} />
                      ))}
                    </div>
                    {g.total.length > g.shown.length && <div className="hwprint__more-note">+{g.total.length - g.shown.length} more</div>}
                  </div>
                ))
              )}
            </div>
            <div className="hwprint__preview-caption">{previewCaption}</div>
          </div>

          {/* ---- controls (right, 302px) */}
          <div className="hwprint__controls" aria-busy={busy}>
            {opts.mode === 'scaled' && (
              <Section title="Scale">
                <Row label="View" htmlFor="print-view-select">
                  <select
                    id="print-view-select"
                    className="hwprint__select"
                    aria-label="View"
                    value={opts.view}
                    onChange={(e) => {
                      const view = e.target.value as PrintViewKind
                      // A standard view has no "current" frame: the Current view extent falls back to Model.
                      setOpts((o) => ({ ...o, view, extent: view !== 'current' && o.extent === 'view' ? 'model' : o.extent }))
                    }}
                  >
                    {(Object.keys(VIEW_LABEL) as PrintViewKind[]).map((v) => (
                      <option key={v} value={v}>{VIEW_LABEL[v]}</option>
                    ))}
                  </select>
                </Row>
                <Row label="Scale" htmlFor="print-scale-select">
                  <div className="hwprint__inline">
                    <select id="print-scale-select" className="hwprint__select" aria-label="Scale" style={{ flex: 1 }} value={scaleSelectValue} onChange={(e) => onScaleSelect(e.target.value)}>
                      {scaleRows.map((r) => (
                        <option key={r.value} value={r.value}>{r.label}</option>
                      ))}
                    </select>
                    <button type="button" className="hwprint__btn" onClick={fitScale} title="Largest standard scale that fits one page">Fit</button>
                  </div>
                </Row>
                {showCustomScaleUi && (
                  <Solo>
                    <div className="hwprint__inline hwprint__inline--wrap">
                      <input aria-label="On paper" className="hwprint__input" placeholder={LENGTH_SYSTEM_OF[format] === 'metric' ? '1 cm' : '1 in'} value={customScalePaper} onChange={(e) => editCustomPaper(e.target.value)} onBlur={commitCustomScale} onKeyDown={(e) => e.key === 'Enter' && commitCustomScale()} />
                      <span className="hwprint__inline-text">on paper =</span>
                      <input aria-label="In model" className="hwprint__input" placeholder={LENGTH_SYSTEM_OF[format] === 'metric' ? '10 cm' : '1 ft'} value={customScaleModel} onChange={(e) => editCustomModel(e.target.value)} onBlur={commitCustomScale} onKeyDown={(e) => e.key === 'Enter' && commitCustomScale()} />
                      <span className="hwprint__inline-text">in model</span>
                    </div>
                  </Solo>
                )}
                {showCustomScaleUi && customScaleErr !== null && <Solo className="hwprint__error">{customScaleErr}</Solo>}
                <Row label="Extent">
                  <SegControl
                    testId="print-extent"
                    ariaLabel="Extent"
                    value={opts.extent}
                    onChange={(v) => setOpts((o) => ({ ...o, extent: v as PrintOptions['extent'], view: v === 'view' ? 'current' : o.view }))}
                    options={[
                      { value: 'model', label: 'Model' },
                      { value: 'selection', label: 'Selection', disabled: !hasSelection, title: hasSelection ? 'Only the selected parts' : 'Nothing is selected.' },
                      { value: 'view', label: 'Current view', title: 'Exactly what the viewport frames right now (sets View: Current view)' },
                    ]}
                  />
                </Row>
              </Section>
            )}

            <Section title="Page">
              {opts.mode === 'standard' && (
                <Row label="Zoom">
                  <SegControl
                    testId="print-zoom"
                    ariaLabel="Zoom"
                    value={opts.zoom}
                    onChange={(v) => setOpts((o) => ({ ...o, zoom: v as PrintOptions['zoom'] }))}
                    options={[
                      { value: 'current', label: 'Current', title: 'The viewport\u2019s own framing' },
                      { value: 'fit', label: 'Fit', title: 'Fill the page with the model' },
                    ]}
                  />
                </Row>
              )}
              <Row label="Paper" htmlFor="print-paper-select">
                <select id="print-paper-select" className="hwprint__select" aria-label="Paper" value={paperIsCustom ? 'custom' : (opts.paper as PaperName)} onChange={(e) => onPaperSelect(e.target.value)}>
                  {PAPER_NAMES.map((n) => (
                    <option key={n} value={n}>{PAPER_LABEL[n]}</option>
                  ))}
                  <option value="custom">Custom…</option>
                </select>
              </Row>
              {paperIsCustom && (
                <Solo>
                  <div className="hwprint__inline">
                    <input aria-label="Paper width" className="hwprint__input" value={paperWText} onChange={(e) => setPaperWText(e.target.value)} onBlur={commitCustomPaper} onKeyDown={(e) => e.key === 'Enter' && commitCustomPaper()} />
                    <span className="hwprint__inline-text">×</span>
                    <input aria-label="Paper height" className="hwprint__input" value={paperHText} onChange={(e) => setPaperHText(e.target.value)} onBlur={commitCustomPaper} onKeyDown={(e) => e.key === 'Enter' && commitCustomPaper()} />
                  </div>
                </Solo>
              )}
              {paperIsCustom && paperErr !== null && <Solo className="hwprint__error">{paperErr}</Solo>}
              <Row label="Orientation">
                <SegControl
                  testId="print-orientation"
                  ariaLabel="Orientation"
                  value={opts.orientation}
                  onChange={(v) => setOpts((o) => ({ ...o, orientation: v as PrintOptions['orientation'] }))}
                  options={[
                    { value: 'auto', label: 'Auto' },
                    { value: 'portrait', label: 'Portrait' },
                    { value: 'landscape', label: 'Landscape' },
                  ]}
                />
              </Row>
              <Row label="Margins">
                <SegControl
                  testId="print-margins"
                  ariaLabel="Margins"
                  value={opts.margin}
                  onChange={(v) => setOpts((o) => ({ ...o, margin: v as PrintOptions['margin'] }))}
                  options={[
                    { value: 'normal', label: 'Normal', title: '½ in' },
                    { value: 'narrow', label: 'Narrow', title: '¼ in' },
                  ]}
                />
              </Row>
              {scenes.length > 0 && (
                <Row label="Pages" htmlFor="print-pages-select">
                  <span data-testid="print-pages">
                    <select id="print-pages-select" className="hwprint__select" aria-label="Pages" value={opts.pages} onChange={(e) => setOpts((o) => ({ ...o, pages: e.target.value as PrintOptions['pages'] }))}>
                      <option value="current">Current view</option>
                      <option value="each_scene">Each Scene ({scenes.length})</option>
                    </select>
                  </span>
                </Row>
              )}
            </Section>

            <Section title="Appearance">
              <Row label="Style">
                <SegControl
                  testId="print-style"
                  ariaLabel="Style"
                  value={opts.style}
                  onChange={(v) => setOpts((o) => ({ ...o, style: v as PrintOptions['style'] }))}
                  options={[
                    { value: 'shaded', label: 'As shown' },
                    { value: 'lineart', label: 'Line art' },
                  ]}
                />
              </Row>
              <Row label="Include" top>
                <div className="hwprint__checks">
                  <label className="hwprint__check">
                    <input type="checkbox" checked={opts.includeDimensions} onChange={(e) => setOpts((o) => ({ ...o, includeDimensions: e.target.checked }))} /> Dimensions &amp; text
                  </label>
                  <label className="hwprint__check">
                    <input type="checkbox" checked={opts.includeGuides} onChange={(e) => setOpts((o) => ({ ...o, includeGuides: e.target.checked }))} /> Guides
                  </label>
                  {opts.mode === 'standard' && (
                    <label className="hwprint__check">
                      <input type="checkbox" checked={opts.includeGridAxes} onChange={(e) => setOpts((o) => ({ ...o, includeGridAxes: e.target.checked }))} /> Grid &amp; axes
                    </label>
                  )}
                  {opts.mode === 'scaled' && (
                    <label className={`hwprint__check${opts.style !== 'lineart' ? ' hwprint__check--disabled' : ''}`}>
                      <input type="checkbox" disabled={opts.style !== 'lineart'} checked={opts.hiddenDashed} onChange={(e) => setOpts((o) => ({ ...o, hiddenDashed: e.target.checked }))} /> Hidden lines dashed
                    </label>
                  )}
                  <label className="hwprint__check">
                    <input type="checkbox" checked={opts.cutList} onChange={(e) => setOpts((o) => ({ ...o, cutList: e.target.checked }))} /> Cut list page
                  </label>
                  {opts.mode === 'standard' && (
                    <label className="hwprint__check">
                      <input type="checkbox" checked={opts.titleBlock} onChange={(e) => setOpts((o) => ({ ...o, titleBlock: e.target.checked }))} /> Title block
                    </label>
                  )}
                </div>
              </Row>
            </Section>

            {opts.mode === 'scaled' && (
              <Section title="On the page">
                <Solo>
                  <div className="hwprint__checks">
                    <label className="hwprint__check">
                      <input type="checkbox" checked={opts.overlap} onChange={(e) => setOpts((o) => ({ ...o, overlap: e.target.checked }))} /> Overlap for gluing
                    </label>
                    <label className="hwprint__check">
                      <input type="checkbox" checked={opts.marks} onChange={(e) => setOpts((o) => ({ ...o, marks: e.target.checked }))} /> Marks
                    </label>
                    <label className="hwprint__check">
                      <input type="checkbox" checked={opts.scaleBar} onChange={(e) => setOpts((o) => ({ ...o, scaleBar: e.target.checked }))} /> Scale bar
                    </label>
                    <label className="hwprint__check">
                      <input type="checkbox" checked={opts.titleBlock} onChange={(e) => setOpts((o) => ({ ...o, titleBlock: e.target.checked }))} /> Title block
                    </label>
                  </div>
                </Solo>
              </Section>
            )}
          </div>
        </div>

        {/* ---- footer */}
        <div className="hwprint__footer">
          <div className="hwprint__footer-summary">
            <div data-testid="print-summary" className={`hwprint__summary${isWarning ? ' hwprint__summary--warning' : ''}${isError ? ' hwprint__summary--danger' : ''}`}>
              {summaryLine !== '' ? summaryLine : isWarning ? summary : ''}
            </div>
            {job !== null && (job.warnings.length > 0 || vectorNote !== null) && status === 'idle' && (
              <div className="hwprint__warnings" data-testid="print-warnings">{[...job.warnings, ...(vectorNote !== null ? [vectorNote] : [])].join(' · ')}</div>
            )}
            {isError && hasPrintFallback && (
              <button type="button" className="hwprint__link-btn" onClick={() => void c.doPrintFallback()}>
                Open the browser print dialog instead
              </button>
            )}
          </div>
          <div className="hwprint__footer-actions">
            <button type="button" className="hwprint__btn hwprint__btn--ghost" onClick={onClose}>{status === 'sent' || status === 'saved' ? 'Done' : 'Cancel'}</button>
            <button type="button" className="hwprint__btn hwprint__btn--savepdf" onClick={() => void c.doSavePdf()} disabled={!canPrint} data-testid="print-save-pdf">Save PDF…</button>
            <button type="button" className="hwprint__btn hwprint__btn--print" onClick={() => void c.doPrint()} disabled={!canPrint} data-testid="print-confirm">Print…</button>
          </div>
        </div>
      </div>
      {printed !== null && <PrintRoot layout={printed.layout} pages={printed.pages} title={printed.title} />}
    </div>
  )
}

const PRINT_CSS = `
.hwprint__overlay {
  position: fixed;
  inset: 0;
  background: var(--backdrop-dim);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 2000;
}
.hwprint {
  display: flex;
  flex-direction: column;
  /* A large share of the Hew window (it is a fixed-size sheet, not a
   * resizable window), capped only for very large displays. */
  width: min(2200px, calc(100vw - 120px));
  height: min(1400px, calc(100vh - 100px));
  background: var(--surface-panel);
  border: 1px solid var(--border-panel);
  border-radius: 14px;
  box-shadow: var(--shadow-window);
  font-family: var(--font-family-ui);
  color: var(--text-secondary);
  overflow: hidden;
}

/* ---- Header ------------------------------------------------------------ */
.hwprint__header {
  flex: 0 0 48px;
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 0 18px;
  border-bottom: 1px solid var(--border-panel);
}
.hwprint__title {
  font-size: 13.5px;
  font-weight: 600;
  color: var(--text-primary);
}
.hwprint__kbd-hints {
  margin-left: auto;
  display: flex;
  gap: 12px;
  font-size: 10px;
  color: var(--print-text-muted);
  white-space: nowrap;
}

/* ---- Segmented controls -------------------------------------------------*/
.hwprint__seg {
  display: inline-flex;
  min-width: 0;
  background: var(--surface-input);
  border: 1px solid var(--border-panel);
  border-radius: 7px;
  padding: 2px;
  gap: 2px;
}
.hwprint__seg-btn {
  flex: 1 1 auto;
  padding: 3px 8px;
  border: none;
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  font-family: inherit;
  font-size: 11.5px;
  white-space: nowrap;
  cursor: pointer;
}
.hwprint__seg-btn[aria-pressed='true'] {
  background: var(--seg-active);
  color: var(--text-primary);
  font-weight: 600;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.18);
}
.hwprint__seg-btn:disabled {
  opacity: 0.45;
  cursor: default;
}
.hwprint__seg--mode .hwprint__seg-btn {
  padding: 4px 14px;
  font-size: 12.5px;
}

/* ---- Body / preview ------------------------------------------------------*/
.hwprint__body {
  flex: 1;
  display: flex;
  min-height: 0;
}
.hwprint__preview-pane {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  background: var(--surface-desk);
  position: relative;
}
.hwprint__persp-note {
  padding: 10px 16px 0;
  font-size: 12px;
  color: var(--print-text-muted);
  text-align: center;
}
.hwprint__center-btn {
  position: absolute;
  top: 10px;
  right: 12px;
  z-index: 2;
  background: var(--surface-input);
  color: var(--text-primary);
  border: 1px solid var(--border-panel);
  border-radius: 7px;
  padding: 3px 10px;
  font-size: 11px;
  cursor: pointer;
}
.hwprint__groups {
  flex: 1;
  overflow: auto;
  padding: 14px 16px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 16px;
  min-height: 0;
  touch-action: none;
  outline: none;
}
.hwprint__group {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
}
.hwprint__group-label {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--print-text-muted);
}
.hwprint__tile-grid {
  display: grid;
  gap: 12px 10px;
  justify-items: center;
}
.hwprint__tile-col {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 3px;
}
.hwprint__tile {
  position: relative;
  overflow: hidden;
  background: #ffffff;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.3);
  cursor: zoom-in;
  flex-shrink: 0;
}
.hwprint__tile--pending {
  background: transparent;
  border: 1px dashed var(--print-text-muted);
  opacity: 0.6;
  box-shadow: none;
  cursor: default;
}
.hwprint__tile-id {
  font-size: 10px;
  color: var(--print-text-muted);
}
.hwprint__more-note {
  font-size: 11px;
  color: var(--print-text-muted);
}
.hwprint__empty {
  margin: auto;
  color: var(--print-text-muted);
  padding: 24px;
}
.hwprint__preview-caption {
  padding: 0 16px 10px;
  font-size: 11px;
  color: var(--print-text-muted);
  text-align: center;
  flex-shrink: 0;
}
.hwprint__groups--inspect {
  /* Inspecting: a scroll box around a real-size page (block layout, so the
   * page's top-left is reachable — a centred flex child of an overflowing
   * box clips at the start). */
  display: block;
  padding: 24px;
}
.hwprint__inspect {
  /* Sized to the page so the scroll box has the whole sheet to scroll over. */
  display: inline-block;
  min-width: 100%;
  text-align: center;
}
.hwprint__inspect-page {
  position: relative;
  display: inline-block;
  background: #ffffff;
  box-shadow: 0 8px 30px rgba(0, 0, 0, 0.35);
  overflow: hidden;
  text-align: left;
}

/* ---- Controls (right, 302px) --------------------------------------------*/
.hwprint__controls {
  width: 302px;
  flex-shrink: 0;
  border-left: 1px solid var(--border-panel);
  overflow-y: auto;
  padding: 14px 16px;
  display: flex;
  flex-direction: column;
  gap: 18px;
}
.hwprint__controls[aria-busy='true'] {
  pointer-events: none;
  opacity: 0.6;
}
.hwprint__section-head {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--print-text-muted);
  margin-bottom: 9px;
}
.hwprint__grid {
  display: grid;
  grid-template-columns: 80px minmax(0, 1fr);
  row-gap: 9px;
  column-gap: 10px;
  align-items: center;
}
.hwprint__label {
  text-align: right;
  font-size: 12px;
  color: var(--print-text-muted);
}
.hwprint__label--top {
  align-self: start;
  padding-top: 4px;
}
.hwprint__control {
  min-width: 0;
}
.hwprint__solo {
  grid-column: 2;
  min-width: 0;
}
.hwprint__select {
  width: 100%;
  background: var(--surface-input);
  color: var(--text-primary);
  border: 1px solid var(--border-panel);
  border-radius: 7px;
  padding: 4px 8px;
  font-size: 12px;
  font-family: inherit;
}
.hwprint__input {
  width: 62px;
  background: var(--surface-input);
  color: var(--text-primary);
  border: 1px solid var(--border-panel);
  border-radius: 7px;
  padding: 4px 6px;
  font-size: 12px;
  text-align: center;
  font-family: inherit;
}
.hwprint__inline {
  display: flex;
  align-items: center;
  gap: 6px;
}
.hwprint__inline--wrap {
  flex-wrap: wrap;
}
.hwprint__inline-text {
  font-size: 12px;
  color: var(--print-text-muted);
  white-space: nowrap;
}
.hwprint__btn {
  background: var(--surface-input);
  color: var(--text-primary);
  border: 1px solid var(--border-panel);
  border-radius: 7px;
  padding: 4px 10px;
  font-size: 12px;
  font-family: inherit;
  cursor: pointer;
  white-space: nowrap;
}
.hwprint__error {
  font-size: 11px;
  color: var(--danger-base);
}
.hwprint__hint {
  font-size: 11px;
  color: var(--print-text-muted);
}
.hwprint__reading {
  font-size: 12px;
  color: var(--print-text-muted);
}
.hwprint__checks {
  display: flex;
  flex-direction: column;
  gap: 7px;
}
.hwprint__check {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 12px;
  color: var(--text-primary);
  cursor: pointer;
}
.hwprint__check input {
  accent-color: var(--accent-base);
  margin: 0;
}
.hwprint__check--disabled {
  opacity: 0.45;
  cursor: default;
}

/* ---- Footer --------------------------------------------------------------*/
.hwprint__footer {
  flex: 0 0 49px;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 0 16px;
  border-top: 1px solid var(--border-panel);
}
.hwprint__footer-summary {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: baseline;
  gap: 10px;
}
.hwprint__summary {
  font-size: 12px;
  color: var(--print-text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.hwprint__summary--warning {
  color: var(--status-warning);
  font-weight: 600;
}
.hwprint__summary--danger {
  color: var(--danger-base);
  font-weight: 600;
  white-space: normal;
}
.hwprint__warnings {
  font-size: 12px;
  color: var(--status-warning);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.hwprint__link-btn {
  background: none;
  border: none;
  padding: 0;
  color: var(--accent-base);
  font-size: 12px;
  cursor: pointer;
  text-decoration: underline;
  white-space: nowrap;
}
.hwprint__footer-actions {
  margin-left: auto;
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}
.hwprint__btn--ghost {
  background: transparent;
}
.hwprint__btn--savepdf {
  background: transparent;
  color: var(--accent-base);
  font-weight: 600;
}
[data-theme='light'] .hwprint__btn--savepdf {
  color: var(--accent-strong);
}
.hwprint__btn--print {
  background: var(--accent-strong);
  color: #ffffff;
  border-color: transparent;
  font-weight: 600;
}
.hwprint__footer-actions .hwprint__btn:disabled {
  opacity: 0.6;
  cursor: default;
}
`
