/**
 * RecoveryDialog — modal shown at startup when an autosaved recovery
 * snapshot exists and nothing else was loaded yet (see shouldPromptRecovery
 * in ../io/recoveryStore.ts).
 *
 * Dismissed via "Recover" (loads the snapshot) or "Discard" (clears it).
 * Escape only dismisses the dialog WITHOUT clearing the snapshot — an
 * accidental keypress must never destroy recoverable work; the snapshot is
 * simply re-offered on the next launch.
 */

import { useEffect, useCallback } from 'react'
import type { RecoverySnapshot } from '../io/recoveryStore'
import { formatRecoveryTime } from '../io/recoveryStore'

interface RecoveryDialogProps {
  snapshot: RecoverySnapshot
  onRecover: () => void
  onDiscard: () => void
  /** Dismiss without clearing the snapshot (Escape) — re-offered next launch. */
  onDismiss: () => void
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
  maxWidth: '480px',
  fontFamily: 'system-ui, sans-serif',
  color: '#ddd',
}

const HEADING_STYLE: React.CSSProperties = {
  fontSize: '15px',
  fontWeight: 600,
  color: '#eee',
  marginBottom: '12px',
}

const BODY_STYLE: React.CSSProperties = {
  fontSize: '13px',
  color: '#ccc',
  marginBottom: '20px',
  lineHeight: '1.5',
}

const BUTTON_ROW_STYLE: React.CSSProperties = {
  display: 'flex',
  justifyContent: 'flex-end',
  gap: '10px',
}

const DISCARD_BUTTON_STYLE: React.CSSProperties = {
  padding: '6px 20px',
  background: '#444',
  color: '#eee',
  border: 'none',
  borderRadius: '4px',
  fontSize: '13px',
  fontFamily: 'system-ui, sans-serif',
  cursor: 'pointer',
}

const RECOVER_BUTTON_STYLE: React.CSSProperties = {
  padding: '6px 20px',
  background: '#3a5e9e',
  color: '#fff',
  border: 'none',
  borderRadius: '4px',
  fontSize: '13px',
  fontFamily: 'system-ui, sans-serif',
  cursor: 'pointer',
}

export function RecoveryDialog({ snapshot, onRecover, onDiscard, onDismiss }: RecoveryDialogProps) {
  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        onDismiss()
      }
    },
    [onDismiss],
  )

  useEffect(() => {
    document.addEventListener('keydown', handleKeyDown)
    return () => document.removeEventListener('keydown', handleKeyDown)
  }, [handleKeyDown])

  const { name } = snapshot.meta
  const when = formatRecoveryTime(snapshot.meta.savedAt, Date.now())

  return (
    <div style={OVERLAY_STYLE}>
      <div
        style={DIALOG_STYLE}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label="Recover unsaved document"
      >
        <div style={HEADING_STYLE}>Recover Unsaved Document?</div>
        <div style={BODY_STYLE}>
          Hew found an autosaved version of <strong style={{ color: '#eee' }}>{name}</strong>{' '}
          from {when} that wasn&apos;t saved before the app closed.
        </div>
        <div style={BUTTON_ROW_STYLE}>
          <button style={DISCARD_BUTTON_STYLE} onClick={onDiscard}>
            Discard
          </button>
          <button style={RECOVER_BUTTON_STYLE} onClick={onRecover} autoFocus>
            Recover
          </button>
        </div>
      </div>
    </div>
  )
}
