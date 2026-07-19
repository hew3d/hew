/**
 * InferenceTooltip — the cursor-anchored snap-kind chip (
 * `07_inference_feedback.md`). Net new: nothing like this existed before
 * this milestone (`Viewport.tsx` rendered no DOM overlay at all).
 *
 * Positioned via `InferenceInfo`'s container-relative `screenX`/`screenY`
 * (from `Viewport.tsx`'s `onInferenceChange`, fired at the same point the
 * existing `onStatusChange` status-bar text is derived).
 */
import { inferenceAxisName, inferenceCssColor } from './inferenceColor'
import type { InferenceInfo } from './Viewport'

const KIND_LABEL: Record<string, string> = {
  endpoint: 'Endpoint',
  center: 'Center',
  quadrant: 'Quadrant',
  tangent: 'Tangent',
  midpoint: 'Midpoint',
  intersection: 'Intersection',
  'on-edge': 'On Edge',
  'on-face': 'On Face',
  'on-guide': 'On Guide',
  'on-axis': 'On Axis',
  ground: 'Ground',
  plane: 'On Plane',
}

export function InferenceTooltip({ info }: { info: InferenceInfo | null }) {
  if (info === null) return null

  const label = KIND_LABEL[info.kind] ?? info.kind
  const axisName = inferenceAxisName(info)
  const chipColor = inferenceCssColor(info)

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
