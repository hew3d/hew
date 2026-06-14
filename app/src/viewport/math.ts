/**
 * Pure geometric math utilities — no WebGL, no three.js, testable in Node.
 */

export interface Ray {
  origin: [number, number, number]
  direction: [number, number, number]
}

export interface Vec3 {
  x: number
  y: number
  z: number
}

/**
 * Convert a pixel snap radius and camera field-of-view (vertical, radians)
 * to a ray cone half-angle aperture (radians).
 *
 * The cone aperture is the half-angle of the cone traced through the pixel
 * radius at the near plane. Using the vertical FOV and viewport height:
 *   tanHalfFov = tan(fovY / 2)
 *   pixelAngle  = atan(snapRadiusPx / (viewportHeightPx / 2) * tanHalfFov)
 *
 * For small angles atan(x) ≈ x, but we compute exactly.
 */
export function pixelRadiusToAperture(
  snapRadiusPx: number,
  viewportHeightPx: number,
  fovYDeg: number,
): number {
  const fovYRad = (fovYDeg * Math.PI) / 180
  const tanHalfFov = Math.tan(fovYRad / 2)
  const ratio = snapRadiusPx / (viewportHeightPx / 2)
  return Math.atan(ratio * tanHalfFov)
}

/**
 * Intersect a ray with the Z=0 ground plane.
 * Returns the intersection point, or null if the ray is parallel to the plane
 * or points away from it.
 *
 * Parametric: P = origin + t * direction; solve for t where P.z = 0.
 *   t = -origin.z / direction.z
 */
export function intersectGroundPlane(ray: Ray): Vec3 | null {
  const [ox, oy, oz] = ray.origin
  const [dx, dy, dz] = ray.direction

  // Parallel or pointing away from the plane
  if (Math.abs(dz) < 1e-12) return null

  const t = -oz / dz
  if (t < 0) return null // Behind the camera

  return {
    x: ox + t * dx,
    y: oy + t * dy,
    z: 0,
  }
}
