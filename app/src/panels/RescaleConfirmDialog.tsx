/**
 * RescaleConfirmDialog — Tape Measure's "resize the model?" confirmation
 * (design tool-parity §3, SketchUp's tape-measure rescale flow): after
 * measuring a real distance between two known points, typing a different
 * length arms this dialog instead of dropping a guide immediately.
 * Confirming uniformly rescales the WHOLE model (`rescale_document`) about
 * the world origin so the measured distance becomes the typed one. Two ways
 * to arm this: typing mid-gesture, before the second click (a LIVE arm) — on
 * that path, cancelling reverts to the ordinary guide-point commit the typed
 * distance would otherwise have produced — or typing after the fact, once
 * the gesture has already completed and the widget shows the frozen reading
 * (a RECALLED arm, tape-measure-rework part 2) — on that path there is no
 * pending gesture to fall back to, so cancelling just restores the frozen
 * reading and leaves the recall available to re-arm.
 *
 * `scope` (group-session.md's "Tape Measure scoped rescale") swaps the copy
 * for an IN-CONTEXT resize when a group/component session is open: instead
 * of the whole model, only the named frame's contents resize (anchored at
 * the measurement's first point, not the origin) — and for a component
 * frame, every other instance of it resizes too, since they share one
 * definition. `scope` is null — the whole-model copy below, unchanged — at
 * the top level.
 *
 * Styling follows the StlUnitsDialog family — theme tokens with the same
 * dark fallbacks the rest of the token consumers carry. Escape cancels,
 * same as the STL units chooser.
 */

import { useCallback, useEffect } from 'react'
import { formatLength } from '../settings/units'

interface RescaleConfirmDialogProps {
  /** The real, currently-measured distance between the two picked points. */
  currentDistance: number
  /** The length the user just typed. */
  typedDistance: number
  /** `typedDistance / currentDistance` — shown so the effect is legible
   *  before committing to it. */
  factor: number
  /** The innermost open session frame's display label and whether it's a
   *  component (vs. group) frame, or `null` for the whole-model rescale
   *  (top level, or the design's pre-existing behavior). Decided once by
   *  the caller (App's `handleRescaleArmed`) so this dialog's copy and the
   *  eventual commit call can never disagree about which is happening. */
  scope: { label: string; isComponent: boolean } | null
  /** Apply the rescale. */
  onConfirm: () => void
  /** Decline. For a LIVE arm (typed mid-gesture), falls back to the normal
   *  guide-point commit the typed distance would have produced; for a
   *  RECALLED arm (typed after the gesture already completed), there is no
   *  such commit to fall back to — it just restores the frozen reading.
   *  Also triggered by Escape. */
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
  marginBottom: '8px',
  lineHeight: '1.5',
}

const FACTOR_STYLE: React.CSSProperties = {
  fontSize: 'var(--font-size-body, 13px)',
  color: 'var(--text-tertiary, #999)',
  marginBottom: '16px',
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

const CONFIRM_BUTTON_STYLE: React.CSSProperties = {
  padding: '6px 20px',
  background: 'var(--accent-base, #3a5e9e)',
  color: 'var(--accent-text-strong, #fff)',
  border: 'none',
  borderRadius: 'var(--radius-control, 4px)',
  fontSize: 'var(--font-size-menu-item, 13px)',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  cursor: 'pointer',
}

export function RescaleConfirmDialog({
  currentDistance,
  typedDistance,
  factor,
  scope,
  onConfirm,
  onCancel,
}: RescaleConfirmDialogProps) {
  const heading = scope === null ? 'Resize the model?' : `Resize ${scope.label}?`
  const ariaLabel = scope === null ? 'Resize the model' : `Resize ${scope.label}`
  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        // Consume the event here so it never reaches the Viewport's
        // window-level keydown listener too. Without this, the SAME Escape
        // both cancels here (stage → idle, preserving TapeMeasureTool's
        // idlePlaneLock per its "aborting a gesture keeps the plane lock"
        // invariant) and then bubbles to window, where onKeyDown routes it
        // to the now-idle tool's onKey — which treats an idle Escape as
        // "clear the plane lock" and wipes it out from under the cancel
        // that just ran. One Escape must resolve through exactly one path.
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
        aria-label={ariaLabel}
      >
        <div style={HEADING_STYLE}>{heading}</div>
        <div style={BODY_STYLE}>
          {scope === null ? (
            <>
              The measured distance is {formatLength(currentDistance)}. Resize the whole model so it
              becomes {formatLength(typedDistance)}?
            </>
          ) : scope.isComponent ? (
            <>
              The measured distance is {formatLength(currentDistance)}. Resize {scope.label} — every
              copy of it — so it becomes {formatLength(typedDistance)}? Geometry outside it stays put.
            </>
          ) : (
            <>
              The measured distance is {formatLength(currentDistance)}. Resize {scope.label} so it
              becomes {formatLength(typedDistance)}? Geometry outside it stays put.
            </>
          )}
        </div>
        <div style={FACTOR_STYLE}>Scale factor: {factor.toFixed(4)}</div>
        <div style={BUTTON_ROW_STYLE}>
          <button style={CANCEL_BUTTON_STYLE} onClick={onCancel}>
            Cancel
          </button>
          <button style={CONFIRM_BUTTON_STYLE} onClick={onConfirm} autoFocus>
            Resize
          </button>
        </div>
      </div>
    </div>
  )
}
