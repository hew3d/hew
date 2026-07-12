/**
 * ExportDialog — unified File ▸ Export… dialog.
 *
 * Before this, "Export…" (glTF) and "Export STL…" were two separate menu
 * entries —  "I'd very much prefer that handled in the dialog itself
 * like literally every other app." Now there is exactly one menu entry
 * ("Export…") and the format choice moves into this dialog's Format select.
 *
 * STL carries one per-format option: **curve resolution**. Drawn circles
 * and arcs keep their exact analytic definitions on the solid
 * (docs/design/true-curves.md), so STL export can re-facet cylinder walls
 * at any chosen smoothness — the model's stored facets are the floor, not
 * the ceiling. "As modeled" exports the stored facets verbatim.
 *
 * Escape cancels, mirroring the StlExportDialog / RecoveryDialog family
 * this is styled after. STL keeps its own solid-gating confirmation
 * (StlExportDialog) as a follow-on step after this dialog's Export is
 * clicked — this dialog only decides the format and options, never the
 * actual bytes-on-disk write.
 */

import { useEffect, useCallback, useState } from 'react'

export type ExportFormat = 'glb' | 'stl'

/** STL curve-resolution choices: segments per full turn (0 = stored facets). */
const STL_RESOLUTIONS: { value: number; label: string }[] = [
  { value: 0, label: 'As modeled (stored facets)' },
  { value: 24, label: 'Draft (24 segments per turn)' },
  { value: 48, label: 'Standard (48 segments per turn)' },
  { value: 96, label: 'Fine (96 segments per turn)' },
  { value: 192, label: 'Ultra (192 segments per turn)' },
]

interface ExportDialogProps {
  /**
   * Proceed with the export in the currently selected format.
   * `stlSegmentsPerTurn` is the STL curve resolution (segments per full
   * turn, 0 = stored facets); meaningless for glTF.
   */
  onExport: (format: ExportFormat, stlSegmentsPerTurn: number) => void
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
  const [stlSegments, setStlSegments] = useState(48)

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

        {format === 'stl' && (
          <>
            <label style={LABEL_STYLE} htmlFor="export-stl-resolution-select">
              Curve resolution
            </label>
            <select
              id="export-stl-resolution-select"
              style={SELECT_STYLE}
              value={stlSegments}
              onChange={(e) => setStlSegments(Number(e.target.value))}
            >
              {STL_RESOLUTIONS.map((r) => (
                <option key={r.value} value={r.value}>
                  {r.label}
                </option>
              ))}
            </select>
          </>
        )}

        <div style={BUTTON_ROW_STYLE}>
          <button style={CANCEL_BUTTON_STYLE} onClick={onCancel}>
            Cancel
          </button>
          <button style={EXPORT_BUTTON_STYLE} onClick={() => onExport(format, stlSegments)}>
            Export
          </button>
        </div>
      </div>
    </div>
  )
}
