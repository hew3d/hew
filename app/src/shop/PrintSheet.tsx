/**
 * PrintSheet — Shop Mode's Print sheet (design_handoff_printing SPEC.md §4
 * "The phone Print sheet"): portrait bottom sheet / landscape centered card,
 * built on the SAME `usePrintController` brain the desktop dialog
 * (`panels/PrintDialog.tsx`) uses — pages/options/job planning/rendering are
 * identical, only the chrome differs. Document-only: nothing here mutates
 * the model.
 *
 * Deliberately minimal versus the desktop dialog (SPEC.md §4): no nudge, no
 * Each-Scene grouping, no custom paper, no orientation/margins/include
 * toggles — those keep whatever value desktop last set (shared
 * `settings/print.ts` prefs) and are simply not exposed here. Save PDF… is
 * the filled PRIMARY action on the phone (reversed from desktop) because
 * AirPrint can't honor Hew's composed paper — only the PDF is exact
 * (SPEC.md §1 decision #4).
 *
 * `PrintSheet` itself is a thin gate — `open` unmounts `PrintSheetPanel`
 * entirely (matching `ViewsSheet`/`UnitPicker`'s own `open` prop shape) so
 * the controller's debounced preview-render effect, which calls
 * `ViewportApi.renderPrintPages` on a timer, never runs while the sheet is
 * closed.
 */
import { useEffect, useState } from 'react'
import type { CSSProperties } from 'react'
import { PrintPage, PrintRoot } from '../print/PrintDocument'
import { mmToPx } from '../print/layout'
import { PAPER_LABEL, PAPER_NAMES, paperLabel, type PaperName } from '../print/paper'
import { scaleDisplay, scaleHint } from '../print/scale'
import { usePrintController, type PrintControllerProps } from '../print/usePrintController'
import type { ShopOrientation } from './orientation'

export interface PrintSheetProps extends Omit<PrintControllerProps, 'compact'> {
  open: boolean
  /** Portrait: bottom sheet. Landscape: centered 620×≤350 card (SPEC.md §4
   *  decision #15), same fork every other Shop Mode sheet uses. */
  orientation: ShopOrientation
  onClose: () => void
}

const FONT = 'var(--font-family-ui)'

const rowButtonStyle: CSSProperties = {
  display: 'flex', alignItems: 'center', justifyContent: 'space-between', width: '100%',
  minHeight: '48px', padding: '0 2px', background: 'transparent', border: 'none',
  borderBottom: '1px solid var(--shop-hairline-2)', cursor: 'pointer', textAlign: 'left',
  fontFamily: FONT, fontSize: '16px', color: 'var(--shop-text)',
}
const rowDivStyle: CSSProperties = { ...rowButtonStyle, cursor: 'default' }
const rowValueStyle: CSSProperties = { color: 'var(--shop-text-faint)', fontSize: '15px', fontFamily: FONT }
// The Fit chip and the in-row segmented chips keep the prototype's compact
// look but carry a ≥ 44 px hit area (SPEC §4 "All targets ≥ 48 px" — the
// 48 px row itself is the target; the chips fill it minus the hairline).
const fitChipStyle: CSSProperties = {
  background: 'var(--shop-eyebg)', color: 'var(--shop-text)', border: 'none', borderRadius: '9px',
  minHeight: '44px', minWidth: '48px', padding: '0 14px', fontSize: '13px', fontWeight: 600, cursor: 'pointer', fontFamily: FONT,
}
const segWrapStyle: CSSProperties = { display: 'flex', background: 'var(--shop-eyebg)', borderRadius: '14px', padding: '3px', gap: '3px', marginTop: '14px' }
const miniSegWrapStyle: CSSProperties = { display: 'flex', background: 'var(--shop-eyebg)', borderRadius: '10px', padding: '3px', gap: '3px' }
function segChipStyle(active: boolean): CSSProperties {
  return {
    flex: 1, minHeight: '44px', border: 'none', borderRadius: '12px', fontSize: '15px', fontWeight: 600,
    background: active ? 'var(--shop-accent-fill)' : 'transparent', color: active ? 'var(--shop-on-accent)' : 'var(--shop-text)',
    cursor: 'pointer', fontFamily: FONT, whiteSpace: 'nowrap',
  }
}
function miniSegChipStyle(active: boolean): CSSProperties {
  return {
    flex: 1, minHeight: '42px', border: 'none', borderRadius: '8px', fontSize: '12.5px', fontWeight: 600,
    background: active ? 'var(--shop-accent-fill)' : 'transparent', color: active ? 'var(--shop-on-accent)' : 'var(--shop-text-faint)',
    cursor: 'pointer', padding: '0 10px', fontFamily: FONT, whiteSpace: 'nowrap',
  }
}
function pickerItemStyle(active: boolean): CSSProperties {
  return {
    display: 'flex', alignItems: 'center', justifyContent: 'space-between', width: '100%', minHeight: '48px',
    padding: '0 10px', margin: '2px 0', borderRadius: '10px', textAlign: 'left', cursor: 'pointer', border: 'none',
    background: active ? 'var(--shop-picker-active-wash)' : 'transparent',
    fontFamily: FONT, fontSize: '15px', fontWeight: active ? 600 : 500, color: 'var(--shop-text)',
  }
}
const customInputStyle: CSSProperties = {
  width: '92px', minHeight: '40px', padding: '0 8px', borderRadius: '9px', border: '1px solid var(--shop-hairline)',
  background: 'var(--shop-eyebg)', color: 'var(--shop-text)', fontSize: '14px', fontFamily: FONT,
}
function saveButtonStyle(enabled: boolean): CSSProperties {
  return {
    width: '100%', minHeight: '50px', marginTop: '14px', background: 'var(--shop-accent-fill)', color: 'var(--shop-on-accent)',
    border: 'none', borderRadius: '14px', fontSize: '17px', fontWeight: 600, fontFamily: FONT,
    cursor: enabled ? 'pointer' : 'not-allowed', opacity: enabled ? 1 : 0.5,
  }
}
function printButtonStyle(enabled: boolean): CSSProperties {
  return {
    width: '100%', minHeight: '50px', marginTop: '10px', background: 'transparent', color: 'var(--shop-accent-text)',
    border: '1.5px solid var(--shop-hairline)', borderRadius: '14px', fontSize: '17px', fontWeight: 600, fontFamily: FONT,
    cursor: enabled ? 'pointer' : 'not-allowed', opacity: enabled ? 1 : 0.5,
  }
}

export function PrintSheet({ open, ...rest }: PrintSheetProps) {
  if (!open) return null
  return <PrintSheetPanel {...rest} />
}

function PrintSheetPanel({ orientation, onClose, ...controllerProps }: Omit<PrintSheetProps, 'open'>) {
  const c = usePrintController({ ...controllerProps, compact: true })
  const { opts, setOpts, job, status, progress, error, busy, canPrint, previewPages, printed, presets, presetIndex, format } = c

  const [paperOpen, setPaperOpen] = useState(false)
  const [scaleOpen, setScaleOpen] = useState(false)
  const [showCustomScale, setShowCustomScale] = useState(opts.scale.kind === 'custom')
  const [customPaperText, setCustomPaperText] = useState('')
  const [customModelText, setCustomModelText] = useState('')
  const [customScaleError, setCustomScaleError] = useState<string | null>(null)

  useEffect(() => {
    const onKey = (ev: KeyboardEvent): void => {
      if (ev.key === 'Escape') {
        ev.preventDefault()
        onClose()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  const applyCustom = (): void => setCustomScaleError(c.applyCustomScale(customPaperText, customModelText))

  const isLandscape = orientation === 'landscape'
  const paperIsNamed = typeof opts.paper === 'string'
  const busyStyle: CSSProperties = busy ? { pointerEvents: 'none', opacity: 0.6 } : {}

  const modeSwitch = (
    <div role="group" aria-label="Mode" style={segWrapStyle}>
      {(['standard', 'scaled'] as const).map((m) => (
        <button key={m} type="button" aria-pressed={opts.mode === m} onClick={() => c.switchMode(m)} style={segChipStyle(opts.mode === m)}>
          {m === 'standard' ? 'Standard' : 'Scaled'}
        </button>
      ))}
    </div>
  )

  const paperRow = (
    <>
      <button type="button" aria-label="Paper" aria-expanded={paperOpen} onClick={() => setPaperOpen((v) => !v)} style={rowButtonStyle}>
        <span>Paper</span>
        <span style={rowValueStyle}>{paperLabel(opts.paper)} ›</span>
      </button>
      {paperOpen && (
        <div role="listbox" aria-label="Paper size" style={{ padding: '2px 0 6px' }}>
          {PAPER_NAMES.map((n: PaperName) => {
            const active = paperIsNamed && opts.paper === n
            return (
              <button
                key={n}
                type="button"
                aria-selected={active}
                onClick={() => {
                  setOpts((o) => ({ ...o, paper: n }))
                  setPaperOpen(false)
                }}
                style={pickerItemStyle(active)}
              >
                {PAPER_LABEL[n]}
              </button>
            )
          })}
        </div>
      )}
    </>
  )

  const scaleRow = opts.mode !== 'scaled' ? null : (
    <>
      <div style={rowDivStyle}>
        <span>Scale</span>
        <div style={{ display: 'flex', alignItems: 'center', gap: '10px' }}>
          <button type="button" onClick={() => c.fit()} style={fitChipStyle}>Fit</button>
          <button type="button" aria-label="Scale" aria-expanded={scaleOpen} onClick={() => setScaleOpen((v) => !v)} style={{ background: 'transparent', border: 'none', cursor: 'pointer', padding: 0, ...rowValueStyle }}>
            {scaleDisplay(opts.scale)} ›
          </button>
        </div>
      </div>
      {scaleOpen && (
        <div role="listbox" aria-label="Scale preset" style={{ padding: '2px 0 6px' }}>
          {presets.map((p, i) => {
            const active = presetIndex === i && !showCustomScale
            const hint = scaleHint(p, format)
            return (
              <button
                key={p.label}
                type="button"
                aria-selected={active}
                onClick={() => {
                  setOpts((o) => ({ ...o, scale: p }))
                  setShowCustomScale(false)
                  setScaleOpen(false)
                }}
                style={pickerItemStyle(active)}
              >
                {hint === null ? scaleDisplay(p) : `${p.label} (${hint})`}
              </button>
            )
          })}
          <button
            type="button"
            aria-selected={showCustomScale}
            onClick={() => {
              setShowCustomScale(true)
              setScaleOpen(false)
            }}
            style={pickerItemStyle(showCustomScale)}
          >
            Custom…
          </button>
        </div>
      )}
      {showCustomScale && (
        <div style={{ padding: '8px 2px 10px', display: 'flex', flexDirection: 'column', gap: '6px' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px', fontSize: '14px', color: 'var(--shop-text)', fontFamily: FONT }}>
            <input aria-label="On paper" value={customPaperText} placeholder="1 cm" onChange={(e) => setCustomPaperText(e.target.value)} onBlur={applyCustom} onKeyDown={(e) => e.key === 'Enter' && applyCustom()} style={customInputStyle} />
            <span>on paper =</span>
            <input aria-label="In model" value={customModelText} placeholder="10 cm" onChange={(e) => setCustomModelText(e.target.value)} onBlur={applyCustom} onKeyDown={(e) => e.key === 'Enter' && applyCustom()} style={customInputStyle} />
          </div>
          <div style={{ fontSize: '12.5px', fontFamily: FONT, color: customScaleError !== null ? 'var(--danger-base, #e5484d)' : 'var(--shop-text-faint)' }}>
            {customScaleError ?? `Now: ${scaleDisplay(opts.scale)}`}
          </div>
        </div>
      )}
      <div style={rowDivStyle}>
        <span>Extent</span>
        <div role="group" aria-label="Extent" style={miniSegWrapStyle}>
          {(['model', 'view'] as const).map((v) => (
            <button key={v} type="button" aria-pressed={opts.extent === v} onClick={() => setOpts((o) => ({ ...o, extent: v }))} style={miniSegChipStyle(opts.extent === v)}>
              {v === 'model' ? 'Model' : 'Current view'}
            </button>
          ))}
        </div>
      </div>
    </>
  )

  const styleRow = (
    <div style={rowDivStyle}>
      <span>Style</span>
      <div role="group" aria-label="Style" style={miniSegWrapStyle}>
        {(['shaded', 'lineart'] as const).map((v) => (
          <button key={v} type="button" aria-pressed={opts.style === v} onClick={() => setOpts((o) => ({ ...o, style: v }))} style={miniSegChipStyle(opts.style === v)}>
            {v === 'shaded' ? 'As shown' : 'Line art'}
          </button>
        ))}
      </div>
    </div>
  )

  const stripHeight = isLandscape ? 40 : 46
  const shownPages = previewPages.slice(0, 5)
  const extraPages = previewPages.length - shownPages.length
  const strip = (
    <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginTop: isLandscape ? 0 : '14px', minHeight: `${stripHeight + 4}px` }}>
      {shownPages.map(({ layout, page, index, group }) => {
        const hPxFull = mmToPx(layout.page.paper.h, 96)
        const wPxFull = mmToPx(layout.page.paper.w, 96)
        const scale = stripHeight / hPxFull
        const wPx = wPxFull * scale
        return (
          <div key={`${group ?? ''}#${page.tile.id}#${index}`} data-testid="print-sheet-strip-page" style={{ width: `${wPx}px`, height: `${stripHeight}px`, borderRadius: '2px', boxShadow: '0 1px 3px rgba(0,0,0,.25)', overflow: 'hidden', position: 'relative', flexShrink: 0 }}>
            <PrintPage layout={layout} page={page} scale={scale} />
          </div>
        )
      })}
      {extraPages > 0 && <span style={{ fontSize: '12px', fontWeight: 600, color: 'var(--shop-text-faint)', fontFamily: FONT }}>+{extraPages}</span>}
    </div>
  )

  // Quiet at rest — the strip and the rows already say it all; the line
  // speaks only for progress, an empty document, or an error.
  const statusText =
    status === 'rendering' && progress !== null
      ? `Rendering page ${Math.min(progress.done + 1, progress.total)} of ${progress.total}…`
      : status === 'error'
        ? `Couldn't print: ${error ?? 'unknown error'}`
        : ''

  const summaryText = job !== null && job.empty ? 'Nothing visible to print.' : statusText
  const summaryBlock =
    summaryText === '' ? null : (
      <div data-testid="print-sheet-summary" style={{ fontSize: isLandscape ? '12px' : '13px', color: status === 'error' ? 'var(--danger-base, #e5484d)' : 'var(--shop-text-faint)', marginTop: isLandscape ? '4px' : '6px', fontFamily: FONT }}>
        {summaryText}
      </div>
    )

  const buttons = (
    <>
      <button type="button" disabled={!canPrint} onClick={() => void c.doSavePdf()} style={saveButtonStyle(canPrint)}>Save PDF…</button>
      <button type="button" disabled={!canPrint} onClick={() => void c.doPrint()} style={printButtonStyle(canPrint)}>Print…</button>
    </>
  )

  const dialogRef = (el: HTMLDivElement | null): void => {
    if (el !== null && !el.contains(document.activeElement)) el.querySelector<HTMLElement>('button[aria-pressed]')?.focus()
  }

  const controlsBody = (
    <div style={{ display: 'flex', flexDirection: 'column' }}>
      {modeSwitch}
      <div style={{ display: 'flex', flexDirection: 'column', marginTop: '6px' }}>
        {paperRow}
        {scaleRow}
        {styleRow}
      </div>
    </div>
  )

  return (
    <>
      <div data-testid="shop-print-scrim" aria-hidden="true" onClick={onClose} style={{ position: 'absolute', inset: 0, background: 'rgba(27,26,23,.35)', zIndex: 55 }} />
      {isLandscape ? (
        <div
          ref={dialogRef}
          role="dialog"
          aria-modal="true"
          aria-label="Print Layout"
          data-status={status}
          data-testid="print-sheet-card"
          style={{
            position: 'absolute', left: '50%', top: '50%', transform: 'translate(-50%, -50%)',
            width: '620px', maxHeight: '350px', zIndex: 56,
            background: 'var(--surface-sheet)', borderRadius: '24px', padding: '12px 22px 18px',
            boxShadow: '0 16px 50px -14px rgba(27,26,23,.5)', display: 'flex', flexDirection: 'column', overflow: 'hidden',
          }}
        >
          <div style={{ width: '44px', height: '5px', borderRadius: '3px', background: 'var(--shop-hairline)', margin: '0 auto' }} />
          <div style={{ display: 'flex', gap: '22px', marginTop: '10px', minHeight: 0, flex: 1, ...busyStyle }} aria-busy={busy}>
            <div style={{ flex: 1.15, display: 'flex', flexDirection: 'column', minWidth: 0, overflowY: 'auto' }}>
              <div style={{ fontSize: '19px', fontWeight: 700, color: 'var(--shop-text)', fontFamily: FONT }}>Print Layout</div>
              {controlsBody}
            </div>
            <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0 }}>
              {strip}
              {summaryBlock}
              <div style={{ flex: 1 }} />
              {buttons}
            </div>
          </div>
        </div>
      ) : (
        <div
          ref={dialogRef}
          role="dialog"
          aria-modal="true"
          aria-label="Print Layout"
          data-status={status}
          data-testid="print-sheet-sheet"
          style={{
            position: 'absolute', left: 0, right: 0, bottom: 0, zIndex: 56, maxHeight: '92vh', overflowY: 'auto',
            background: 'var(--surface-sheet)', borderRadius: '24px 24px 0 0', padding: '10px 20px 0',
            boxShadow: '0 -14px 40px -12px rgba(27,26,23,.5)',
          }}
        >
          <div style={{ width: '44px', height: '5px', borderRadius: '3px', background: 'var(--shop-hairline)', margin: '0 auto' }} />
          <div style={{ fontSize: '21px', fontWeight: 700, color: 'var(--shop-text)', marginTop: '12px', fontFamily: FONT }}>Print Layout</div>
          <div style={busyStyle} aria-busy={busy}>
            {controlsBody}
            {strip}
            {summaryBlock}
            {buttons}
          </div>
          <div style={{ height: '22px' }} />
          <div style={{ width: '134px', height: '5px', borderRadius: '3px', background: 'var(--shop-hairline)', margin: '0 auto' }} />
          <div style={{ height: 'max(6px, env(safe-area-inset-bottom))' }} />
        </div>
      )}
      {printed !== null && <PrintRoot layout={printed.layout} pages={printed.pages} title={printed.title} />}
    </>
  )
}
