/**
 * StlExportDialog — pre-export confirmation shown when the model contains
 * objects that are not watertight solids ( solid gating).
 *
 * STL is consumed by slicers, which expect manifold geometry; Hew never
 * repairs geometry silently (rule 4), so the user chooses: Export Anyway or
 * Cancel. Escape cancels. Not shown at all when every object is solid.
 *
 * Styling follows the RecoveryDialog / ImportReportDialog family, but on the
 *  Studio theme tokens (with the same dark fallbacks the rest of the
 * token consumers carry) rather than hardcoded colors.
 */

import { useEffect, useCallback } from 'react'

interface StlExportDialogProps {
  /** Display names of the non-solid objects that would be exported. */
  offenders: string[]
  /** Proceed with the export despite the non-solid objects. */
  onExport: () => void
  /** Abort the export (also triggered by Escape). */
  onCancel: () => void
}

const OVERLAY_STYLE: React.CSSProperties = {
  position: 'fixed',
  inset: 0,
  background: 'var(--backdrop-dim, rgba(0,0,0,0.6))',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  zIndex: 2000,
}

const DIALOG_STYLE: React.CSSProperties = {
  background: 'var(--surface-overlay, #2a2a2a)',
  border: '1px solid var(--border-strong, #4a4a4a)',
  borderRadius: 'var(--radius-control, 6px)',
  boxShadow: 'var(--shadow-palette, 0 8px 32px rgba(0,0,0,0.6))',
  padding: '20px 24px',
  minWidth: '340px',
  maxWidth: '480px',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  color: 'var(--text-secondary, #ddd)',
}

const HEADING_STYLE: React.CSSProperties = {
  fontSize: '15px',
  fontWeight: 600,
  color: 'var(--text-primary, #eee)',
  marginBottom: '12px',
}

const BODY_STYLE: React.CSSProperties = {
  fontSize: 'var(--font-size-body, 13px)',
  color: 'var(--text-tertiary, #ccc)',
  marginBottom: '12px',
  lineHeight: '1.5',
}

const LIST_STYLE: React.CSSProperties = {
  margin: '0 0 20px 0',
  padding: '0 0 0 16px',
  fontSize: 'var(--font-size-body, 12px)',
  color: 'var(--text-secondary, #ccc)',
  fontFamily: 'var(--font-family-mono, monospace)',
  lineHeight: '1.6',
  maxHeight: '120px',
  overflowY: 'auto',
}

const BUTTON_ROW_STYLE: React.CSSProperties = {
  display: 'flex',
  justifyContent: 'flex-end',
  gap: '10px',
}

const CANCEL_BUTTON_STYLE: React.CSSProperties = {
  padding: '6px 20px',
  background: 'var(--surface-input, #444)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, transparent)',
  borderRadius: 'var(--radius-control, 4px)',
  fontSize: 'var(--font-size-menu-item, 13px)',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  cursor: 'pointer',
}

const EXPORT_BUTTON_STYLE: React.CSSProperties = {
  padding: '6px 20px',
  background: 'var(--accent-base, #3a5e9e)',
  color: 'var(--accent-text-strong, #fff)',
  border: 'none',
  borderRadius: 'var(--radius-control, 4px)',
  fontSize: 'var(--font-size-menu-item, 13px)',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  cursor: 'pointer',
}

export function StlExportDialog({ offenders, onExport, onCancel }: StlExportDialogProps) {
  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        onCancel()
      }
    },
    [onCancel],
  )

  useEffect(() => {
    document.addEventListener('keydown', handleKeyDown)
    return () => document.removeEventListener('keydown', handleKeyDown)
  }, [handleKeyDown])

  return (
    <div style={OVERLAY_STYLE} onClick={onCancel}>
      <div
        style={DIALOG_STYLE}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label="Export STL warning"
      >
        <div style={HEADING_STYLE}>Export STL Anyway?</div>
        <div style={BODY_STYLE}>
          These objects are not watertight solids; the STL may not be manifold:
        </div>
        <ul style={LIST_STYLE}>
          {offenders.map((name, i) => (
            <li key={i}>{name}</li>
          ))}
        </ul>
        <div style={BUTTON_ROW_STYLE}>
          <button style={CANCEL_BUTTON_STYLE} onClick={onCancel}>
            Cancel
          </button>
          <button style={EXPORT_BUTTON_STYLE} onClick={onExport} autoFocus>
            Export Anyway
          </button>
        </div>
      </div>
    </div>
  )
}
