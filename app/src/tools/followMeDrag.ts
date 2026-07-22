/**
 * followMeDrag — maps a cursor ray to an arc-length position along a picked
 * Follow Me path, for the drag-to-partial-sweep gesture (design §10 /
 * `Object::from_follow_me_to`'s `stop_len`, arc length from the SEAM).
 *
 * WHY A SEPARATE MODULE. `followMeStart.ts` mirrors the kernel's start-legality
 * rule byte-for-byte, with the exact/decisive/unknown discipline that
 * demands. This module does NOT: it is a best-effort PREVIEW aid — where to
 * draw the live station marker and what length to show in the VCB while the
 * user drags — not a legality claim. The kernel remains the sole authority on
 * whether a given `stop_len` is accepted; a preview that guesses the seam's
 * position slightly wrong in a pathological case costs a slightly-off-looking
 * preview, never a wrong commit (the actual `stop_len` sent is always the
 * literal arc length this module computed, and the kernel measures its own
 * seam from its own anchor choice independently).
 *
 * SEAM APPROXIMATION. The kernel's real anchor choice (design §2/§2b) is the
 * candidate nearest the profile's centroid, filtered to only the candidates
 * that are actually legal starts — reproducing that exactly means
 * reproducing the whole candidate scan `followMeStart.ts` already does. For
 * an OPEN path the anchor is always exactly one of the two ends (never a
 * midpoint), so `openSeamWalk` picks between them with the same priority the
 * kernel uses (design §2a: on-plane-and-square wins outright, else the
 * nearer of the two). For a CLOSED path, `closedSeamWalk` uses the single
 * nearest point on the WHOLE loop to the profile centroid — the kernel's own
 * tie-break rule, just applied without the candidate-eligibility filter —
 * which coincides with the true anchor whenever the profile is actually
 * placed at (or near) its intended start, the situation dragging happens in.
 */

export type Vec3 = readonly [number, number, number]

export interface CurveGeom {
  center: Vec3
  radius: number
}

export interface PathSegment {
  a: Vec3
  b: Vec3
  curve: CurveGeom | null
}

export interface PathPolyline {
  segments: PathSegment[]
  closed: boolean
}

export interface PlaneDef {
  point: Vec3
  normal: Vec3
}

/**
 * The path re-walked starting at the seam, in the direction the sweep
 * actually leaves it — the same shape as the kernel's `pts` array. A closed
 * path's walk completes one full lap back to the seam point (`total` is the
 * whole loop's length); an open path's walk ends at the far end.
 */
export interface SeamWalk {
  /** Vertices from the seam (index 0) onward. `points[0]` and
   *  `points[points.length - 1]` are BOTH the seam point for a closed path
   *  (the lap closes); for an open path they are the two different ends. */
  points: Vec3[]
  /** Cumulative arc length at each `points` index — `cumulative[0] === 0`,
   *  `cumulative[cumulative.length - 1] === total`. */
  cumulative: number[]
  total: number
}

// ------------------------------------------------------------------ vec math

function sub(a: Vec3, b: Vec3): Vec3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
function add(a: Vec3, b: Vec3): Vec3 {
  return [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
function scale(a: Vec3, k: number): Vec3 {
  return [a[0] * k, a[1] * k, a[2] * k]
}
function dot(a: Vec3, b: Vec3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
function length(a: Vec3): number {
  return Math.sqrt(dot(a, a))
}
function unit(a: Vec3): Vec3 | null {
  const l = length(a)
  if (!(l > 1e-12)) return null
  return [a[0] / l, a[1] / l, a[2] / l]
}
function signedDistance(plane: PlaneDef, p: Vec3): number {
  return dot(plane.normal, sub(p, plane.point))
}

// -------------------------------------------------------------------- build

/** Build the seam-relative walk for `path` under `plane`, or `null` for an
 *  empty path. `profileCentroid` is only consulted for a CLOSED path (the
 *  mean of the hovered profile's own boundary vertices — the same point
 *  `FollowMeTool` already computes for its verdict badge). */
export function seamWalk(
  path: PathPolyline,
  plane: PlaneDef,
  profileCentroid: Vec3,
): SeamWalk | null {
  if (path.segments.length === 0) return null
  return path.closed ? closedSeamWalk(path, plane, profileCentroid) : openSeamWalk(path, plane)
}

function pointsToWalk(points: Vec3[]): SeamWalk {
  const cumulative = [0]
  let acc = 0
  for (let i = 1; i < points.length; i++) {
    acc += length(sub(points[i], points[i - 1]))
    cumulative.push(acc)
  }
  return { points, cumulative, total: acc }
}

function openSeamWalk(path: PathPolyline, plane: PlaneDef): SeamWalk {
  const n = plane.normal
  const m = path.segments.length
  const first = path.segments[0]
  const last = path.segments[m - 1]

  const score = (end: Vec3, far: Vec3, seg: PathSegment): { align: number; sd: number } => {
    const chord = unit(sub(far, end)) ?? [0, 0, 0]
    let dir = chord
    if (seg.curve !== null) {
      const radial = unit(sub(end, seg.curve.center))
      if (radial !== null) {
        const tangent = unit(sub(chord, scale(radial, dot(chord, radial))))
        if (tangent !== null) dir = tangent
      }
    }
    return { align: Math.abs(dot(n, dir)), sd: Math.abs(signedDistance(plane, end)) }
  }
  const fs = score(first.a, first.b, first)
  const ls = score(last.b, last.a, last)
  // Design §2a's priority, without the strict tolerance banding
  // `followMeStart.ts` needs for a legality claim — this only decides WHICH
  // end the preview walks from, a best-effort choice either way.
  const firstOnPlane = fs.sd <= 1e-6
  const lastOnPlane = ls.sd <= 1e-6
  const useFirst = firstOnPlane
    ? true
    : lastOnPlane
      ? false
      : fs.align !== ls.align
        ? fs.align > ls.align
        : fs.sd <= ls.sd

  const points: Vec3[] = useFirst ? [first.a] : [last.b]
  const ordered = useFirst ? path.segments : [...path.segments].reverse()
  for (const seg of ordered) {
    points.push(useFirst ? seg.b : seg.a)
  }
  return pointsToWalk(points)
}

function closedSeamWalk(path: PathPolyline, plane: PlaneDef, centroid: Vec3): SeamWalk {
  const n = plane.normal
  const m = path.segments.length

  let bestSeg = 0
  let bestT = 0
  let bestDistSq = Infinity
  for (let k = 0; k < m; k++) {
    const { a, b } = path.segments[k]
    const ab = sub(b, a)
    const len2 = dot(ab, ab)
    let t = len2 > 1e-18 ? dot(sub(centroid, a), ab) / len2 : 0
    t = Math.max(0, Math.min(1, t))
    const p = add(a, scale(ab, t))
    const d2 = dot(sub(p, centroid), sub(p, centroid))
    if (d2 < bestDistSq) {
      bestDistSq = d2
      bestSeg = k
      bestT = t
    }
  }
  const seg = path.segments[bestSeg]
  const seamPoint = add(seg.a, scale(sub(seg.b, seg.a), bestT))
  // The kernel always leaves a closed anchor along +n, re-orienting if
  // needed (design §1/§2b) — walk this segment's own chord forward if it
  // already leaves that way, backward otherwise.
  const segDir = unit(sub(seg.b, seg.a)) ?? [0, 0, 0]
  const forward = dot(n, segDir) >= 0

  const points: Vec3[] = [seamPoint, forward ? seg.b : seg.a]
  for (let i = 1; i <= m - 1; i++) {
    const k = forward ? (bestSeg + i) % m : (bestSeg - i + m * 2) % m
    points.push(forward ? path.segments[k].b : path.segments[k].a)
  }
  points.push(seamPoint) // closes the lap
  return pointsToWalk(points)
}

// ------------------------------------------------------------------- query

/**
 * The point on `walk` nearest the ray `(origin, direction)`, and its arc
 * length from the seam. For each segment of the walk, the closest point on
 * the (clamped) segment to the ray's LINE is found via the standard skew-
 * line closest-approach solution; the global winner is the one whose point
 * sits nearest the ray among all segments. `direction` need not be unit.
 */
export function nearestOnWalk(
  walk: SeamWalk,
  origin: Vec3,
  direction: Vec3,
): { point: Vec3; arcLen: number } {
  let bestDistSq = Infinity
  let bestPoint: Vec3 = walk.points[0]
  let bestArcLen = 0
  const d1 = direction
  const aa = dot(d1, d1)
  for (let i = 0; i < walk.points.length - 1; i++) {
    const a = walk.points[i]
    const b = walk.points[i + 1]
    const d2 = sub(b, a)
    const segLen = length(d2)
    if (!(segLen > 1e-12) || !(aa > 1e-18)) continue
    const r = sub(origin, a)
    const bb = dot(d1, d2)
    const cc = dot(d2, d2)
    const dd = dot(d1, r)
    const ee = dot(d2, r)
    const denom = aa * cc - bb * bb
    let s = denom > 1e-12 ? (aa * ee - bb * dd) / denom : 0
    s = Math.max(0, Math.min(1, s))
    const point = add(a, scale(d2, s))
    // Distance from `point` to the ray's own closest point, for comparison.
    const t = Math.max(0, dot(sub(point, origin), d1) / aa)
    const onRay = add(origin, scale(d1, t))
    const distSq = dot(sub(point, onRay), sub(point, onRay))
    if (distSq < bestDistSq) {
      bestDistSq = distSq
      bestPoint = point
      bestArcLen = walk.cumulative[i] + s * segLen
    }
  }
  return { point: bestPoint, arcLen: bestArcLen }
}

/** The walk's own vertices from the seam up to (and including) `arcLen`
 *  (clamped to `[0, walk.total]`), plus the exact interpolated point at
 *  `arcLen` itself — the "swept so far" sub-path for the drag preview. */
export function subWalkTo(walk: SeamWalk, arcLen: number): Vec3[] {
  const clamped = Math.max(0, Math.min(walk.total, arcLen))
  const out: Vec3[] = [walk.points[0]]
  for (let i = 1; i < walk.points.length; i++) {
    if (walk.cumulative[i] <= clamped) {
      out.push(walk.points[i])
      continue
    }
    const segLen = walk.cumulative[i] - walk.cumulative[i - 1]
    const t = segLen > 1e-12 ? (clamped - walk.cumulative[i - 1]) / segLen : 0
    // `t <= 0` lands exactly on the point already at the end of `out` (arc
    // length 0, or a clamp to a joint) — skip the redundant duplicate.
    if (t > 1e-12) {
      out.push(add(walk.points[i - 1], scale(sub(walk.points[i], walk.points[i - 1]), t)))
    }
    break
  }
  return out
}

/** The exact point on `walk` at `arcLen` (clamped to `[0, walk.total]`). */
export function pointAtArcLength(walk: SeamWalk, arcLen: number): Vec3 {
  const sub_ = subWalkTo(walk, arcLen)
  return sub_[sub_.length - 1]
}
