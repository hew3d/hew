/**
 * SkpLossNotice — the non-blocking notice shown when a SketchUp file could
 * not hold everything exactly (`simplified` and `writer_notes` from
 * `Scene.save_skp` / `export_skp`). A small dismissible card, not a modal: the save has
 * already happened.
 *
 * For a Save / Save As it offers "Save a .hew copy" (written through the Save
 * As picker WITHOUT rebinding the document, which stays bound to its
 * `.skp`); an Export has no such button.
 */

import { useEffect } from 'react'
import { Trans, useLingui } from '@lingui/react/macro'
import { isComposingKey } from '../input/keyboard'

export interface SkpLossNoticeData {
  /** What the file could not hold exactly, as short phrases. */
  simplified: string[]
  /** What the older SketchUp release's format could not hold at all (the
   * writer's own notes, one entry per kind), as short phrases; empty when
   * nothing was left out. */
  writerNotes: string[]
  /** Offer the "Save a .hew copy" button (Save / Save As; not Export). */
  offerHewCopy: boolean
}

interface SkpLossNoticeProps {
  notice: SkpLossNoticeData
  onSaveHewCopy: () => void
  onDismiss: () => void
}

const CARD_STYLE: React.CSSProperties = {
  position: 'fixed',
  left: '50%',
  bottom: '56px',
  transform: 'translateX(-50%)',
  zIndex: 1900,
  width: 'min(440px, calc(100vw - 32px))',
  background: 'var(--surface-overlay, #2a2a2a)',
  border: '1px solid var(--border-strong, #4a4a4a)',
  borderRadius: 'var(--radius-control, 6px)',
  boxShadow: 'var(--shadow-palette, 0 8px 32px rgba(0,0,0,0.6))',
  padding: '14px 16px',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  color: 'var(--text-secondary, #ddd)',
}

const HEADING_STYLE: React.CSSProperties = {
  fontSize: '13px',
  fontWeight: 600,
  color: 'var(--text-primary, #eee)',
  marginBottom: '6px',
}

const BODY_STYLE: React.CSSProperties = {
  fontSize: 'var(--font-size-body, 12px)',
  color: 'var(--text-tertiary, #ccc)',
  lineHeight: 1.5,
}

const LIST_STYLE: React.CSSProperties = {
  margin: '6px 0 0',
  paddingLeft: '18px',
  fontSize: 'var(--font-size-body, 12px)',
  maxHeight: '140px',
  overflowY: 'auto',
}

const BUTTON_ROW_STYLE: React.CSSProperties = {
  display: 'flex',
  justifyContent: 'flex-end',
  gap: '8px',
  marginTop: '12px',
}

const BUTTON_STYLE: React.CSSProperties = {
  padding: '5px 14px',
  background: 'var(--surface-input, #444)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, transparent)',
  borderRadius: 'var(--radius-control, 4px)',
  fontSize: 'var(--font-size-menu-item, 13px)',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  cursor: 'pointer',
}

export function SkpLossNotice({ notice, onSaveHewCopy, onDismiss }: SkpLossNoticeProps) {
  const { t } = useLingui()
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (isComposingKey(e)) return
      if (e.key === 'Escape') onDismiss()
    }
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [onDismiss])

  return (
    <div style={CARD_STYLE} role="status" aria-label={t`SketchUp file notice`} data-testid="skp-loss-notice">
      <div style={HEADING_STYLE}>
        <Trans>The SketchUp file can&rsquo;t hold everything exactly</Trans>
      </div>
      {notice.simplified.length > 0 && (
        <>
          <div style={BODY_STYLE}>
            <Trans>Some parts of this model were simplified in the SketchUp file:</Trans>
          </div>
          <ul style={LIST_STYLE}>
            {notice.simplified.map((item, i) => (
              <li key={i}>{item}</li>
            ))}
          </ul>
        </>
      )}
      {notice.writerNotes.length > 0 && (
        <>
          <div style={BODY_STYLE}>
            <Trans>This SketchUp format left out:</Trans>
          </div>
          <ul style={LIST_STYLE} data-testid="skp-loss-writer-notes">
            {notice.writerNotes.map((item, i) => (
              <li key={i}>{item}</li>
            ))}
          </ul>
        </>
      )}
      <div style={BUTTON_ROW_STYLE}>
        {notice.offerHewCopy && (
          <button style={BUTTON_STYLE} onClick={onSaveHewCopy} data-testid="skp-loss-hew-copy">
            <Trans>Save a .hew copy</Trans>
          </button>
        )}
        <button style={BUTTON_STYLE} onClick={onDismiss} data-testid="skp-loss-dismiss">
          <Trans>Dismiss</Trans>
        </button>
      </div>
    </div>
  )
}
