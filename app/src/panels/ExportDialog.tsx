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
 * (the true-curves design), so STL export can re-facet cylinder walls
 * at any chosen smoothness — the model's stored facets are the floor, not
 * the ceiling. "As modeled" exports the stored facets verbatim.
 *
 * Escape cancels, mirroring the StlExportDialog / RecoveryDialog family
 * this is styled after. The slicer formats (STL, 3MF) keep their own
 * solid-gating confirmation (StlExportDialog) as a follow-on step after
 * this dialog's Export is clicked — this dialog only decides the format
 * and options, never the actual bytes-on-disk write.
 */

import { useEffect, useCallback, useState } from 'react'
import { VIEW_LABEL, type PrintViewKind } from '../print/printJob'
import { scaleDisplay, scaleHint, scalePresetsFor, type PrintScale } from '../print/scale'
import { getLengthUnit } from '../settings/units'

export type ExportFormat = 'glb' | 'stl' | '3mf' | 'usdz' | 'svg'

/** SVG line-drawing options (docs/design/printing.md §7b). */
export interface SvgExportOptions {
  view: PrintViewKind
  scale: PrintScale
  hiddenDashed: boolean
  includeDimensions: boolean
}

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
  onExport: (format: ExportFormat, stlSegmentsPerTurn: number, svg?: SvgExportOptions) => void
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
  // SVG line-drawing options (docs/design/printing.md §7b): a view, a
  // drawing scale from the unit family's ladder (1:1 default — laser/CNC
  // software reads the file at true size), dashed hidden lines, dimensions.
  const unitFormat = getLengthUnit()
  const svgPresets = scalePresetsFor(unitFormat)
  const [svgView, setSvgView] = useState<PrintViewKind>('current')
  const [svgScaleIndex, setSvgScaleIndex] = useState<number>(() => Math.max(0, svgPresets.findIndex((p) => Math.abs(p.paperMeters / p.modelMeters - 1) < 1e-9)))
  const [svgHidden, setSvgHidden] = useState(false)
  const [svgDimensions, setSvgDimensions] = useState(true)
  const svgOptions = (): SvgExportOptions => ({ view: svgView, scale: svgPresets[svgScaleIndex] as PrintScale, hiddenDashed: svgHidden, includeDimensions: svgDimensions })

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
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
          <option value="3mf">3MF (.3mf) — Z-up, millimeters, keeps part names and colors</option>
          <option value="usdz">USDZ (.usdz) — Y-up, meters, for AR Quick Look and USD pipelines</option>
          <option value="svg">SVG line drawing (.svg) — hidden lines removed, true size at a drawing scale</option>
        </select>

        {format === 'svg' && (
          <>
            <label style={LABEL_STYLE} htmlFor="export-svg-view-select">View</label>
            <select id="export-svg-view-select" style={SELECT_STYLE} value={svgView} onChange={(e) => setSvgView(e.target.value as PrintViewKind)}>
              {(Object.keys(VIEW_LABEL) as PrintViewKind[]).map((v) => (
                <option key={v} value={v}>{VIEW_LABEL[v]}</option>
              ))}
            </select>
            <label style={LABEL_STYLE} htmlFor="export-svg-scale-select">Scale</label>
            <select id="export-svg-scale-select" style={SELECT_STYLE} value={svgScaleIndex} onChange={(e) => setSvgScaleIndex(Number(e.target.value))}>
              {svgPresets.map((p, i) => {
                const hint = scaleHint(p, unitFormat)
                return (
                  <option key={p.label} value={i}>{hint === null ? scaleDisplay(p) : `${p.label}  (${hint})`}</option>
                )
              })}
            </select>
            <label style={{ ...LABEL_STYLE, display: 'flex', alignItems: 'center', gap: '6px' }}>
              <input type="checkbox" checked={svgHidden} onChange={(e) => setSvgHidden(e.target.checked)} /> Hidden lines dashed
            </label>
            <label style={{ ...LABEL_STYLE, display: 'flex', alignItems: 'center', gap: '6px', marginBottom: '16px' }}>
              <input type="checkbox" checked={svgDimensions} onChange={(e) => setSvgDimensions(e.target.checked)} /> Dimensions &amp; text
            </label>
          </>
        )}

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
          <button style={EXPORT_BUTTON_STYLE} onClick={() => onExport(format, stlSegments, format === 'svg' ? svgOptions() : undefined)}>
            Export
          </button>
        </div>
      </div>
    </div>
  )
}
