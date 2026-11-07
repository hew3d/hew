/**
 * Snap service — calls Scene.snap() with ground/constraint-plane fallback.
 *
 * Architecture: snap-first with fallback.
 *
 * 1. Attempt Scene.snap() — kernel inference.
 * 2. If snap() returns undefined (no snap candidate), fall back to
 *    ray∩constraintPlane when a constraint plane was supplied (kind
 *    'plane'), else intersectGroundPlane(ray) (kind 'ground').
 * 3. If snap() throws unexpectedly, log a warning and fall back.
 */

import type { Scene, SnapJs } from '../wasm/pkg/wasm_api.js'
import type { Snap } from '../tools/types'
import {
  intersectGroundPlane,
  apertureForPixelRadius,
  apertureModeFor,
  type ApertureBasis,
  type Ray,
} from './math'
import { rayPlaneIntersect } from './geoHelpers'
import { isCoarsePointer } from '../platform'

/**
 * Pixel radius to *acquire* a snap candidate. This is the radius for lines
 * (edges, guides, the drawing axes); the inference crate admits named points
 * (endpoints, midpoints, centers, quadrants) out to `POINT_SNAP_REACH` (1.75)
 * times it, about 14 px, so a corner on an edge or axis captures before the
 * line can hold the cursor off it (and releases a held point at about 18 px,
 * `POINT_RELEASE_MARGIN`). The anchor-relative soft axis is further
 * capped on screen at `SOFT_AXIS_SCREEN_REACH` (1.0) times it, so it captures
 * and lets go like an edge.
 * A selection pick (`resolve`'s `pick`) gives points no wider reach.
 */
export const SNAP_RADIUS_PX = 8

/**
 * Pixel radius to *release* an already-held discrete snap (endpoint, midpoint,
 * etc.). Larger than the acquire radius → asymmetric hysteresis, which is what
 * gives inference points their "magnetic"/sticky feel: once the dot lands on a
 * point it resists being dragged off until the cursor moves well past it,
 * rather than dropping the instant the cursor leaves the 8px acquire ring.
 */
export const SNAP_BREAK_RADIUS_PX = 16

/**
 * The same acquire→release widening ratio as `SNAP_BREAK_RADIUS_PX /
 * SNAP_RADIUS_PX`, applied to the soft-axis (`on-axis`, anchor-relative)
 * candidate's OWN angular tolerance instead of the pixel-derived `aperture`
 * (playtest-2 review finding E). `on-axis` is in `STICKY_KINDS`, but that
 * candidate never reads `aperture` at all — it has its own fixed cone
 * (`SOFT_AXIS_APERTURE` in the inference crate), so the ordinary release
 * query (wider `aperture`, unchanged) does not reach it: a held soft-axis
 * snap would drop the instant the cursor left the UNSCALED cone, with no
 * hysteresis at all. Reusing this exact ratio (rather than inventing an
 * independently-tuned second constant) keeps the soft-axis release "feel"
 * consistent with every other sticky kind's.
 */
export const SOFT_AXIS_BREAK_APERTURE_SCALE = SNAP_BREAK_RADIUS_PX / SNAP_RADIUS_PX

/**
 * Multiplier applied to BOTH `SNAP_RADIUS_PX` and `SNAP_BREAK_RADIUS_PX`
 * under a coarse pointer (`platform.isCoarsePointer`) — a fingertip covers
 * several times the screen area a mouse cursor's hotspot does, so the
 * acquire/release radii a mouse-tuned 8px/16px would starve touch input of
 * any snap at all on real geometry density. Deliberately placed HERE —
 * every `resolve()` query, the one choke point every tool's snapping goes
 * through — rather than in an individual tool, so Shop Mode's tap-to-inspect
 * and the Tape Measure (or any future touch caller) all widen together by
 * construction; a per-tool radius bump would need to be remembered and
 * re-applied at every new touch-relevant tool instead.
 */
export const COARSE_POINTER_APERTURE_SCALE = 2

/** Snap kinds that are discrete/linear inference targets worth holding onto
 * with hysteresis. Excludes the broad-area 'ground' and 'on-face' snaps, where
 * "resistance" has no meaning (any point on the face/ground is equally valid). */
const STICKY_KINDS = new Set([
  'endpoint',
  'center',
  'quadrant',
  'midpoint',
  'intersection',
  'tangent',
  'on-edge',
  'on-intersection-line',
  'on-guide',
  'on-axis',
])

/** Do two snaps refer to the same inference target? Used to decide whether a
 * held snap is still the one under (or near) the cursor after the cursor drifts
 * within the break radius. */
function sameTarget(a: Snap, b: Snap): boolean {
  return (
    a.kind === b.kind &&
    a.object === b.object &&
    a.element === b.element &&
    a.elementKind === b.elementKind &&
    a.sketch === b.sketch &&
    // Two DIFFERENT drawn curves in one sketch are different targets: their
    // analytic points carry no `element`, so without this the held snap on
    // one circle's center would be treated as still-current over a
    // neighbouring circle's.
    a.sketchCurve === b.sketchCurve &&
    // A face meets several others: the slab through a wall draws four lines
    // on the wall's one face, and each names that face. The line's ends tell
    // them apart — the kernel orders a pair of faces the same way at every
    // cursor position, so one line's ends are the same numbers each query.
    sameMeetingLine(a.meetingLine, b.meetingLine)
  )
}

function sameMeetingLine(a: Snap['meetingLine'], b: Snap['meetingLine']): boolean {
  if (a === undefined || b === undefined) return a === b
  return a.every((end, i) => end.every((c, j) => c === b[i][j]))
}

/**
 * Convert a SnapJs wasm result to our plain Snap interface, freeing the
 * wasm object afterwards.
 */
function snapJsToSnap(s: SnapJs): Snap {
  try {
    const dir = s.direction()
    const from = s.projected_from()
    const line = s.meeting_line()
    const elem = s.element()
    const elemKind = s.element_kind()
    return {
      x: s.x(),
      y: s.y(),
      z: s.z(),
      kind: s.kind(),
      direction: dir !== undefined ? [dir[0], dir[1], dir[2]] : undefined,
      projectedFrom: from !== undefined ? [from[0], from[1], from[2]] : undefined,
      piercesFace: s.pierces_face(),
      meetingLine:
        line !== undefined
          ? [
              [line[0], line[1], line[2]],
              [line[3], line[4], line[5]],
            ]
          : undefined,
      object: s.object(),
      instance: s.instance(),
      element: elem,
      elementKind: elemKind,
      sketch: s.sketch(),
      sketchRegion: s.sketch_region(),
      sketchCurve: s.sketch_curve(),
    }
  } finally {
    s.free()
  }
}

export class SnapService {
  private scene: Scene
  /** The last resolved snap, kept for magnetic hysteresis (see `resolve`). */
  private lastSnap: Snap | null = null
  /** Precision mode — see `setPrecision`. */
  private precision = false

  constructor(scene: Scene) {
    this.scene = scene
  }

  /**
   * Drop the held sticky snap without otherwise touching state (shop-mode
   * playtest finding 5). The hysteresis this class implements assumes a
   * CONTINUOUS pointer stream: a mouse hovering away from a point keeps
   * calling `resolve()` every few ms, so the acquire query naturally
   * overwrites `lastSnap` the moment it finds anything else. Discrete touch
   * taps have no such stream — between one tap's resolution and the next,
   * nothing calls `resolve()` at all — so a held sticky snap can only ever
   * be cleared by the NEXT tap's own acquire query, which is one query later
   * than a continuous pointer would need. Harmless for the editor (never
   * called there), but Shop Mode's tap-to-inspect calls this once a tap has
   * fully resolved, so the next tap starts from a clean slate rather than
   * potentially resisting release toward whatever the previous tap held.
   */
  clearHold(): void {
    this.lastSnap = null
  }

  /**
   * Turn precision snapping on or off for every subsequent query.
   *
   * Off (the default) the kernel applies its gravity profile: a drawn
   * circle's exact center and quadrant points pull harder than the endpoints
   * and midpoints of the facets crowding around them, so they win even when
   * the cursor is slightly nearer a facet point. On, every kind pulls equally
   * and the nearest candidate wins again — which is how a facet point stays
   * reachable at all. The *weighting* lives in the kernel; only this boolean
   * crosses the boundary (the kernel never learns which key is held).
   *
   * Toggling drops the held sticky snap: hysteresis exists to resist letting
   * go of a target the user is still on, and the whole point of the toggle is
   * that the target should change. Returns whether the mode actually changed,
   * so callers can skip a re-query when it did not.
   */
  setPrecision(on: boolean): boolean {
    if (this.precision === on) return false
    this.precision = on
    this.lastSnap = null
    return true
  }

  /** Whether precision snapping is currently on. */
  isPrecision(): boolean {
    return this.precision
  }

  /** One kernel snap query at a given pixel aperture; null if the kernel
   * returns no candidate or throws. `softAxisApertureScale` multiplies the
   * soft-axis candidate's own fixed cone for THIS query only (finding E) —
   * omitted for the ordinary acquire query, so it behaves exactly as before. */
  private query(
    ray: Ray,
    pixelRadius: number,
    viewportHeightPx: number,
    basis: ApertureBasis,
    anchorArr: Float64Array | null,
    lockAxis: 0 | 1 | 2 | undefined,
    lockDirection: Float64Array | null,
    constraintPlaneArr: Float64Array | null,
    offPlanePoints: boolean,
    softAxisApertureScale?: number,
    pick?: boolean,
    seeThroughPoints?: boolean,
  ): Snap | null {
    try {
      const [ox, oy, oz] = ray.origin
      const [dx, dy, dz] = ray.direction
      const aperture = apertureForPixelRadius(basis, pixelRadius, viewportHeightPx)
      const cylinder = apertureModeFor(basis) === 'cylinder'
      const result = this.scene.snap(
        ox, oy, oz,
        dx, dy, dz,
        aperture,
        anchorArr,
        lockAxis ?? null,
        constraintPlaneArr,
        this.precision,
        cylinder,
        softAxisApertureScale ?? null,
        offPlanePoints,
        pick ?? false,
        seeThroughPoints ?? false,
        lockDirection,
      )
      return result !== undefined ? snapJsToSnap(result) : null
    } catch (err) {
      console.warn('[SnapService] scene.snap() threw unexpectedly:', err)
      return null
    }
  }

  /**
   * Resolve a snap for the given ray and viewport dimensions.
   *
   * Returns a Snap (from kernel or ground/constraint-plane fallback), or null
   * if neither produced a valid intersection.
   *
   * The returned Snap has kind "ground" when it's a pure fallback with no
   * constraint plane, or "plane" when it's a pure fallback onto a supplied
   * `constraintPlane` (no kernel snap available either way). All other kind
   * strings come from the kernel.
   *
   * **Magnetic hysteresis:** discrete inference points (endpoint/midpoint/…,
   * see `STICKY_KINDS`) are *acquired* within `SNAP_RADIUS_PX` but only
   * *released* once the cursor moves past the larger `SNAP_BREAK_RADIUS_PX`.
   * The wider release query only runs when the normal query is about to lose a
   * held sticky snap, so a steady hover on a point costs a single kernel call.
   * `on-axis` (soft-axis inference) is a STICKY_KINDS member too, but its
   * candidate has its own fixed angular tolerance rather than one derived
   * from `aperture`, so the release query ALSO widens that tolerance
   * (`SOFT_AXIS_BREAK_APERTURE_SCALE`) — without it, a held soft-axis snap
   * would have no hysteresis at all (playtest-2 review finding E).
   */
  resolve(
    ray: Ray,
    viewportHeightPx: number,
    basis: ApertureBasis,
    anchor?: [number, number, number],
    lockAxis?: 0 | 1 | 2,
    /** A direction lock through `anchor`; wins over `lockAxis` (the kernel
     * normalizes it and ignores a zero or non-finite one). */
    lockDirection?: [number, number, number] | null,
    constraintPlane?: { point: [number, number, number]; normal: [number, number, number] },
    offPlanePoints?: boolean,
    /**
     * Overrides the coarse-pointer widening (`COARSE_POINTER_APERTURE_SCALE`)
     * for this one query — shop-mode playtest finding 5. Empirically (a
     * scattered face-tap E2E sweep against a 1m fixture cube), the doubled
     * acquire radius makes an EDGE win over the FACE it sits on almost
     * everywhere on the face, not just near its boundary: 5 of 7 taps
     * comfortably inside the face's silhouette resolved 'edge'. Tape Measure
     * and every other tool's own inference genuinely need the touch-widened
     * aperture to acquire thin targets at all on a phone, so this is scoped
     * to a single caller — Shop Mode's Select-tool tap-to-INSPECT
     * (`dispatchSelectPick`) passes `1` (the mouse-tuned unscaled radius,
     * still generous enough for a deliberate near-edge tap) so a casual tap
     * anywhere on a face reads as the part, not a stray edge measurement.
     * `undefined` (every other caller) keeps the existing coarse-pointer
     * behavior byte-identical.
     */
    apertureScaleOverride?: number,
    /**
     * PushPullTool's idle hover (design v1.1 Lane E): when set, only an
     * `on-face` kernel candidate — or the `constraintPlane` fallback itself
     * (`kind: 'plane'`) — is an acceptable result. Inference's rank order is
     * a hard total order (`crates/inference` `rank_group`) in which
     * `on-face` can never outrank a precise point (`endpoint`, `midpoint`,
     * …), so without this a hover near a box corner would resolve — and a
     * drag from there would push/pull from — that corner instead of the
     * face under the cursor. Any other-kind winner, held or fresh, is
     * treated exactly like "no kernel candidate": it falls through to the
     * `constraintPlane` ray intersection (step 4 below), which every caller
     * of this flag supplies alongside it.
     */
    facesOnly?: boolean,
    /**
     * A selection pick, not an inference: named points (endpoints,
     * midpoints, centers, quadrants) are admitted at `SNAP_RADIUS_PX` like
     * lines, without the kernel's wider `POINT_SNAP_REACH` that drawing tools
     * get. The Select tool's hover and click pass it, so a click in the
     * middle of a small face selects the face rather than the edge whose
     * midpoint is 10 px away.
     */
    pick?: boolean,
    /**
     * The tool drops an off-plane snap onto its `constraintPlane` and says
     * so ("projected"), so in X-ray and Wireframe hidden points and edges
     * behind the face it is held to may be offered (the kernel's
     * `see_through_points`). Unset: the plain plane filter in every mode.
     */
    seeThroughPoints?: boolean,
  ): { snap: Snap | null; fromKernel: boolean } {
    const anchorArr = anchor !== undefined ? new Float64Array(anchor) : null
    const lockDirectionArr = lockDirection != null ? new Float64Array(lockDirection) : null
    const constraintPlaneArr =
      constraintPlane !== undefined
        ? new Float64Array([...constraintPlane.point, ...constraintPlane.normal])
        : null

    // Coarse-pointer (touch) widening — see COARSE_POINTER_APERTURE_SCALE.
    const apertureScale = apertureScaleOverride ?? (isCoarsePointer() ? COARSE_POINTER_APERTURE_SCALE : 1)

    // 1. Acquire at the normal radius. Under `facesOnly`, a non-`on-face`
    //    winner is discarded here (treated as no candidate at all) so it can
    //    never be returned NOR latched as `lastSnap` — the two things that
    //    would let a corner/edge chip win the hover or get resurrected by
    //    the hysteresis step below.
    let acquired = this.query(
      ray, SNAP_RADIUS_PX * apertureScale, viewportHeightPx, basis, anchorArr, lockAxis, lockDirectionArr, constraintPlaneArr,
      offPlanePoints ?? false, undefined, pick, seeThroughPoints,
    )
    if (facesOnly && acquired !== null && acquired.kind !== 'on-face') {
      acquired = null
    }
    if (acquired !== null && STICKY_KINDS.has(acquired.kind)) {
      this.lastSnap = acquired
      return { snap: acquired, fromKernel: true }
    }

    // 2. Resist release: the acquire query lost the previously-held sticky
    //    point (returned nothing / a broad ground/face snap). Re-query at the
    //    wider break radius; if that same target is still a candidate, hold it.
    //    Also widen the soft-axis candidate's own cone by the SAME ratio
    //    (finding E) — it never reads the widened `aperture` above, so
    //    without this a held on-axis snap would have no hysteresis at all.
    //    Skipped entirely under `facesOnly`: `on-face` is never a
    //    STICKY_KINDS member, so a sticky point could only be resurrected
    //    here from a PRIOR (non-facesOnly) query's hold — never a valid
    //    result while facesOnly is asked for.
    if (!facesOnly && this.lastSnap !== null && STICKY_KINDS.has(this.lastSnap.kind)) {
      const held = this.query(
        ray, SNAP_BREAK_RADIUS_PX * apertureScale, viewportHeightPx, basis, anchorArr, lockAxis, lockDirectionArr, constraintPlaneArr,
        offPlanePoints ?? false,
        SOFT_AXIS_BREAK_APERTURE_SCALE,
        pick,
        seeThroughPoints,
      )
      if (held !== null && sameTarget(held, this.lastSnap)) {
        this.lastSnap = held
        return { snap: held, fromKernel: true }
      }
    }

    // 3. Otherwise take the acquire result (e.g. an on-face snap), if any.
    if (acquired !== null) {
      this.lastSnap = acquired
      return { snap: acquired, fromKernel: true }
    }

    // 4. Fallback: ray∩constraintPlane when one was supplied (an anchored
    //    non-ground gesture — sketches on any plane, Phase 1), else ray∩ground.
    if (constraintPlane !== undefined) {
      const p = rayPlaneIntersect(ray.origin, ray.direction, constraintPlane.point, constraintPlane.normal)
      if (p !== null) {
        this.lastSnap = { x: p[0], y: p[1], z: p[2], kind: 'plane' }
        return { snap: this.lastSnap, fromKernel: false }
      }
      this.lastSnap = null
      return { snap: null, fromKernel: false }
    }

    const pt = intersectGroundPlane(ray)
    if (pt !== null) {
      this.lastSnap = { x: pt.x, y: pt.y, z: pt.z, kind: 'ground' }
      return { snap: this.lastSnap, fromKernel: false }
    }

    this.lastSnap = null
    return { snap: null, fromKernel: false }
  }
}
