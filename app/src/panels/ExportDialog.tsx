/**
 * ExportDialog — unified File ▸ Export… dialog.
 *
 * Before this, "Export…" (glTF) and "Export STL…" were two separate menu
 * entries —  "I'd very much prefer that handled in the dialog itself
 * like literally every other app." Now there is exactly one menu entry
 * ("Export…") and the format choice moves into this dialog's Format select.
 *
 * No per-format options exist yet (e.g. STL's mm/inch scale, glTF's
 * embed-textures toggle), so there's no Options button here — just the
 * format picker and Export/Cancel. Escape cancels, mirroring the
 * StlExportDialog / RecoveryDialog family this is styled after.
 *
 * STL keeps its own solid-gating confirmation (StlExportDialog) as a
 * follow-on step after this dialog's Export is clicked — this dialog only
 * decides the format, never the actual bytes-on-disk write.
 */

import { useEffect, useCallback, useState } from 'react'

export type ExportFormat = 'glb' | 'stl'

interface ExportDialogProps {
  /** Proceed with the export in the currently selected format. */
  onExport: (format: ExportFormat) => void
  /** Abort the dialog (also triggered by Escape). */
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
  minWidth: '380px',
  maxWidth: '480px',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  color: 'var(--text-secondary, #ddd)',
}

const HEADING_STYLE: React.CSSProperties = {
  fontSize: '15px',
  fontWeight: 600,
  color: 'var(--text-primary, #eee)',
  marginBottom: '16px',
}

const LABEL_STYLE: React.CSSProperties = {
  display: 'block',
  fontSize: 'var(--font-size-body, 12px)',
  color: 'var(--text-tertiary, #ccc)',
  marginBottom: '6px',
}

const SELECT_STYLE: React.CSSProperties = {
  width: '100%',
  padding: '7px 8px',
  background: 'var(--surface-input, #1c1c1c)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, #4a4a4a)',
  borderRadius: 'var(--radius-control, 4px)',
  fontSize: 'var(--font-size-menu-item, 13px)',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  marginBottom: '20px',
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

export function ExportDialog({ onExport, onCancel }: ExportDialogProps) {
  const [format, setFormat] = useState<ExportFormat>('glb')

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
        aria-label="Export"
      >
        <div style={HEADING_STYLE}>Export</div>

        <label style={LABEL_STYLE} htmlFor="export-format-select">
          Format
        </label>
        <select
          id="export-format-select"
          style={SELECT_STYLE}
          value={format}
          onChange={(e) => setFormat(e.target.value as ExportFormat)}
          autoFocus
        >
          <option value="glb">glTF binary (.glb) — Y-up, meters</option>
          <option value="stl">STL binary (.stl) — Z-up, millimeters, for 3D printing</option>
        </select>

        <div style={BUTTON_ROW_STYLE}>
          <button style={CANCEL_BUTTON_STYLE} onClick={onCancel}>
            Cancel
          </button>
          <button style={EXPORT_BUTTON_STYLE} onClick={() => onExport(format)}>
            Export
          </button>
        </div>
      </div>
    </div>
  )
}
