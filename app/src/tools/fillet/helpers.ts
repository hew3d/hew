/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/** Pure helpers shared by the Fillet stages. */

import { formatLength } from '../../settings/units'
import type { V3 as GeoV3 } from '../../viewport/geoHelpers'
import { type CurveToken } from '../curveInput'
import { getFilletSettings } from '../filletSettings'
import { type V3 } from '../filletMath'
import { type Corner2d } from '../filletCorner2d'
import { type CornerTarget } from '../filletPick'
import type { Pick, Sized, Resolved } from './types'

/** A size's primary length, as the kernel's `BlendProfile::size` reads it
 *  (a chamfer's first setback); a bullnose has none. */
export function sizeOf(s: Sized): number | null {
  switch (s.kind) {
    case 'round':
    case 'cove':
      return s.radius
    case 'chamfer':
      return s.a
    case 'shape':
      return s.width
    case 'bullnose':
      return null
  }
}

/** `s` scaled by `f`, as `BlendProfile::scaled` scales (a chamfer's two
 *  setbacks by one factor). */
export function scaledSized(s: Sized, f: number): Sized {
  switch (s.kind) {
    case 'round':
    case 'cove':
      return { ...s, radius: s.radius * f }
    case 'chamfer':
      return { ...s, a: s.a * f, b: s.b * f }
    case 'shape':
      return { ...s, width: s.width * f }
    case 'bullnose':
      return s
  }
}

/** Drawn corners take only Round and Chamfer (`armCorner`). */
export function cornerProfileOk(): boolean {
  const kind = getFilletSettings().profile.kind
  return kind === 'round' || kind === 'chamfer'
}

export function isDigitStart(key: string): boolean {
  return (key >= '0' && key <= '9') || key === '.' || key === '-'
}

export function offsetBy(p: V3, dir: V3, d: number): V3 {
  return [p[0] + dir[0] * d, p[1] + dir[1] * d, p[2] + dir[2] * d]
}

export function cornerGeomOf(c: CornerTarget): Corner2d {
  return { c: c.point as GeoV3, p: c.p as GeoV3, q: c.q as GeoV3 }
}

/** The edge of a pick's chain nearest its picked point. */
export function nearestEdge(p: Pick): bigint | null {
  const seg = p.chain.segments
  let best: { d: number; i: number } | null = null
  for (let i = 0; i < p.chain.edges.length; i++) {
    const a: V3 = [seg[i * 6], seg[i * 6 + 1], seg[i * 6 + 2]]
    const b: V3 = [seg[i * 6 + 3], seg[i * 6 + 4], seg[i * 6 + 5]]
    const ab: V3 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]]
    const l2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2]
    const t = l2 > 0 ? Math.max(0, Math.min(1, ((p.point[0] - a[0]) * ab[0] + (p.point[1] - a[1]) * ab[1] + (p.point[2] - a[2]) * ab[2]) / l2)) : 0
    const d = Math.hypot(a[0] + ab[0] * t - p.point[0], a[1] + ab[1] * t - p.point[1], a[2] + ab[2] * t - p.point[2])
    if (best === null || d < best.d) best = { d, i }
  }
  return best === null ? (p.chain.edges[0] ?? null) : p.chain.edges[best.i]
}

/** The token a successful size repeats as (a double-click, the next retype). */
export function tokenOf(r: Resolved): CurveToken | null {
  const s = r.sized
  switch (s.kind) {
    case 'round':
    case 'cove':
      return { kind: 'length', meters: s.radius }
    case 'chamfer':
      return s.asymmetric ? { kind: 'pair', a: s.a, b: s.b } : { kind: 'length', meters: s.a }
    case 'shape':
      return { kind: 'length', meters: s.width }
    default:
      return null
  }
}

export function tokenText(t: CurveToken): string {
  switch (t.kind) {
    case 'length':
      return formatLength(t.meters)
    case 'pair':
      return `${formatLength(t.a)}, ${formatLength(t.b)}`
    default:
      return ''
  }
}
