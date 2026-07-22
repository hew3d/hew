/**
 * MeasurementBox — the docked VCB (value control box),
 * `07_inference_feedback.md`. SketchUp's type-a-number-to-set-dimension
 * field, docked top-right instead of an easily-missed corner field. Net new:
 * today's measurement text only ever reached the bottom status bar as plain
 * text (`onMeasurement`); this is the first real floating box for it.
 *
 * Reuses the *existing* `measurement` text `App.tsx` already receives via
 * `Viewport`'s `onMeasurement` callback — no new Viewport plumbing. The
 * label is derived from the active tool name (a small local map) since
 * `onMeasurement` only ever carried the formatted value, never a label.
 */
const VCB_LABEL: Record<string, string> = {
  'Move': 'Distance',
  'Push/Pull': 'Push depth',
  'Rotate': 'Angle',
  'Scale': 'Factor',
  'Tape Measure': 'Distance',
  'Protractor': 'Angle',
  'Follow Me': 'Swept length',
}

export interface MeasurementBoxProps {
  toolName: string
  value: string
}

export function MeasurementBox({ toolName, value }: MeasurementBoxProps) {
  if (value === '') return null
  const label = VCB_LABEL[toolName] ?? 'Value'

  return (
    <div
      style={{
        position: 'absolute',
        top: '16px',
        right: '16px',
        display: 'flex',
        alignItems: 'center',
        gap: '8px',
        padding: '6px 12px',
        background: 'var(--surface-overlay)',
        border: '1px solid var(--border-strong)',
        borderRadius: 'var(--radius-control, 7px)',
        boxShadow: 'var(--shadow-chip, none)',
        fontFamily: 'var(--font-family-ui)',
        zIndex: 20,
      }}
    >
      <span style={{ fontSize: '11px', color: 'var(--text-faint)', whiteSpace: 'nowrap' }}>{label}</span>
      <span
        style={{
          fontFamily: 'var(--font-family-mono)',
          fontSize: 'var(--font-size-measurement, 14px)',
          fontWeight: 600,
          color: 'var(--text-primary)',
          whiteSpace: 'nowrap',
        }}
      >
        {value}
        <span aria-hidden="true" className="hew-vcb-caret" style={{ color: 'var(--accent-base)' }}>|</span>
      </span>
    </div>
  )
}
