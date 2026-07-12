/**
 * inferenceColor — the single source of truth for the CSS color of an
 * inference/snap by kind. Shared by the `InferenceTooltip` chip border and the
 * `SnapDot` marker so the two overlays never disagree on a color (Refinement
 * pass, issue B). Mirrors `CueLayer.ts`'s three.js `SNAP_COLORS` values; kept
 * separate because one is a DOM/CSS concern and the other a three.js material
 * concern.
 */
import { axisColorForDirection } from './axisColors'
import type { InferenceInfo } from './Viewport'

/** CSS hex per snap kind. */
export const KIND_CSS_COLOR: Record<string, string> = {
  endpoint: '#00cc44',
  center: '#00aa88',
  quadrant: '#00aa88',
  tangent: '#b050d0',
  midpoint: '#00cccc',
  intersection: '#ffaa00',
  'on-edge': '#cc2200',
  'on-face': '#0055cc',
  'on-guide': '#9933cc',
  ground: '#888888',
}

/** Generous tolerance — a label/color decision, not a snap decision (the
 * kernel already decided the snap; we're just naming the axis for display when
 * the direction happens to be axis-aligned). */
const AXIS_LABEL_TOL_DOT = Math.cos((10 * Math.PI) / 180)
const AXIS_NAME = ['red', 'green', 'blue'] as const

/** The axis name ('red'|'green'|'blue') for an inference whose direction is
 * (near) axis-aligned, else null. */
export function inferenceAxisName(info: InferenceInfo): (typeof AXIS_NAME)[number] | null {
  if (info.direction === undefined) return null
  const match = axisColorForDirection(info.direction, AXIS_LABEL_TOL_DOT)
  return match !== null ? AXIS_NAME[match.axis] : null
}

/** The CSS color string for an inference — an `--axis-*` var when it's an
 * axis snap, otherwise the kind's hex (falling back to `--text-faint`). */
export function inferenceCssColor(info: InferenceInfo): string {
  const axisName = inferenceAxisName(info)
  if (info.kind === 'on-axis' && axisName !== null) return `var(--axis-${axisName})`
  return KIND_CSS_COLOR[info.kind] ?? 'var(--text-faint)'
}
