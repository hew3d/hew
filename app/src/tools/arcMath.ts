/**
 * arcMath — pure 2-point-arc geometry for ArcTool.
 *
 * No three.js or DOM imports — fully testable in Node/vitest, mirroring the
 * moveInput.ts / lineInput.ts / viewport/geoHelpers.ts convention.
 *
 * An arc here is defined SketchUp-style by its chord endpoints A and B plus a
 * signed sagitta `s` — the distance of the bulge point from the chord's
 * midpoint, measured along the in-plane perpendicular of the chord (positive
 * on the counter-clockwise side of A→B). All math is 2D in the sketch plane;
 * `arcPolylineOnPlane` lifts the result into 3D through an orthonormal
 * in-plane basis (u, v).
 *
 * Formulas (chord half-length h = |B−A|/2, sagitta s):
 *   radius          r = (h² + s²) / (2·|s|)
 *   center          on the perpendicular bisector, at distance r − |s| from
 *                    the chord midpoint on the side OPPOSITE the bulge:
 *                    C = M − sign(s)·(r − |s|)·p   (p = unit CCW-perp of A→B)
 *   sweep magnitude |sweep| = 2·atan2(h, r − |s|)  ∈ (0, 2π)
 *                    (r − |s| < 0 for a major arc, so atan2 handles > π)
 *   sweep sign      −sign(s): the arc runs from A to B around C on the bulge
 *                    side of the chord.
 *
 * The emitted polyline's endpoints are EXACTLY the given A and B (assigned,
 * not recomputed) so chained kernel segments meet existing geometry without
 * cumulative float drift; interior vertices come from center + angle (never
 * integrated step-by-step).
 */

/** 2-element number tuple (in-plane coordinates, meters). */
export type Vec2 = [number, number]

/** 3-element number tuple (world coordinates, meters) — matches geoHelpers' V3. */
export type Vec3 = [number, number, number]

/**
 * Draw-time facet density (the true-curves design §6, ratified): the
 * chord-sagitta budget — the largest deviation a stored chord may have from
 * the true circle. Fixed at half a millimeter: about an FDM printer's
 * practical tolerance, and below silhouette visibility. The analytic
 * definition rides the curve chain regardless (center + radius), so export
 * re-faceting restores exactness on demand; this budget only bounds the
 * error of the boolean-frozen carrier.
 */
export const DRAW_SAGITTA_TOL_M = 5e-4

/** Floor of the adaptive segments-per-turn rule — small curves stay 24-gon. */
export const MIN_SEGMENTS_PER_TURN = 24

/** Cap of the adaptive rule — beyond 96 per turn, file/boolean cost buys no
 * visible silhouette and export re-faceting covers accuracy. */
export const MAX_SEGMENTS_PER_TURN = 96

/**
 * Segments per full turn for a drawn curve of `radius` meters:
 * `clamp(24, 96, ceil4(π / acos(1 − s/r)))` — the coarsest count whose
 * sagitta stays within `DRAW_SAGITTA_TOL_M`, rounded UP to a multiple of 4
 * so quadrant-angle vertices always exist on a drawn circle. Degenerate
 * radii take the floor.
 */
export function segmentsPerTurn(radius: number): number {
  if (!Number.isFinite(radius) || radius <= 0) return MIN_SEGMENTS_PER_TURN
  const ratio = DRAW_SAGITTA_TOL_M / radius
  if (ratio >= 1) return MIN_SEGMENTS_PER_TURN
  const exact = Math.PI / Math.acos(1 - ratio)
  const ceil4 = Math.ceil(exact / 4) * 4
  return Math.min(MAX_SEGMENTS_PER_TURN, Math.max(MIN_SEGMENTS_PER_TURN, ceil4))
}

/** Chords shorter than this are degenerate — no arc (mirrors CircleTool's
 * zero-radius guard scale). */
export const ARC_MIN_CHORD_M = 1e-7

/** Sagittas smaller than this are a flat (collinear) "arc" — refuse commit. */
export const ARC_MIN_SAGITTA_M = 1e-7

/** A resolved arc in 2D plane coordinates. */
export interface ArcGeometry {
  center: Vec2
  radius: number
  /** Angle of endpoint A about `center` (radians). */
  startAngle: number
  /** Signed sweep A→B (radians): positive = counter-clockwise. |sweep| ∈ (0, 2π). */
  sweep: number
}

/**
 * Signed sagitta of `bulge` relative to the chord a→b: the component of
 * (bulge − midpoint) along the chord's unit CCW-perpendicular. Positive when
 * the bulge lies on the counter-clockwise side of A→B.
 *
 * Returns null when the chord is degenerate (|b − a| < ARC_MIN_CHORD_M).
 */
export function chordSagitta(a: Vec2, b: Vec2, bulge: Vec2): number | null {
  const dx = b[0] - a[0]
  const dy = b[1] - a[1]
  const len = Math.hypot(dx, dy)
  if (len < ARC_MIN_CHORD_M) return null
  // Unit CCW perpendicular of the chord.
  const px = -dy / len
  const py = dx / len
  const mx = (a[0] + b[0]) / 2
  const my = (a[1] + b[1]) / 2
  return (bulge[0] - mx) * px + (bulge[1] - my) * py
}

/**
 * Resolve the circle (center/radius) and signed sweep for the arc from `a`
 * to `b` bulging by signed sagitta `s`.
 *
 * Returns null for degenerate input: chord shorter than ARC_MIN_CHORD_M, or
 * |s| below ARC_MIN_SAGITTA_M (a flat arc — the caller refuses the commit).
 */
export function arcFromChord(a: Vec2, b: Vec2, s: number): ArcGeometry | null {
  const dx = b[0] - a[0]
  const dy = b[1] - a[1]
  const chord = Math.hypot(dx, dy)
  if (chord < ARC_MIN_CHORD_M) return null
  if (Math.abs(s) < ARC_MIN_SAGITTA_M) return null

  const h = chord / 2
  const absS = Math.abs(s)
  const radius = (h * h + absS * absS) / (2 * absS)

  // Unit CCW perpendicular and chord midpoint.
  const px = -dy / chord
  const py = dx / chord
  const mx = (a[0] + b[0]) / 2
  const my = (a[1] + b[1]) / 2

  // Center sits r − |s| from the midpoint on the side opposite the bulge.
  const side = Math.sign(s)
  const cOff = -side * (radius - absS)
  const center: Vec2 = [mx + cOff * px, my + cOff * py]

  const startAngle = Math.atan2(a[1] - center[1], a[0] - center[0])
  // atan2(h, r − |s|) is the half-sweep: cos = (r−|s|)/r (negative for a
  // major arc, pushing the half-angle past π/2), sin = h/r.
  const sweepMag = 2 * Math.atan2(h, radius - absS)
  const sweep = -side * sweepMag

  return { center, radius, startAngle, sweep }
}

/**
 * Number of straight segments approximating a sweep of |sweep| radians at
 * `radius` meters: the adaptive per-turn density scaled by the sweep
 * fraction — `ceil(|sweep| / 2π · segmentsPerTurn(radius))` — floored at 2
 * so even a sliver arc keeps a visible bulge vertex. An arc and a circle of
 * equal radius facet at equal density (the true-curves design §6).
 */
export function arcSegmentCount(sweep: number, radius: number): number {
  const turns = Math.abs(sweep) / (2 * Math.PI)
  return Math.max(2, Math.ceil(turns * segmentsPerTurn(radius)))
}

/**
 * Faceted polyline for the arc a→b with signed sagitta `s`, in 2D plane
 * coordinates. Returns `arcSegmentCount(sweep) + 1` points; the first point
 * IS `a` and the last IS `b` (exact copies — no float drift at the joints).
 * Interior vertices are computed from center + angle.
 *
 * Returns null on degenerate input (see `arcFromChord`).
 */
export function arcPolyline(a: Vec2, b: Vec2, s: number): Vec2[] | null {
  const arc = arcFromChord(a, b, s)
  if (arc === null) return null
  const { center, radius, startAngle, sweep } = arc

  const n = arcSegmentCount(sweep, radius)
  const pts: Vec2[] = [[a[0], a[1]]]
  for (let i = 1; i < n; i++) {
    const angle = startAngle + (sweep * i) / n
    pts.push([center[0] + radius * Math.cos(angle), center[1] + radius * Math.sin(angle)])
  }
  pts.push([b[0], b[1]])
  return pts
}

/**
 * Lift `arcPolyline` into 3D: the arc lies in the plane through `a3` spanned
 * by the orthonormal in-plane basis (u, v) (e.g. from geoHelpers'
 * `facePlaneBasis`, or world X/Y for the ground plane). `b3` must lie in that
 * plane; `s` is the signed sagitta measured in the (u, v) frame (positive
 * toward +perpCCW of the chord as seen with u→v counter-clockwise).
 *
 * The first point IS `a3` and the last IS `b3` (exact copies). Returns null
 * on degenerate input.
 */
export function arcPolylineOnPlane(
  a3: Vec3,
  b3: Vec3,
  s: number,
  u: Vec3,
  v: Vec3,
): Vec3[] | null {
  // Project b into the (u, v) frame with a as the origin.
  const dx = b3[0] - a3[0]
  const dy = b3[1] - a3[1]
  const dz = b3[2] - a3[2]
  const bu = dx * u[0] + dy * u[1] + dz * u[2]
  const bv = dx * v[0] + dy * v[1] + dz * v[2]

  const pts2 = arcPolyline([0, 0], [bu, bv], s)
  if (pts2 === null) return null

  const out: Vec3[] = [[a3[0], a3[1], a3[2]]]
  for (let i = 1; i < pts2.length - 1; i++) {
    const [x, y] = pts2[i]
    out.push([
      a3[0] + u[0] * x + v[0] * y,
      a3[1] + u[1] * x + v[1] * y,
      a3[2] + u[2] * x + v[2] * y,
    ])
  }
  out.push([b3[0], b3[1], b3[2]])
  return out
}
