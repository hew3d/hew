/**
 * ImportingOverlay — shown while a synchronous COLLADA import is in progress.
 *
 * Because `scene.import_dae()` blocks the main thread, the CSS spinner
 * animation will freeze during the actual parse.  That is intentional and
 * honest: the overlay still communicates that something is happening.
 *
 * The overlay is painted *before* the blocking call via a double rAF in
 * App.tsx (two requestAnimationFrame callbacks back-to-back ensure a real
 * paint has occurred before the synchronous work begins).
 *
 * TODO(future): true smooth progress requires running import_dae in a Web
 * Worker so the main thread stays free.  The worker would need a SharedArrayBuffer
 * channel to the WASM module, which is a non-trivial change to wasm-api.
 */

import type React from 'react'
import { plural, t } from '@lingui/core/macro'
import { Trans, useLingui } from '@lingui/react/macro'
import { formatNumber } from '../i18n/format'

/**
 * How far an open has come, for the overlay's bar and its line of text.
 * `read` is the file being read (one step that cannot be split, so no
 * count); `shapes` the file's placements being read into shapes (`done`
 * of `total` placements, `shapes` built so far); `objects` the document
 * being built from them; `finishing` the one step that completes the
 * document (health, guides, drawing, scenes); `source` the one step that
 * keeps the file for saving back; `view` the viewport getting ready.
 */
export interface OpenProgress {
  phase: 'read' | 'shapes' | 'objects' | 'finishing' | 'source' | 'view'
  done: number
  total: number
  /** Shapes built so far (the `shapes` phase). */
  shapes?: number
  /** The file's size in bytes (the `read` phase). */
  bytes?: number
}

interface ImportingOverlayProps {
  /** Display name of the file being imported. */
  fileName: string
  /** File ▸ Open of a file that becomes the document's own (a .skp):
   * "Opening", not "Importing". */
  opening?: boolean
  /** How far the open has come; without it the overlay only says that
   * the file is being read. */
  progress?: OpenProgress | null
}

/** Where each phase starts and ends on the one bar, as a share of the
 * whole open: measured on a 395 MB model in the browser (read 10 s,
 * shapes 18 s, objects 6 s, finishing 5 s, source 4 s, view 6 s),
 * rounded. */
const PHASE_SPAN: Record<OpenProgress['phase'], [number, number]> = {
  read: [0, 0.2],
  shapes: [0.2, 0.56],
  objects: [0.56, 0.68],
  finishing: [0.68, 0.78],
  source: [0.78, 0.86],
  view: [0.86, 1],
}

/** The bar's fill, 0–1, for a progress report. */
export function openProgressFraction(p: OpenProgress): number {
  const [from, to] = PHASE_SPAN[p.phase]
  const within = p.total > 0 ? Math.min(1, p.done / p.total) : 0
  return from + (to - from) * within
}

function formatCount(n: number): string {
  return formatNumber(n)
}

function formatBytes(n: number): string {
  if (n >= 1_000_000_000) {
    const gb = formatNumber(n / 1_000_000_000, { fractionDigits: 1 })
    return t`${gb} GB`
  }
  if (n >= 1_000_000) {
    const mb = formatNumber(Math.round(n / 1_000_000))
    return t`${mb} MB`
  }
  if (n >= 1_000) {
    const kb = formatNumber(Math.round(n / 1_000))
    return t`${kb} KB`
  }
  return plural(n, { one: '# byte', other: '# bytes' })
}

/** The line under the bar: what is happening and how far it has come. */
export function openProgressText(p: OpenProgress): string {
  switch (p.phase) {
    case 'read':
      if (p.bytes !== undefined) {
        const size = formatBytes(p.bytes)
        return t`Reading the file (${size})`
      }
      return t`Reading the file`
    case 'shapes': {
      const done = formatCount(p.done)
      const total = formatCount(p.total)
      if (p.shapes) {
        const shapes = formatCount(p.shapes)
        return t`Reading placements: ${done} of ${total}, ${shapes} shapes so far`
      }
      return t`Reading placements: ${done} of ${total}`
    }
    case 'objects': {
      const done = formatCount(p.done)
      const total = formatCount(p.total)
      return t`Building objects: ${done} of ${total}`
    }
    case 'finishing':
      return t`Checking the objects`
    case 'source':
      return t`Getting ready to save back to the file`
    case 'view':
      return t`Getting the view ready`
  }
}

const OVERLAY_STYLE: React.CSSProperties = {
  position: 'fixed',
  inset: 0,
  background: 'var(--backdrop-dim)',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  zIndex: 2000,
}

const CARD_STYLE: React.CSSProperties = {
  background: 'var(--surface-window)',
  border: '1px solid var(--border-strong)',
  borderRadius: '6px',
  boxShadow: 'var(--shadow-window)',
  padding: '28px 32px',
  minWidth: '300px',
  maxWidth: '480px',
  fontFamily: 'var(--font-family-ui)',
  color: 'var(--text-secondary)',
  display: 'flex',
  flexDirection: 'column',
  alignItems: 'center',
  gap: '16px',
}

const SPINNER_STYLE: React.CSSProperties = {
  width: '32px',
  height: '32px',
  border: '3px solid var(--border-strong)',
  borderTopColor: 'var(--accent-base)',
  borderRadius: '50%',
  animation: 'hew-spin 0.8s linear infinite',
}

const MESSAGE_STYLE: React.CSSProperties = {
  fontSize: '14px',
  color: 'var(--text-secondary)',
  textAlign: 'center',
  lineHeight: '1.5',
}

const FILE_NAME_STYLE: React.CSSProperties = {
  color: 'var(--text-primary)',
  fontWeight: 600,
}

const HINT_STYLE: React.CSSProperties = {
  fontSize: '11px',
  color: 'var(--text-faint)',
  textAlign: 'center',
}

const BAR_STYLE: React.CSSProperties = {
  width: '100%',
  height: '6px',
  borderRadius: '3px',
  background: 'var(--border-strong)',
  overflow: 'hidden',
}

const BAR_FILL_STYLE: React.CSSProperties = {
  height: '100%',
  background: 'var(--accent-base)',
  borderRadius: '3px',
  transition: 'width 120ms linear',
}

const PROGRESS_LINE_STYLE: React.CSSProperties = {
  fontSize: '12px',
  color: 'var(--text-secondary)',
  textAlign: 'center',
  display: 'flex',
  justifyContent: 'space-between',
  width: '100%',
  gap: '12px',
  fontVariantNumeric: 'tabular-nums',
}

// Inject the keyframes once into the document head.
// Using a <style> tag rather than a CSS file to keep this component self-contained
// and consistent with the rest of the app's inline-style conventions.
// Exported: App.tsx's Library save toast reuses the same `hew-spin`
// keyframes for its own inline thumbnail-rendering spinner rather than
// re-declaring an identical `<style>` injector.
export function ensureSpinnerKeyframes(): void {
  if (document.getElementById('hew-spin-keyframes') !== null) return
  const style = document.createElement('style')
  style.id = 'hew-spin-keyframes'
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- CSS keyframes text, not copy
  style.textContent = '@keyframes hew-spin { to { transform: rotate(360deg); } }'
  document.head.appendChild(style)
}

export function ImportingOverlay({ fileName, opening = false, progress = null }: ImportingOverlayProps) {
  const { t } = useLingui()
  ensureSpinnerKeyframes()
  const fraction = progress ? openProgressFraction(progress) : null
  const percent = fraction === null ? null : Math.round(fraction * 100)

  return (
    <div style={OVERLAY_STYLE} aria-live="assertive" aria-label={opening ? t`Opening model` : t`Importing model`}>
      <div
        style={CARD_STYLE}
        role="status"
        aria-busy="true"
      >
        <div style={SPINNER_STYLE} aria-hidden="true" />
        <div style={MESSAGE_STYLE}>
          {opening ? (
            <Trans>
              Opening <span style={FILE_NAME_STYLE}>&ldquo;{fileName}&rdquo;</span>&hellip;
            </Trans>
          ) : (
            <Trans>
              Importing <span style={FILE_NAME_STYLE}>&ldquo;{fileName}&rdquo;</span>&hellip;
            </Trans>
          )}
        </div>
        {progress && percent !== null ? (
          <>
            <div
              style={BAR_STYLE}
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={percent}
              aria-label={t`Open progress`}
            >
              <div style={{ ...BAR_FILL_STYLE, width: `${percent}%` }} />
            </div>
            {/* The counts change several times a second: the bar's value
                carries them to assistive technology, so the line itself
                is kept out of the card's live announcements. */}
            <div style={PROGRESS_LINE_STYLE} data-testid="open-progress" aria-live="off">
              <span>{openProgressText(progress)}</span>
              <span>{percent}%</span>
            </div>
          </>
        ) : (
          <div style={HINT_STYLE}>
            <Trans>This may take a moment for large files.</Trans>
          </div>
        )}
      </div>
    </div>
  )
}

/** The same card for a long edit (a Mend) on a large object: painted
 *  before the blocking call so the pause reads as work, not a hang. */
export function BusyOverlay({ message }: { message: string }) {
  ensureSpinnerKeyframes()
  return (
    <div style={OVERLAY_STYLE} aria-live="assertive" aria-label={message}>
      <div style={CARD_STYLE} role="status" aria-busy="true">
        <div style={SPINNER_STYLE} aria-hidden="true" />
        <div style={MESSAGE_STYLE}>{message}</div>
        <div style={HINT_STYLE}><Trans>This can take a while on a large object.</Trans></div>
      </div>
    </div>
  )
}
