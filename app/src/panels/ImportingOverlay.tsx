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

interface ImportingOverlayProps {
  /** Display name of the file being imported. */
  fileName: string
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

// Inject the keyframes once into the document head.
// Using a <style> tag rather than a CSS file to keep this component self-contained
// and consistent with the rest of the app's inline-style conventions.
function ensureSpinnerKeyframes(): void {
  if (document.getElementById('hew-spin-keyframes') !== null) return
  const style = document.createElement('style')
  style.id = 'hew-spin-keyframes'
  style.textContent = '@keyframes hew-spin { to { transform: rotate(360deg); } }'
  document.head.appendChild(style)
}

export function ImportingOverlay({ fileName }: ImportingOverlayProps) {
  ensureSpinnerKeyframes()

  return (
    <div style={OVERLAY_STYLE} aria-live="assertive" aria-label="Importing model">
      <div
        style={CARD_STYLE}
        role="status"
        aria-busy="true"
      >
        <div style={SPINNER_STYLE} aria-hidden="true" />
        <div style={MESSAGE_STYLE}>
          Importing{' '}
          <span style={FILE_NAME_STYLE}>&ldquo;{fileName}&rdquo;</span>
          &hellip;
        </div>
        <div style={HINT_STYLE}>This may take a moment for large files.</div>
      </div>
    </div>
  )
}
