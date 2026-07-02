/**
 * Snap service — calls Scene.snap() with ground-plane fallback.
 *
 * Architecture: snap-first with fallback.
 *
 * 1. Attempt Scene.snap() — kernel inference.
 * 2. If snap() returns undefined (no snap candidate), fall back to
 *    intersectGroundPlane(ray).
 * 3. If snap() throws unexpectedly, log a warning and fall back.
 */

import type { Scene, SnapJs } from '../wasm/pkg/wasm_api.js'
import type { Snap } from '../tools/types'
import { intersectGroundPlane, pixelRadiusToAperture, type Ray } from './math'

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
  'midpoint',
  'intersection',
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
    a.elementKind === b.elementKind
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
      element: elem,
      elementKind: elemKind,
    }
  } finally {
    s.free()
  }
}

export class SnapService {
  private scene: Scene
  /** The last resolved snap, kept for magnetic hysteresis (see `resolve`). */
  private lastSnap: Snap | null = null

  constructor(scene: Scene) {
    this.scene = scene
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
   * Returns a Snap (from kernel or ground-plane fallback), or null if neither
   * produced a valid intersection.
   *
   * The returned Snap has kind "ground" when it's a pure fallback (no kernel
   * snap available). All other kind strings come from the kernel.
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

    // 4. Ground-plane fallback.
    const pt = intersectGroundPlane(ray)
    if (pt !== null) {
      this.lastSnap = { x: pt.x, y: pt.y, z: pt.z, kind: 'ground' }
      return { snap: this.lastSnap, fromKernel: false }
    }

    this.lastSnap = null
    return { snap: null, fromKernel: false }
  }
}
