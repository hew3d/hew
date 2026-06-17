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

/** Pixel radius treated as a snap candidate */
export const SNAP_RADIUS_PX = 8

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

  constructor(scene: Scene) {
    this.scene = scene
  }

  /**
   * Resolve a snap for the given ray and viewport dimensions.
   *
   * Returns a Snap (from kernel or ground-plane fallback), or null if neither
   * produced a valid intersection.
   *
   * The returned Snap has kind "ground" when it's a pure fallback (no kernel
   * snap available). All other kind strings come from the kernel.
   */
  resolve(
    ray: Ray,
    viewportHeightPx: number,
    fovYDeg: number,
    anchor?: [number, number, number],
    lockAxis?: 0 | 1 | 2,
    constraintPlane?: { point: [number, number, number]; normal: [number, number, number] },
  ): { snap: Snap | null; fromKernel: boolean } {
    try {
      const [ox, oy, oz] = ray.origin
      const [dx, dy, dz] = ray.direction
      const aperture = pixelRadiusToAperture(SNAP_RADIUS_PX, viewportHeightPx, fovYDeg)

      let anchorArr: Float64Array | null = null
      if (anchor !== undefined) {
        anchorArr = new Float64Array(anchor)
      }

      let constraintPlaneArr: Float64Array | null = null
      if (constraintPlane !== undefined) {
        constraintPlaneArr = new Float64Array([
          ...constraintPlane.point,
          ...constraintPlane.normal,
        ])
      }

      const result = this.scene.snap(
        ox, oy, oz,
        dx, dy, dz,
        aperture,
        anchorArr,
        lockAxis ?? null,
        constraintPlaneArr,
      )

      if (result !== undefined) {
        return { snap: snapJsToSnap(result), fromKernel: true }
      }
      // snap returned undefined — no kernel snap; fall through to fallback
    } catch (err) {
      console.warn('[SnapService] scene.snap() threw unexpectedly:', err)
    }

    // Ground-plane fallback
    const pt = intersectGroundPlane(ray)
    if (pt !== null) {
      return {
        snap: { x: pt.x, y: pt.y, z: pt.z, kind: 'ground' },
        fromKernel: false,
      }
    }

    return { snap: null, fromKernel: false }
  }
}
