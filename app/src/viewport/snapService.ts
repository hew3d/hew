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
import { intersectGroundPlane, pixelRadiusToAperture, type Ray } from './math'
import { rayPlaneIntersect } from './geoHelpers'

/** Pixel radius to *acquire* a snap candidate. */
export const SNAP_RADIUS_PX = 8

/**
 * Pixel radius to *release* an already-held discrete snap (endpoint, midpoint,
 * etc.). Larger than the acquire radius → asymmetric hysteresis, which is what
 * gives inference points their "magnetic"/sticky feel: once the dot lands on a
 * point it resists being dragged off until the cursor moves well past it,
 * rather than dropping the instant the cursor leaves the 8px acquire ring.
 */
export const SNAP_BREAK_RADIUS_PX = 16

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
    a.sketchCurve === b.sketchCurve
  )
}

/**
 * Convert a SnapJs wasm result to our plain Snap interface, freeing the
 * wasm object afterwards.
 */
function snapJsToSnap(s: SnapJs): Snap {
  try {
    const dir = s.direction()
    const elem = s.element()
    const elemKind = s.element_kind()
    return {
      x: s.x(),
      y: s.y(),
      z: s.z(),
      kind: s.kind(),
      direction: dir !== undefined ? [dir[0], dir[1], dir[2]] : undefined,
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
   * returns no candidate or throws. */
  private query(
    ray: Ray,
    pixelRadius: number,
    viewportHeightPx: number,
    fovYDeg: number,
    anchorArr: Float64Array | null,
    lockAxis: 0 | 1 | 2 | undefined,
    constraintPlaneArr: Float64Array | null,
  ): Snap | null {
    try {
      const [ox, oy, oz] = ray.origin
      const [dx, dy, dz] = ray.direction
      const aperture = pixelRadiusToAperture(pixelRadius, viewportHeightPx, fovYDeg)
      const result = this.scene.snap(
        ox, oy, oz,
        dx, dy, dz,
        aperture,
        anchorArr,
        lockAxis ?? null,
        constraintPlaneArr,
        this.precision,
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
   */
  resolve(
    ray: Ray,
    viewportHeightPx: number,
    fovYDeg: number,
    anchor?: [number, number, number],
    lockAxis?: 0 | 1 | 2,
    constraintPlane?: { point: [number, number, number]; normal: [number, number, number] },
  ): { snap: Snap | null; fromKernel: boolean } {
    const anchorArr = anchor !== undefined ? new Float64Array(anchor) : null
    const constraintPlaneArr =
      constraintPlane !== undefined
        ? new Float64Array([...constraintPlane.point, ...constraintPlane.normal])
        : null

    // 1. Acquire at the normal radius.
    const acquired = this.query(
      ray, SNAP_RADIUS_PX, viewportHeightPx, fovYDeg, anchorArr, lockAxis, constraintPlaneArr,
    )
    if (acquired !== null && STICKY_KINDS.has(acquired.kind)) {
      this.lastSnap = acquired
      return { snap: acquired, fromKernel: true }
    }

    // 2. Resist release: the acquire query lost the previously-held sticky
    //    point (returned nothing / a broad ground/face snap). Re-query at the
    //    wider break radius; if that same target is still a candidate, hold it.
    if (this.lastSnap !== null && STICKY_KINDS.has(this.lastSnap.kind)) {
      const held = this.query(
        ray, SNAP_BREAK_RADIUS_PX, viewportHeightPx, fovYDeg, anchorArr, lockAxis, constraintPlaneArr,
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
