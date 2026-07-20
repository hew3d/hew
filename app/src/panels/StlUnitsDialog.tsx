/**
 * StlUnitsDialog — shown once per `.stl` import, before the blocking parse
 * begins, to ask which unit the file's numbers are in.
 *
 * STL carries no unit information at all (unlike COLLADA/glTF/SketchUp,
 * which all record real-world scale); the maker-community convention is
 * millimeters, but a slicer-bound file could be centimeters, inches, or
 * meters just as easily. Getting this wrong is the single most common STL
 * import frustration ("my model is 1000x too big/small"), so the prompt is
 * mandatory rather than a silently-assumed default — an explicit design
 * call (DESIGN §5). The last choice is remembered for the rest of the session
 * (`settings/stlImportUnit.ts`) so importing several STLs in a row doesn't
 * repeat the same click.
 *
 * Styling follows the StlExportDialog / RecoveryDialog / ImportReportDialog
 * family — theme tokens with the same dark fallbacks the rest of the token
 * consumers carry. Escape cancels (leaves the current document untouched,
 * same as cancelling the file picker).
 */

import { useCallback, useEffect, useState } from 'react'
import { STL_UNIT_OPTIONS, getLastStlImportUnit, type StlUnitOption } from '../settings/stlImportUnit'

interface StlUnitsDialogProps {
  /** Display name of the file being imported. */
  fileName: string
  /** Proceed with the given unit choice's `unitScale`. */
  onChoose: (unitScale: number, value: StlUnitOption['value']) => void
  /** Abort the import (also triggered by Escape). */
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
  marginBottom: '16px',
  lineHeight: '1.5',
}

const OPTIONS_STYLE: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: '4px',
  marginBottom: '20px',
}

const OPTION_LABEL_STYLE: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '8px',
  padding: '6px 8px',
  borderRadius: 'var(--radius-control, 4px)',
  fontSize: 'var(--font-size-body, 13px)',
  color: 'var(--text-primary, #eee)',
  cursor: 'pointer',
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

const IMPORT_BUTTON_STYLE: React.CSSProperties = {
  padding: '6px 20px',
  background: 'var(--accent-base, #3a5e9e)',
  color: 'var(--accent-text-strong, #fff)',
  border: 'none',
  borderRadius: 'var(--radius-control, 4px)',
  fontSize: 'var(--font-size-menu-item, 13px)',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  cursor: 'pointer',
}

export function StlUnitsDialog({ fileName, onChoose, onCancel }: StlUnitsDialogProps) {
  const [selected, setSelected] = useState<StlUnitOption['value']>(getLastStlImportUnit())

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

  const commit = () => {
    const option = STL_UNIT_OPTIONS.find((o) => o.value === selected) ?? STL_UNIT_OPTIONS[0]
    onChoose(option.unitScale, option.value)
  }

  return (
    <div style={OVERLAY_STYLE} onClick={onCancel}>
      <div
        style={DIALOG_STYLE}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label="STL import units"
      >
        <div style={HEADING_STYLE}>Import &ldquo;{fileName}&rdquo;</div>
        <div style={BODY_STYLE}>STL files don&rsquo;t record their units. Import as:</div>
        <div style={OPTIONS_STYLE} role="radiogroup" aria-label="Units">
          {STL_UNIT_OPTIONS.map((option) => (
            <label key={option.value} style={OPTION_LABEL_STYLE}>
              <input
                type="radio"
                name="stl-import-unit"
                value={option.value}
                checked={selected === option.value}
                onChange={() => setSelected(option.value)}
              />
              {option.label}
              {option.value === 'mm' ? ' (default)' : ''}
            </label>
          ))}
        </div>
        <div style={BUTTON_ROW_STYLE}>
          <button style={CANCEL_BUTTON_STYLE} onClick={onCancel}>
            Cancel
          </button>
          <button style={IMPORT_BUTTON_STYLE} onClick={commit} autoFocus>
            Import
          </button>
        </div>
      </div>
    </div>
  )
}
