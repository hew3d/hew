/* oxlint-disable lingui/no-unlocalized-strings -- SVG path data and Preview-only identifiers */
import { t } from '@lingui/core/macro'
import type { FilletProfile } from '../tools/filletSettings'
import type { ProfileShapeJson } from '../tools/filletMath'

/** SVG serialization only: retain the supplied line/arc geometry, including
 * major arcs. These display bounds never participate in model operations. */
export function profileOutline(shape: unknown): { path: string; transform: string } | null {
  if (shape === null || typeof shape !== 'object') return null
  const { b, segs } = shape as ProfileShapeJson
  if (!Number.isFinite(b) || b <= 0 || !Array.isArray(segs) || segs.length === 0) return null
  const point = (p: unknown): p is [number, number] => Array.isArray(p) && p.length === 2 && p.every(Number.isFinite)
  const tau = 2 * Math.PI
  const positive = (n: number): number => (n % tau + tau) % tau
  let previous: [number, number] = [1, 0]
  let path = 'M1 0'
  const xs = [0, 1.25], ys = [0, b + 0.25]
  for (const seg of segs) {
    if (!seg || !point(seg.to)) return null
    xs.push(seg.to[0]); ys.push(seg.to[1])
    if (seg.kind === 'line') path += `L${seg.to[0]} ${seg.to[1]}`
    else if (seg.kind === 'arc' && point(seg.center) && typeof seg.ccw === 'boolean') {
      const [cx, cy] = seg.center
      const radius = Math.hypot(previous[0] - cx, previous[1] - cy)
      if (!(radius > 0) || !Number.isFinite(radius)) return null
      const start = Math.atan2(previous[1] - cy, previous[0] - cx)
      const end = Math.atan2(seg.to[1] - cy, seg.to[0] - cx)
      const sweep = positive(seg.ccw ? end - start : start - end)
      if (sweep === 0) return null // No guessed full-circle interpretation.
      for (const angle of [0, Math.PI / 2, Math.PI, 3 * Math.PI / 2]) {
        if (positive(seg.ccw ? angle - start : start - angle) <= sweep) {
          xs.push(cx + radius * Math.cos(angle)); ys.push(cy + radius * Math.sin(angle))
        }
      }
      path += `A${radius} ${radius} 0 ${sweep > Math.PI ? 1 : 0} ${seg.ccw ? 1 : 0} ${seg.to[0]} ${seg.to[1]}`
    } else return null
    previous = seg.to
  }
  // The remaining workpiece gives every section the same orientation.
  path += `L0 ${b + 0.25}H1.25V0Z`
  const minX = Math.min(...xs), maxX = Math.max(...xs)
  const minY = Math.min(...ys), maxY = Math.max(...ys)
  const scale = 18 / Math.max(maxX - minX, maxY - minY)
  const tx = 12 + scale * (minX + maxX) / 2
  const ty = 12 - scale * (minY + maxY) / 2
  return { path, transform: `translate(${tx} ${ty}) scale(${-scale} ${scale})` }
}

export function ProfileIcon({ kind, shape }: { kind: FilletProfile['kind']; shape?: unknown }) {
  const custom = kind === 'shape' ? profileOutline(shape) : null
  if (kind === 'shape' && custom === null) return <span className="fillet-profile-icon-unavailable" title={t`Profile outline unavailable`} aria-hidden="true" />
  const cut = {
    round: 'M3 21V3H9A12 12 0 0 0 21 15V21Z',
    chamfer: 'M3 21V3H9L21 15V21Z',
    cove: 'M3 21V3H9A12 12 0 0 1 21 15V21Z',
    bullnose: 'M3 21V3H15A9 9 0 0 1 15 21Z',
    shape: '',
  }[kind]
  return <svg aria-hidden="true" data-profile-kind={kind} viewBox="0 0 24 24" width="24" height="24" style={{ flexShrink: 0 }}>
    <path d={custom?.path ?? cut} transform={custom?.transform} fill="none" stroke="currentColor" strokeWidth="1.5" vectorEffect="non-scaling-stroke" strokeLinejoin="round" strokeLinecap="round" />
  </svg>
}
