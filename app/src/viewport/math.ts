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

// ── screen-constant sizing ───────────────────────────────────────────────────
//
// Shared math for widgets that must keep a fixed apparent PIXEL size on screen
// regardless of camera distance, field of view, or viewport resize (grip
// markers, rotate/protractor disks, slice/section-plane preview quads). See
// ScaleTool.updateGripScale's doc comment for the full derivation and the
// off-axis `1/cosθ` approximation every caller of this helper accepts (`dist`
// should be the real Euclidean camera→point distance, not view-space depth).
//
// Do NOT go back to a `k · dist` constant that bakes `tanHalfFov /
// viewportHeight` into a single number — it silently drifts the moment either
// the fov changes or the viewport is resized, which is exactly the bug this
// helper exists to fix.

/** tan(halfFovY) in radians, for a vertical field of view given in degrees. */
export function tanHalfFovRad(fovYDeg: number): number {
  return Math.tan((fovYDeg * Math.PI) / 360)
}

/**
 * World-space half-extent (or radius) that renders as `desiredPixels` pixels
 * on screen for a point at Euclidean camera distance `dist`, under a
 * perspective camera whose vertical fov gives `tanHalfFov` (pass
 * `tanHalfFovRad(camera.fov)`) and whose viewport is `viewportHeightPx` pixels
 * tall. The standard perspective-projection inverse:
 *
 *   worldHalf = desiredPixels · dist · tanHalfFov / viewportHeightPx
 *
 * Clamped to `minWorldHalf` — pass a nonzero floor (as `ScaleTool` does) so a
 * degenerate viewport height or a point very near the camera never collapses
 * a widget below a usable size; the default of 0 applies no floor.
 */
export function screenConstantWorldHalf(
  desiredPixels: number,
  dist: number,
  tanHalfFov: number,
  viewportHeightPx: number,
  minWorldHalf = 0,
): number {
  if (viewportHeightPx <= 0) return minWorldHalf
  return Math.max((desiredPixels * dist * tanHalfFov) / viewportHeightPx, minWorldHalf)
}

/**
 * Baseline (fov, viewport height) used to migrate this app's older `k · dist`
 * screen-constant widgets onto `screenConstantWorldHalf` without changing
 * their on-screen size at that baseline: the app's own default camera fov
 * (`new THREE.PerspectiveCamera(45, …)` in Viewport.tsx) and Playwright's
 * Desktop Chrome default project viewport height. Only meaningful to
 * `legacyScreenConstantToPixels` callers — a new widget should pick a
 * `desiredPixels` value directly instead of reaching for these.
 */
export const LEGACY_REFERENCE_FOV_DEG = 45
export const LEGACY_REFERENCE_VIEWPORT_HEIGHT_PX = 720

/**
 * Converts a superseded `worldSize = k · dist` screen-constant factor into the
 * equivalent `desiredPixels` for `screenConstantWorldHalf`, evaluated at a
 * reference fov/viewport — so a widget migrating off the old form keeps its
 * current apparent size at that baseline, rather than an invented new one.
 */
export function legacyScreenConstantToPixels(
  k: number,
  refFovYDeg: number,
  refViewportHeightPx: number,
): number {
  return (k * refViewportHeightPx) / tanHalfFovRad(refFovYDeg)
}

/**
 * World-space `dashSize`/`gapSize` for the origin axes' negative (dashed)
 * halves (`buildAxisLine`/`clampOriginAxes`, Viewport.tsx), kept
 * SCREEN-constant via `screenConstantWorldHalf` rather than the flat world
 * constant it replaces (former 0.28/0.22 m, which read solid at cm-scale
 * work — a whole dash+gap period dwarfed the visible model — and only read
 * clearly dashed around 10 m scale, since a fixed world length occupies a
 * shrinking fraction of the screen as the camera pulls back).
 *
 * `dist` should be the Euclidean camera→origin distance (the axes all
 * emanate from the world origin, so that's the natural single reference
 * distance for the whole dashed half, mirroring `clampOriginAxes`'s own
 * near-margin calc). `minWorld` floors both so neither collapses to (near)
 * zero world size when the camera sits right on top of the origin.
 */
export function axisDashGapWorld(
  dashScreenPx: number,
  gapScreenPx: number,
  dist: number,
  tanHalfFov: number,
  viewportHeightPx: number,
  minWorld = 0,
): { dashSize: number; gapSize: number } {
  return {
    dashSize: screenConstantWorldHalf(dashScreenPx, dist, tanHalfFov, viewportHeightPx, minWorld),
    gapSize: screenConstantWorldHalf(gapScreenPx, dist, tanHalfFov, viewportHeightPx, minWorld),
  }
}
