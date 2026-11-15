/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * filletMath — the pure arithmetic of the Fillet tool: sizing from the
 * pointer, the chamfer angle form, segment-count clamps and the sentences
 * the status bar shows when a size does not fit.
 *
 * No three.js or DOM imports; fully testable in Node.
 */

export type V3 = [number, number, number]

/** Sizes under this are treated as "the pointer has not moved yet". */
export const MIN_FILLET_SIZE = 1e-6

/** Facet counts the kernel accepts across a round or cove (kernel
 *  `SegmentsOutOfRange`). */
export const MIN_BLEND_SEGMENTS = 2
export const MAX_BLEND_SEGMENTS = 96

/** Facet counts a rounded 2D corner accepts. The kernel's own message for a
 *  count of zero talks about circles, so the tool never sends one (D-17). */
export const MIN_CORNER_SEGMENTS = 1
export const MAX_CORNER_SEGMENTS = 1024

export function clampBlendSegments(n: number): number {
  return Math.min(MAX_BLEND_SEGMENTS, Math.max(MIN_BLEND_SEGMENTS, Math.round(n)))
}

export function clampCornerSegments(n: number): number {
  return Math.min(MAX_CORNER_SEGMENTS, Math.max(MIN_CORNER_SEGMENTS, Math.round(n)))
}

/** The dihedral θ ∈ (0, π) between two faces from their outward unit
 *  normals: the angle between the in-face directions away from the edge. */
export function dihedralFromNormals(na: V3, nb: V3): number {
  const dot = Math.max(-1, Math.min(1, na[0] * nb[0] + na[1] * nb[1] + na[2] * nb[2]))
  return Math.PI - Math.acos(dot)
}

/** Round: the ball radius whose tangent line sits `d` from the edge on a
 *  face meeting the other at dihedral θ. `r = d · tan(θ/2)`. */
export function roundRadiusFromSetback(d: number, theta: number): number {
  return d * Math.tan(theta / 2)
}

/** The inverse of `roundRadiusFromSetback`. */
export function setbackFromRoundRadius(r: number, theta: number): number {
  return r / Math.tan(theta / 2)
}

export type ChamferAngle = { ok: true; setbackB: number } | { ok: false }

/**
 * Chamfer by distance and angle: setback `d` on face A and the bevel
 * leaving face A at angle `alpha`, on an edge of dihedral `theta`. By the
 * law of sines in the triangle the bevel cuts off,
 * `d_b = d_a · sin α / sin(π − θ − α)`. An angle at or beyond `π − θ`
 * (or at or under 0) cannot close the triangle, so it does not fit.
 */
export function chamferFromAngle(d: number, alpha: number, theta: number): ChamferAngle {
  if (!(alpha > 0) || !(alpha < Math.PI - theta)) return { ok: false }
  return { ok: true, setbackB: (d * Math.sin(alpha)) / Math.sin(Math.PI - theta - alpha) }
}

export interface PlaneSide {
  /** Outward unit normal of the face. */
  normal: V3
  /** A point inside the face (its vertex centroid), to orient the side. */
  inside: V3
}

export interface PointerSetback {
  /** Distance from the edge, measured in the plane, on the picked side. */
  d: number
  /** Which face's plane the pointer was read on. */
  side: 'A' | 'B'
}

function sub(a: V3, b: V3): V3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
function dot(a: V3, b: V3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
function cross(a: V3, b: V3): V3 {
  return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
function norm(a: V3): V3 | null {
  const l = Math.hypot(a[0], a[1], a[2])
  return l > 0 ? [a[0] / l, a[1] / l, a[2] / l] : null
}

/**
 * The setback the pointer asks for on an edge (§3.4): intersect the pointer
 * ray with the planes of both faces, keep the intersections that lie on their
 * face's side of the edge, take the one nearer the camera, and measure its
 * distance from the edge line in that plane. `null` when neither plane gives
 * a point on its face's side (the ray is parallel, behind the camera, or off
 * the far side of the edge).
 */
export function setbackFromPointer(
  rayOrigin: V3,
  rayDir: V3,
  edgePoint: V3,
  edgeDir: V3,
  a: PlaneSide,
  b: PlaneSide,
): PointerSetback | null {
  const dir = norm(edgeDir)
  if (dir === null) return null
  let best: { d: number; t: number; side: 'A' | 'B' } | null = null
  for (const [side, face] of [['A', a], ['B', b]] as const) {
    const denom = dot(rayDir, face.normal)
    if (Math.abs(denom) < 1e-9) continue
    const t = dot(sub(edgePoint, rayOrigin), face.normal) / denom
    if (!(t > 0)) continue
    const q: V3 = [rayOrigin[0] + rayDir[0] * t, rayOrigin[1] + rayDir[1] * t, rayOrigin[2] + rayDir[2] * t]
    // The in-plane direction from the edge into the face.
    let into = norm(cross(face.normal, dir))
    if (into === null) continue
    if (dot(sub(face.inside, edgePoint), into) < 0) into = [-into[0], -into[1], -into[2]]
    const d = dot(sub(q, edgePoint), into)
    if (!(d > 0)) continue
    if (best === null || t < best.t) best = { d, t, side }
  }
  return best === null ? null : { d: best.d, side: best.side }
}

/** The status-bar phrase for what limits a size, keyed by the kernel's
 *  `limited_by` name. */
export function limitFeatureCopy(limitedBy: string): string {
  switch (limitedBy) {
    case 'adjacent_face':
      return 'by the adjacent face'
    case 'neighbor_edge':
      return 'by the neighboring edge'
    case 'hole':
      return 'by the hole in this face'
    case 'imprint':
      return 'by the shape drawn on this face'
    case 'opposite_blend':
      return 'by the blend across the face'
    case 'short_edge':
      return 'by the length of this edge'
    case 'rim_radius':
      return "by the rim's radius"
    case 'facet_width':
      return 'by the facet width of the curved wall'
    case 'other_geometry':
      return 'by other geometry'
    default:
      return 'by the shape of the model'
  }
}

/** "Radius limited to 14 mm by the adjacent face." */
export function limitedSentence(
  label: string,
  maxMeters: number,
  limitedBy: string,
  fmt: (m: number) => string,
): string {
  return `${label} limited to ${fmt(maxMeters)} ${limitFeatureCopy(limitedBy)}.`
}

/** "This profile needs 40 mm; 25 mm is available." */
export function profileTooWideSentence(needed: number, available: number, fmt: (m: number) => string): string {
  return `This profile needs ${fmt(needed)}; ${fmt(available)} is available.`
}

/** The kernel puts the limit of a failed size in the message of its
 *  `RADIUS_TOO_LARGE: radius limited to {max} by {kind}` text, and the
 *  profile one in `PROFILE_TOO_WIDE: the profile needs {needed}; {available}
 *  is available`. These read the numbers back out of that text. */
export function parseRadiusLimit(message: string): { max: number; limitedBy: string } | null {
  const m = /radius limited to ([0-9.eE+-]+) by ([a-z_]+)/.exec(message)
  if (m === null) return null
  const max = parseFloat(m[1])
  return Number.isFinite(max) ? { max, limitedBy: m[2] } : null
}

export function parseProfileWidth(message: string): { needed: number; available: number } | null {
  const m = /needs ([0-9.eE+-]+); ([0-9.eE+-]+) is available/.exec(message)
  if (m === null) return null
  const needed = parseFloat(m[1])
  const available = parseFloat(m[2])
  return Number.isFinite(needed) && Number.isFinite(available) ? { needed, available } : null
}

// ── Profile shapes ──────────────────────────────────────────────────────────

/** The kernel's normalized profile shape (`ProfileShape` serde JSON): the
 *  corner at (0,0), face A the line v = 0, face B the line u = 0; the chain
 *  runs from (1, 0) to (0, b). */
export interface ProfileShapeJson {
  b: number
  segs: Array<
    | { kind: 'line'; to: [number, number] }
    | { kind: 'arc'; to: [number, number]; center: [number, number]; ccw: boolean }
  >
}

/** The largest arc radius in a normalized shape (1 when it has no arcs). */
export function shapeArcRadiusMax(shape: ProfileShapeJson): number {
  let prev: [number, number] = [1, 0]
  let best = 0
  for (const seg of shape.segs) {
    if (seg.kind === 'arc') {
      best = Math.max(best, Math.hypot(prev[0] - seg.center[0], prev[1] - seg.center[1]))
    }
    prev = seg.to
  }
  return best > 0 ? best : 1
}

/**
 * Facets per quarter turn of a custom profile's arcs when drawn `width` wide:
 * `max(2, segmentsPerTurn(width · r_arc_max) / 4)` (WP11 §3.1), rounded down.
 * `perTurn` is the drawing rule `segmentsPerTurn` from `arcMath`.
 */
export function customArcSegments(
  shape: ProfileShapeJson,
  width: number,
  perTurn: (radius: number) => number,
): number {
  return Math.max(2, Math.floor(perTurn(width * shapeArcRadiusMax(shape)) / 4))
}
