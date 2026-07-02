/**
 * InferenceTooltip — the cursor-anchored snap-kind chip (
 * `07_inference_feedback.md`). Net new: nothing like this existed before
 * this milestone (`Viewport.tsx` rendered no DOM overlay at all).
 *
 * Positioned via `InferenceInfo`'s container-relative `screenX`/`screenY`
 * (from `Viewport.tsx`'s `onInferenceChange`, fired at the same point the
 * existing `onStatusChange` status-bar text is derived).
 */
import { axisColorForDirection } from './axisColors'
import type { InferenceInfo } from './Viewport'

const KIND_LABEL: Record<string, string> = {
  endpoint: 'Endpoint',
  midpoint: 'Midpoint',
  intersection: 'Intersection',
  'on-edge': 'On Edge',
  'on-face': 'On Face',
  'on-guide': 'On Guide',
  'on-axis': 'On Axis',
  ground: 'Ground',
}

/** CSS hex per snap kind — mirrors `CueLayer.ts`'s three.js `SNAP_COLORS`
 * (kept as a separate, hand-authored map since one is a DOM/CSS concern and
 * the other a three.js material concern; values are the same colors). */
const KIND_CSS_COLOR: Record<string, string> = {
  endpoint: '#00cc44',
  midpoint: '#00cccc',
  intersection: '#ffaa00',
  'on-edge': '#cc2200',
  'on-face': '#0055cc',
  'on-guide': '#9933cc',
  ground: '#888888',
}

/** Generous tolerance — this is a label/color decision, not a snap decision
 * (the kernel/inference engine already decided the snap; we're just naming
 * the axis for display when the direction happens to be axis-aligned). */
const AXIS_LABEL_TOL_DOT = Math.cos((10 * Math.PI) / 180)

const AXIS_NAME = ['red', 'green', 'blue'] as const

export function InferenceTooltip({ info }: { info: InferenceInfo | null }) {
  if (info === null) return null

  const label = KIND_LABEL[info.kind] ?? info.kind
  const axisMatch = info.direction !== undefined
    ? axisColorForDirection(info.direction, AXIS_LABEL_TOL_DOT)
    : null
  const axisName = axisMatch !== null ? AXIS_NAME[axisMatch.axis] : null
  const chipColor = info.kind === 'on-axis' && axisMatch !== null
    ? `var(--axis-${axisName})`
    : (KIND_CSS_COLOR[info.kind] ?? 'var(--text-faint)')

  return (
    <div
      style={{
        position: 'absolute',
        left: `${info.screenX + 16}px`,
        top: `${info.screenY + 16}px`,
        pointerEvents: 'none',
        display: 'flex',
        alignItems: 'center',
        gap: '5px',
        padding: '4px 8px',
        background: 'var(--surface-overlay)',
        border: `1px solid ${chipColor}`,
        borderRadius: '7px',
        fontFamily: 'var(--font-family-ui)',
        fontSize: '12px',
        color: 'var(--text-primary)',
        whiteSpace: 'nowrap',
        zIndex: 20,
      }}
    >
      <span>{label}</span>
      {axisName !== null && info.kind !== 'on-axis' && (
        <span style={{ color: `var(--axis-${axisName})` }}>on {axisName} axis</span>
      )}
    </div>
  )
}
