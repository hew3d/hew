/**
 * ImportReportDialog — modal shown after a successful COLLADA import.
 *
 * Displays the kernel ImportReport fields:
 *   - objects created (watertight vs leaky breakdown)
 *   - skipped meshes (name + reason), if any
 *   - missing texture URIs, if any
 *
 * Dismissed by clicking OK or pressing Escape.
 */

import { useEffect, useCallback } from 'react'
import type { ImportReport } from '../io/fileHost'

interface ImportReportDialogProps {
  report: ImportReport
  onClose: () => void
}

const OVERLAY_STYLE: React.CSSProperties = {
  position: 'fixed',
  inset: 0,
  background: 'rgba(0,0,0,0.6)',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  zIndex: 2000,
}

const DIALOG_STYLE: React.CSSProperties = {
  background: '#2a2a2a',
  border: '1px solid #4a4a4a',
  borderRadius: '6px',
  boxShadow: '0 8px 32px rgba(0,0,0,0.6)',
  padding: '20px 24px',
  minWidth: '340px',
  maxWidth: '520px',
  fontFamily: 'system-ui, sans-serif',
  color: '#ddd',
}

const HEADING_STYLE: React.CSSProperties = {
  fontSize: '15px',
  fontWeight: 600,
  color: '#eee',
  marginBottom: '16px',
}

const SECTION_STYLE: React.CSSProperties = {
  marginBottom: '12px',
}

const SECTION_LABEL_STYLE: React.CSSProperties = {
  fontSize: '11px',
  textTransform: 'uppercase',
  letterSpacing: '0.06em',
  color: '#888',
  marginBottom: '6px',
}

const LIST_STYLE: React.CSSProperties = {
  margin: 0,
  padding: '0 0 0 16px',
  fontSize: '12px',
  color: '#bbb',
  lineHeight: '1.6',
  maxHeight: '120px',
  overflowY: 'auto',
}

const OK_BUTTON_STYLE: React.CSSProperties = {
  display: 'block',
  marginLeft: 'auto',
  marginTop: '16px',
  padding: '6px 20px',
  background: '#3a5e9e',
  color: '#fff',
  border: 'none',
  borderRadius: '4px',
  fontSize: '13px',
  fontFamily: 'system-ui, sans-serif',
  cursor: 'pointer',
}

export function ImportReportDialog({ report, onClose }: ImportReportDialogProps) {
  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        onClose()
      }
    },
    [onClose],
  )

  useEffect(() => {
    document.addEventListener('keydown', handleKeyDown)
    return () => document.removeEventListener('keydown', handleKeyDown)
  }, [handleKeyDown])

  const { objects_created, watertight, leaky, skipped, textures_missing } = report

  return (
    <div style={OVERLAY_STYLE} onClick={onClose}>
      <div
        style={DIALOG_STYLE}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label="Import report"
      >
        <div style={HEADING_STYLE}>Import Complete</div>

        {/* Summary row */}
        <div style={{ ...SECTION_STYLE, fontSize: '13px', color: '#eee' }}>
          {objects_created === 0 ? (
            <span style={{ color: '#aaa' }}>No objects were created.</span>
          ) : (
            <>
              <strong>{objects_created}</strong>
              {' '}
              {objects_created === 1 ? 'object' : 'objects'} imported
              {' — '}
              <span style={{ color: '#5a9e5a' }}>{watertight} solid</span>
              {leaky > 0 && (
                <>
                  {', '}
                  <span style={{ color: '#cc6644' }}>{leaky} leaky</span>
                </>
              )}
            </>
          )}
        </div>

        {/* Skipped meshes */}
        {skipped.length > 0 && (
          <div style={SECTION_STYLE}>
            <div style={SECTION_LABEL_STYLE}>Skipped meshes ({skipped.length})</div>
            <ul style={LIST_STYLE}>
              {skipped.map((item, i) => (
                <li key={i}>
                  <strong style={{ color: '#dda' }}>{item.name}</strong>
                  {' — '}
                  <span>{item.reason}</span>
                </li>
              ))}
            </ul>
          </div>
        )}

        {/* Missing textures */}
        {textures_missing.length > 0 && (
          <div style={SECTION_STYLE}>
            <div style={SECTION_LABEL_STYLE}>Missing textures ({textures_missing.length})</div>
            <ul style={LIST_STYLE}>
              {textures_missing.map((uri, i) => (
                <li key={i} style={{ fontFamily: 'monospace', fontSize: '11px' }}>{uri}</li>
              ))}
            </ul>
          </div>
        )}

        <button style={OK_BUTTON_STYLE} onClick={onClose} autoFocus>
          OK
        </button>
      </div>
    </div>
  )
}
