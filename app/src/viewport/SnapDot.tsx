/**
 * SnapDot — the precise on-cursor inference marker (`07_inference_feedback.md`,
 * Refinement pass issue B). A filled ~13px dot centered exactly on the snap
 * point, colored by inference type, with a soft halo ring and a gentle pulse.
 *
 * This is the fix for "other than Select, it's hard to see where the snap
 * point physically is": the old marker was a thin 1px three.js cross that got
 * lost against geometry. The dot reuses the same container-relative
 * `screenX/screenY` the `InferenceTooltip` chip already projects (from
 * `Viewport.tsx`'s `onInferenceChange`), so it sits on the exact snap point
 * while the tool-glyph cursor rides offset (its hotspot is the click point,
 * the glyph body sits up-and-right of it — see `tools/toolIcons.ts`).
 *
 * The pulse + halo are CSS (`.hew-snap-dot` in index.css) so
 * `prefers-reduced-motion` can drop the animation to a static dot.
 */
import { inferenceCssColor } from './inferenceColor'
import type { InferenceInfo } from './Viewport'

export function SnapDot({ info }: { info: InferenceInfo | null }) {
  if (info === null) return null
  const color = inferenceCssColor(info)
  return (
    <div
      aria-hidden="true"
      className="hew-snap-dot"
      style={{
        position: 'absolute',
        left: `${info.screenX}px`,
        top: `${info.screenY}px`,
        width: 10,
        height: 10,
        borderRadius: '50%',
        background: color,
        // White ring for contrast on any material/background; soft tinted halo.
        border: '1.25px solid rgba(255, 255, 255, 0.92)',
        boxShadow: `0 0 0 3px color-mix(in srgb, ${color} 22%, transparent)`,
        pointerEvents: 'none',
        // Below the tooltip chip (z 20) — they never overlap (the chip is
        // offset +16,+16), but keep the label on top if they ever do.
        zIndex: 19,
        // Centering is expressed in the keyframes (so the animated transform
        // doesn't fight it); this inline value covers the reduced-motion case
        // where the animation is disabled.
        transform: 'translate(-50%, -50%)',
      }}
    />
  )
}
