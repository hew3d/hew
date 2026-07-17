/**
 * offsetMath — pure in-plane helpers for the Offset tool.
 *
 * The kernel owns the offset geometry itself (`sketch_offset_region` /
 * `offset_face` and their previews); these helpers only turn the cursor
 * position into the SIGNED drag distance the kernel calls take (negative =
 * toward the material, i.e. the cursor is inside the boundary) and decode
 * the preview's multi-loop wire format.
 */

import { facePlaneBasis, type V3 } from '../viewport/geoHelpers'

/**
 * Signed offset distance for a cursor `point` lying (approximately) on the
 * plane of a closed `boundary` loop (flat `[x,y,z, …]`, implicit closure):
 * the distance from the point to the nearest boundary edge, negative when
 * the point is inside the loop (an inward offset), positive outside.
 * `null` when the boundary is degenerate (fewer than 3 vertices, or no
 * usable plane basis).
 */
export function signedOffsetDistance(
  point: V3,
  boundary: ArrayLike<number>,
  normal: V3,
): number | null {
  const n = Math.floor(boundary.length / 3)
  if (n < 3) return null
  const basis = facePlaneBasis(normal)
  if (basis === null) return null
  const { u, v } = basis

  const px = point[0] * u[0] + point[1] * u[1] + point[2] * u[2]
  const py = point[0] * v[0] + point[1] * v[1] + point[2] * v[2]
  const xs = new Float64Array(n)
  const ys = new Float64Array(n)
  for (let i = 0; i < n; i++) {
    const bx = boundary[i * 3]
    const by = boundary[i * 3 + 1]
    const bz = boundary[i * 3 + 2]
    xs[i] = bx * u[0] + by * u[1] + bz * u[2]
    ys[i] = bx * v[0] + by * v[1] + bz * v[2]
  }

  // Distance to the nearest boundary segment.
  let best = Infinity
  for (let i = 0; i < n; i++) {
    const j = (i + 1) % n
    const ax = xs[i]
    const ay = ys[i]
    const dx = xs[j] - ax
    const dy = ys[j] - ay
    const len2 = dx * dx + dy * dy
    const t = len2 > 0 ? Math.min(1, Math.max(0, ((px - ax) * dx + (py - ay) * dy) / len2)) : 0
    const cx = ax + dx * t
    const cy = ay + dy * t
    const d = Math.hypot(px - cx, py - cy)
    if (d < best) best = d
  }

  // Even-odd point-in-polygon for the sign.
  let inside = false
  for (let i = 0, j = n - 1; i < n; j = i++) {
    if (
      (ys[i] > py) !== (ys[j] > py) &&
      px < ((xs[j] - xs[i]) * (py - ys[i])) / (ys[j] - ys[i]) + xs[i]
    ) {
      inside = !inside
    }
  }

  return inside ? -best : best
}

/**
 * Decode `sketch_offset_region_preview`'s wire format —
 * `[loopCount, n₀, x,y,z × n₀, n₁, …]` — into one flat `[x,y,z, …]` array
 * per loop. Malformed input yields the loops that decode cleanly.
 */
export function decodeOffsetLoops(data: ArrayLike<number>): number[][] {
  const loops: number[][] = []
  if (data.length < 1) return loops
  const count = data[0]
  let at = 1
  for (let k = 0; k < count; k++) {
    if (at >= data.length) break
    const n = data[at]
    at += 1
    const end = at + n * 3
    if (end > data.length) break
    const loop: number[] = new Array(n * 3)
    for (let i = 0; i < n * 3; i++) loop[i] = data[at + i]
    loops.push(loop)
    at = end
  }
  return loops
}

/**
 * Turn one closed loop (flat `[x,y,z, …]`) into flat SEGMENT-PAIR positions
 * (`[ax,ay,az, bx,by,bz, …]`, closing edge included) — the layout
 * `makeFatSegments` consumes.
 */
export function loopToSegmentPairs(loop: ArrayLike<number>): Float32Array {
  const n = Math.floor(loop.length / 3)
  const out = new Float32Array(n * 6)
  for (let i = 0; i < n; i++) {
    const j = (i + 1) % n
    out[i * 6] = loop[i * 3]
    out[i * 6 + 1] = loop[i * 3 + 1]
    out[i * 6 + 2] = loop[i * 3 + 2]
    out[i * 6 + 3] = loop[j * 3]
    out[i * 6 + 4] = loop[j * 3 + 1]
    out[i * 6 + 5] = loop[j * 3 + 2]
  }
  return out
}

/**
 * Vertex-match tolerance for `boundaryContainsEdge`. Region boundaries
 * cross the wasm boundary as f32 while edge endpoints stay f64, so
 * identical kernel vertices can differ by the f32 round-trip error of
 * their magnitude — which is RELATIVE (≤ 0.5 ulp ≈ 6e-8 · |coord| per
 * component), not a fixed distance: at 500 m from the origin it already
 * exceeds 1e-5 m. The per-comparison tolerance is therefore the larger of
 * a small absolute floor (near-origin coordinates, where the relative
 * term vanishes) and a magnitude-proportional term with headroom over the
 * worst per-component ulp. Both stay orders of magnitude below the
 * kernel's vertex-merge spacing at any magnitude a sketch can plausibly
 * reach, so distinct vertices never alias. UI-side pick plumbing only —
 * kernel comparisons keep using `kernel::tol` (DEVELOPMENT.md rule 6).
 */
const BOUNDARY_VERTEX_TOL_ABS = 1e-5
/** f32 unit roundoff (2^-24): |fround(x) − x| ≤ F32_UNIT_ROUNDOFF · |x|. */
const F32_UNIT_ROUNDOFF = 2 ** -24
/** Headroom over the per-component 0.5-ulp bound (three components sum in
 * the squared distance, plus slack for upstream arithmetic on the way to
 * the boundary buffer). */
const BOUNDARY_VERTEX_TOL_REL = 8 * F32_UNIT_ROUNDOFF

/**
 * True when the segment `endpoints` (`[ax,ay,az, bx,by,bz]`, f64) appears as
 * a pair of CONSECUTIVE vertices — in either direction, including the
 * implicit closing segment — of the `boundary` loop (flat `[x,y,z, …]`, f32,
 * implicit closure). This is how a picked sketch edge is resolved to the
 * region whose outer boundary carries it (the Offset tool's edge-click
 * fallback: the region pick itself is interior-only).
 */
export function boundaryContainsEdge(
  boundary: ArrayLike<number>,
  endpoints: ArrayLike<number>,
): boolean {
  const n = Math.floor(boundary.length / 3)
  if (n < 2 || endpoints.length < 6) return false

  const near = (i: number, px: number, py: number, pz: number): boolean => {
    const dx = boundary[i * 3] - px
    const dy = boundary[i * 3 + 1] - py
    const dz = boundary[i * 3 + 2] - pz
    // Magnitude-aware tolerance: the f32 round-trip error grows with the
    // largest coordinate involved in THIS comparison (see the constants'
    // doc comment), so scale the relative term by it.
    const maxAbs = Math.max(
      Math.abs(boundary[i * 3]), Math.abs(boundary[i * 3 + 1]), Math.abs(boundary[i * 3 + 2]),
      Math.abs(px), Math.abs(py), Math.abs(pz),
    )
    const tol = Math.max(BOUNDARY_VERTEX_TOL_ABS, BOUNDARY_VERTEX_TOL_REL * maxAbs)
    return dx * dx + dy * dy + dz * dz <= tol * tol
  }

  const [ax, ay, az, bx, by, bz] = [
    endpoints[0], endpoints[1], endpoints[2],
    endpoints[3], endpoints[4], endpoints[5],
  ]
  for (let i = 0; i < n; i++) {
    const j = (i + 1) % n // implicit closure: last vertex pairs with first
    if (
      (near(i, ax, ay, az) && near(j, bx, by, bz)) ||
      (near(i, bx, by, bz) && near(j, ax, ay, az))
    ) {
      return true
    }
  }
  return false
}
