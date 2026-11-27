/**
 * loftMath — the pure parts of the Loft tool: the spec JSON the kernel takes,
 * reading `loft_start_points`, and choosing the start vertex a grip drag
 * lands on. No three.js, no wasm.
 */

import { dotV3, subV3, type V3 } from '../viewport/geoHelpers'

export type LoftSource =
  | { kind: 'region'; sketch: bigint; region: bigint }
  | { kind: 'face'; object: bigint; face: bigint }
  | { kind: 'vertex'; point: V3 }

export interface LoftRail {
  sketch: bigint
  edges: bigint[]
}

/** The JSON `loft`, `loft_preview` and `loft_start_points` take. */
export function loftSpecJson(
  sources: readonly LoftSource[],
  rails: readonly LoftRail[],
  startOffsets: readonly number[],
  stations: number,
): string {
  const spec: Record<string, unknown> = {
    sources: sources.map((s) => {
      if (s.kind === 'region') return { region: { sketch: s.sketch.toString(), region: s.region.toString() } }
      if (s.kind === 'face') return { face: { object: s.object.toString(), face: s.face.toString() } }
      return { vertex: s.point }
    }),
    rails: rails.map((r) => ({ edges: { sketch: r.sketch.toString(), edges: r.edges.map((e) => e.toString()) } })),
    start_offsets: [...startOffsets],
  }
  if (stations > 0) spec.stations = stations
  return JSON.stringify(spec)
}

/** `loft_start_points`' flat answer, `[n, x, y, z, …]` per profile, as one
 *  list of vertices per profile. A short or malformed buffer yields what
 *  could be read. */
export function parseStartPoints(flat: ArrayLike<number>): V3[][] {
  const out: V3[][] = []
  let i = 0
  while (i < flat.length) {
    const n = Math.round(flat[i])
    i += 1
    const pts: V3[] = []
    for (let k = 0; k < n && i + 2 < flat.length; k++, i += 3) pts.push([flat[i], flat[i + 1], flat[i + 2]])
    out.push(pts)
  }
  return out
}

/** Index of the point nearest `p` (measured in the plane when `normal` is
 *  given), or -1 for an empty list. */
export function nearestVertexIndex(points: readonly V3[], p: V3, normal?: V3): number {
  let best = -1
  let bestD = Infinity
  for (let i = 0; i < points.length; i++) {
    let d = subV3(points[i], p)
    if (normal !== undefined) {
      const k = dotV3(d, normal)
      d = [d[0] - normal[0] * k, d[1] - normal[1] * k, d[2] - normal[2] * k]
    }
    const dd = d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
    if (dd < bestD) {
      bestD = dd
      best = i
    }
  }
  return best
}
