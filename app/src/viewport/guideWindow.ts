/**
 * guideWindow — which stretch of an (infinite) construction-guide line the
 * viewport actually draws.
 *
 * A guide is mathematically infinite, but a fixed ±50 m segment is a
 * rendering hazard at the scale people model at: its endpoints project
 * thousands of screen-heights away, and the rasterizer's depth interpolation
 * along that span is too noisy for the `polygonOffset` that is supposed to
 * keep a guide in front of the face it lies on (depthPolicy.ts spells out the
 * same precondition for the origin axes, whose companion is
 * `clampOriginAxes`). A guide in the plane of a 2 cm wall then vanishes and
 * returns as the camera orbits.
 *
 * So the drawn segment follows the view: centred on the point of the line
 * nearest the focus the camera is looking at, a couple of screen-widths each
 * way. Recomputing it every frame would rewrite the dash phase and buffers
 * constantly, so the window is quantised — its half-length snaps up to a power
 * of two with headroom, and the window only moves once the view would reach
 * past its end — `windowFor` returns the previous window while that holds.
 * Dash phase is the line parameter itself, so a recentre never shifts dashes.
 */

/** A drawn stretch of a guide line, as parameters along its unit direction
 *  measured from the guide's origin: the segment is `t0 … t1`. */
export interface GuideWindow {
  /** Centre parameter. */
  readonly center: number
  /** Half-length. */
  readonly half: number
}

/** Headroom the half-length keeps over what the view needs, so the window
 *  survives some focus drift and a small dolly before it must be rebuilt. */
const HEADROOM = 1.5

/** Smallest half-length ever drawn (metres): keeps a degenerate zoom from
 *  collapsing the segment to nothing. */
const MIN_HALF = 1e-3

/** Round up to a power of two, so the half-length is stable under the small
 *  zoom changes of an ordinary dolly. */
function ceilPow2(x: number): number {
  return Math.pow(2, Math.ceil(Math.log2(Math.max(x, MIN_HALF))))
}

/**
 * The window to draw for a guide whose line is nearest the focus at
 * parameter `focusT`, given the half-length `needed` the view calls for
 * either side of the focus. Returns `prev` itself (same object) while it
 * still serves: it reaches `needed` past the focus on both sides and is not
 * more than twice the size a fresh window would be.
 */
export function windowFor(focusT: number, needed: number, prev: GuideWindow | null): GuideWindow {
  const want = ceilPow2(needed * HEADROOM)
  if (prev !== null && prev.half - Math.abs(focusT - prev.center) >= needed && prev.half <= want * 2) {
    return prev
  }
  return { center: focusT, half: want }
}

type V3 = readonly [number, number, number]

/** Floor on sin(angle between a guide and the view direction) when scaling its
 *  reach, so a guide pointing straight at the camera is capped at 10× rather
 *  than infinite. */
const MIN_VIEW_SIN = 0.1

/** Cap on the perspective depth scaling of a guide's reach. */
const MAX_DEPTH_SCALE = 20

/**
 * Half-length a guide needs either side of its point nearest the focus to
 * cross the screen. `base` is what a guide square-on to the view, at the
 * focus's depth, needs (a couple of viewport lengths). Two things stretch it:
 * a shallow angle to the view direction (`dir`, unit) — the line foreshortens
 * to `sin` of its length on screen — and, in perspective, lying deeper than
 * the focus, where one screen covers more world. `dirAlong` is the guide's
 * unit direction, `nearest` its point nearest the focus, `focusDepth` the
 * focus's distance along `dir` from `eye`.
 */
export function guideReach(
  base: number,
  dirAlong: V3,
  nearest: V3,
  view: { eye: V3; dir: V3; focusDepth: number; perspective: boolean },
): number {
  const [dx, dy, dz] = dirAlong
  const [vx, vy, vz] = view.dir
  const sin = Math.hypot(dy * vz - dz * vy, dz * vx - dx * vz, dx * vy - dy * vx)
  let reach = base / Math.max(sin, MIN_VIEW_SIN)
  if (view.perspective && view.focusDepth > 1e-9) {
    const depth =
      (nearest[0] - view.eye[0]) * vx + (nearest[1] - view.eye[1]) * vy + (nearest[2] - view.eye[2]) * vz
    reach *= Math.min(Math.max(depth / view.focusDepth, 1), MAX_DEPTH_SCALE)
  }
  return reach
}

/**
 * The span of a line guide, as signed distances `lo < hi` along its unit
 * direction from its origin, read from `guide_geometry`'s
 * `[ox, oy, oz, dx, dy, dz, from, to]`. `-Infinity` / `+Infinity` mean the
 * guide runs on forever that way. A 6-element array (an older build, or a
 * mock) is an infinite guide. A reversed or empty pair is put in order so a
 * finite span is never inverted.
 */
export function guideSpanOf(geometry: ArrayLike<number>): { lo: number; hi: number } {
  let lo = geometry.length >= 8 ? geometry[6] : -Infinity
  let hi = geometry.length >= 8 ? geometry[7] : Infinity
  if (Number.isNaN(lo)) lo = -Infinity
  if (Number.isNaN(hi)) hi = Infinity
  if (lo > hi) [lo, hi] = [hi, lo]
  if (lo === hi) hi = lo + 1e-9
  return { lo, hi }
}

/**
 * Intersect the stretch `t0 … t1` with the guide's span `lo … hi`. Returns
 * the clamped pair, or null when nothing of the stretch lies on the guide
 * (for example a window around the focus that misses a short segment or the
 * far side of a ray's start).
 */
export function clampToSpan(t0: number, t1: number, lo: number, hi: number): [number, number] | null {
  const a = Math.max(t0, lo)
  const b = Math.min(t1, hi)
  return a < b ? [a, b] : null
}

/**
 * The stretch of a guide to draw, highlight, pick or print, given the
 * stretch `t0 … t1` a whole line would use: a guide with both ends finite
 * (a segment) is used whole, wherever it lies along its line, and any
 * other is that stretch clamped to its span (null when they miss).
 */
export function stretchToUse(t0: number, t1: number, lo: number, hi: number): [number, number] | null {
  if (Number.isFinite(lo) && Number.isFinite(hi)) return [lo, hi]
  return clampToSpan(t0, t1, lo, hi)
}

/** Half-length (meters) of a line guide's highlight-style overlay — the
 *  selected guide's highlight and the Move tool's guide ghost. Drawn with the
 *  depth test off, so unlike the guide itself its length cannot misplace it. */
export const GUIDE_OVERLAY_HALF_LENGTH = 50
/** Half-size (meters) of a point guide's cross marker. */
export const GUIDE_POINT_MARKER_HALF_SIZE = 0.05

/**
 * Segment endpoints (`LineSegments` positions, xyz per vertex) for a solid
 * overlay of a guide read from `guide_kind` / `guide_geometry`: a line over
 * its span, or `GUIDE_OVERLAY_HALF_LENGTH` either side of its origin where
 * the span runs on; a point as a small 3-axis cross. Null for a degenerate
 * line, a span the stretch misses, or an unknown kind.
 */
export function guideOverlayPositions(kind: string, geometry: ArrayLike<number>): number[] | null {
  if (kind === 'line') {
    const ox = geometry[0], oy = geometry[1], oz = geometry[2]
    const dx = geometry[3], dy = geometry[4], dz = geometry[5]
    const len = Math.hypot(dx, dy, dz)
    if (len < 1e-9) return null
    const ux = dx / len, uy = dy / len, uz = dz / len
    const { lo, hi } = guideSpanOf(geometry)
    const span = stretchToUse(-GUIDE_OVERLAY_HALF_LENGTH, GUIDE_OVERLAY_HALF_LENGTH, lo, hi)
    if (span === null) return null
    const [s0, s1] = span
    return [ox + ux * s0, oy + uy * s0, oz + uz * s0, ox + ux * s1, oy + uy * s1, oz + uz * s1]
  }
  if (kind === 'point') {
    const x = geometry[0], y = geometry[1], z = geometry[2]
    const k = GUIDE_POINT_MARKER_HALF_SIZE
    return [x - k, y, z, x + k, y, z, x, y - k, z, x, y + k, z, x, y, z - k, x, y, z + k]
  }
  return null
}
