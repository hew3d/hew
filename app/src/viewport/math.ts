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

/**
 * Scale a camera pose by the SAME similarity `rescale_document(factor)` just
 * applied to the whole model (tool-parity design §3, Tape Measure "resize
 * the model?"): the kernel scales every object/sketch/guide about the WORLD
 * ORIGIN ([`Transform::uniform_scale`] — see `crates/kernel/src/transform.rs`
 * — composed with `Transform::scale`, itself "per-axis scale about the
 * origin"), so scaling the eye and target about that SAME pivot by the SAME
 * factor keeps the view visually IDENTICAL: the model just "is" the new
 * size, at the same apparent angular size, because the camera-to-target
 * distance grows/shrinks in lockstep with the model. Scaling about the
 * origin is `p' = pivot + factor·(p − pivot)`, which for `pivot = (0,0,0)`
 * reduces to the plain `p' = factor·p` this function computes.
 *
 * This is a view-side adjustment only — it has no undo of its own (camera
 * moves are outside undo by design in this app); the caller decides when to
 * apply it (typically once, right after a rescale commits).
 */
export function scaleCameraAboutOrigin(
  eye: [number, number, number],
  target: [number, number, number],
  factor: number,
): { eye: [number, number, number]; target: [number, number, number] } {
  return {
    eye: [eye[0] * factor, eye[1] * factor, eye[2] * factor],
    target: [target[0] * factor, target[1] * factor, target[2] * factor],
  }
}

/** The camera/controls world-LENGTH bounds `scaleViewLimits` rescales —
 * everything besides the eye/target pose that is measured in world meters
 * rather than a dimensionless ratio or an angle (see that function's doc
 * comment for why the distinction matters). */
export interface CameraViewLimits {
  near: number
  far: number
  minDistance: number
  maxDistance: number
}

/**
 * Scale a camera's near/far clip planes and an OrbitControls' min/max orbit
 * distance by the SAME `rescale_document(factor)` that `scaleCameraAboutOrigin`
 * re-poses the eye/target for (tool-parity design §3).
 *
 * These four are the OTHER world-length quantities the view holds besides
 * the eye/target pose itself, and every one of them must move in the same
 * lockstep or the rescale stops being visually identical:
 *
 *  - `minDistance`/`maxDistance` clamp the eye→target distance on every
 *    `OrbitControls.update()`. Left fixed, a big-enough factor pins the
 *    camera at the old `maxDistance` instead of the intended (scaled)
 *    distance — the model is orders of magnitude bigger than the view can
 *    back away to, and Zoom Extents can't recover either (same clamp).
 *  - `near`/`far` bound the perspective frustum. Left fixed, `far` stays at
 *    its old (now comparatively tiny) value and far-clips most or all of
 *    the rescaled model — including, after an undo restores the
 *    PRE-rescale model but leaves the (still-scaled) camera in place, the
 *    restored model itself, which can end up entirely outside `[near,
 *    far]` (a blank viewport). `near` must shrink on a SHRINK factor too —
 *    otherwise a big enough reduction near-clips a model that now sits
 *    entirely closer than the old fixed `near`.
 *
 * A factor of 1 is a no-op (returns `limits` unchanged in value, a fresh
 * object). Ratios and angles elsewhere on the camera/controls (fov,
 * dampingFactor, min/maxPolarAngle, zoomSpeed, …) are NOT world lengths and
 * are deliberately left out of this function — scaling an angle by a
 * document rescale factor would be a bug, not a fix.
 */
export function scaleViewLimits(limits: CameraViewLimits, factor: number): CameraViewLimits {
  return {
    near: limits.near * factor,
    far: limits.far * factor,
    minDistance: limits.minDistance * factor,
    maxDistance: limits.maxDistance * factor,
  }
}

/**
 * The default home-framing 3/4-view eye OFFSET (unscaled, home scale 1) —
 * `camera.position.set(HOME_EYE_OFFSET[0] * scale, HOME_EYE_OFFSET[1] *
 * scale, HOME_EYE_OFFSET[2] * scale)` against `target = (0,0,0)` — used at
 * BOTH Viewport.tsx's mount time and `setHomeFraming`'s re-pose (welcome
 * screen unit choice). Kept as ONE literal, not re-typed at each call site,
 * so `MOUNT_FIT_DISTANCE` below — which needs this exact offset to derive
 * the mount-time fit distance — cannot silently drift from what the mount
 * pose actually is (tool-parity delta-review Finding 3).
 */
export const HOME_EYE_OFFSET: [number, number, number] = [3.5, -3.0, 2.5]

/**
 * The camera/controls' MOUNT-TIME view limits (Viewport.tsx's `new
 * THREE.PerspectiveCamera(45, …, MOUNT_LIMITS.near, MOUNT_LIMITS.far)` and
 * `controls.minDistance = MOUNT_LIMITS.minDistance; controls.maxDistance =
 * MOUNT_LIMITS.maxDistance` — both consume this module's constants
 * directly rather than restating the numbers, so the two can't drift apart,
 * delta-review Finding 3), and the fit distance Zoom Extents' own framing
 * formula (`(radius * 1.2) / Math.tan(fovRad / 2)`) produces at mount time
 * for the default home-framing pose (`HOME_EYE_OFFSET`, `target = (0,0,0)`,
 * home scale 1): `Math.hypot(...HOME_EYE_OFFSET)`. `MOUNT_LIMITS` were
 * tuned BY EYE for a scene that frames at approximately this distance —
 * see `zoomExtentsViewLimits`'s doc comment for why they must be re-derived
 * as RATIOS of it rather than reused as flat constants.
 */
export const MOUNT_FIT_DISTANCE = Math.hypot(...HOME_EYE_OFFSET)

/** Mount-time `CameraViewLimits` (see `MOUNT_FIT_DISTANCE`'s doc comment) —
 * the values `zoomExtentsViewLimits` reproduces exactly when handed a fit
 * distance of `MOUNT_FIT_DISTANCE`. */
export const MOUNT_LIMITS: CameraViewLimits = { near: 0.01, far: 100, minDistance: 0.1, maxDistance: 50 }

/**
 * Derives fresh `CameraViewLimits` from a Zoom Extents fit distance
 * (tool-parity delta-review Finding 1): a Tape Measure rescale scales
 * `minDistance`/`maxDistance`/`near`/`far` by its factor (`scaleViewLimits`),
 * but that scale is permanent even after the rescale is UNDone — undo
 * restores the (tiny) pre-rescale model but deliberately never touches view
 * state (see `applyRescaleToView`'s doc comment), so `OrbitControls.update()`
 * keeps clamping the eye→target distance to the OLD scaled `minDistance`.
 * Repeated rescale/undo cycles compound the mismatch, and Zoom Extents —
 * the view's universal "recover the framing" action — couldn't break out of
 * it either, because it re-posed the camera without ever touching the very
 * limits that clamp that re-pose.
 *
 * The fix: Zoom Extents already computes a `fitDistance` from the scene's
 * OWN bounding sphere — the one piece of ground truth that is never stale,
 * unlike the view limits. Re-deriving `near`/`far`/`minDistance`/
 * `maxDistance` from THAT distance every time makes Zoom Extents a full
 * resync, recovering from any prior state (never rescaled, forward-rescaled,
 * undone, or any mix) in one call.
 *
 * The four bounds keep their ORIGINAL mount-time RATIOS to the fit distance
 * — `scaleViewLimits(MOUNT_LIMITS, fitDistance / MOUNT_FIT_DISTANCE)` — so a
 * plain unscaled scene, framed at exactly `MOUNT_FIT_DISTANCE`, reproduces
 * `MOUNT_LIMITS` exactly (ratio 1); any other fit distance scales all four
 * in the same lockstep `scaleViewLimits` already uses for a rescale. Since
 * `minDistance`/`maxDistance` bracket the fit distance itself by a wide
 * margin (~0.02x and ~10x it, from the mount-time ratios), the pose Zoom
 * Extents is about to set is never clamped by the very limits this function
 * just derived for it.
 */
export function zoomExtentsViewLimits(fitDistance: number): CameraViewLimits {
  return scaleViewLimits(MOUNT_LIMITS, fitDistance / MOUNT_FIT_DISTANCE)
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

// ── projection-agnostic (CameraRig) forms ───────────────────────────────────
//
// Everything above derives its screen-constant world size from `dist` +
// `tanHalfFov` — the perspective-projection inverse. That pair has no
// meaning under parallel (orthographic) projection, where apparent size is
// independent of camera distance and instead tracks the frustum/zoom. The
// forms below take a precomputed `worldPerPixel` (see `CameraRig.worldPerPixel`
// in `cameraRig.ts`) instead, which each projection derives its own way —
// callers (tools' `updateGripScale`/`updateDiskScale`, the origin-axis dash
// sizing) move to these so the SAME code path works in both projections with
// no `instanceof PerspectiveCamera` guard (docs/design/camera.md §1).

/**
 * World-space size of one screen pixel (vertical) for a PERSPECTIVE camera —
 * the standard projection inverse: `2 · dist · tan(fovY/2) / viewportHeightPx`.
 * `dist` is the Euclidean camera→point distance (see `screenConstantWorldHalf`'s
 * doc comment for the off-axis `1/cosθ` approximation this implies). Zero for
 * a degenerate (<=0) viewport height.
 */
export function worldPerPixelPerspective(
  dist: number,
  fovYDeg: number,
  viewportHeightPx: number,
): number {
  if (viewportHeightPx <= 0) return 0
  return (2 * dist * tanHalfFovRad(fovYDeg)) / viewportHeightPx
}

/**
 * World-space size of one screen pixel (vertical) for an ORTHOGRAPHIC camera:
 * `frustumHeight / zoom / viewportHeightPx` — deliberately independent of
 * camera distance (parallel projection keeps apparent size constant with
 * depth). This is exactly why every `tan(fov/2)`-based screen-constant
 * formula built for perspective breaks under ortho — `CameraRig.worldPerPixel`
 * dispatches here instead. Zero for a degenerate (<=0) viewport height or
 * zoom.
 */
export function worldPerPixelOrtho(
  frustumHeight: number,
  zoom: number,
  viewportHeightPx: number,
): number {
  if (viewportHeightPx <= 0 || zoom <= 0) return 0
  return frustumHeight / zoom / viewportHeightPx
}

/**
 * Ortho `minZoom`/`maxZoom` bounds for OrbitControls, mirroring the visual
 * range perspective's own `minDistance`/`maxDistance` clamp reaches
 * (Viewport.tsx `configureControls`). Before this, wheel-zoom under
 * parallel projection had NO zoom clamp at all — it could drive
 * `orthographic.zoom` toward 0, sending `CameraRig.effectiveDistance`
 * toward Infinity and silently rendering guide dashes solid.
 *
 * `toggleProjection` always resets `orthographic.zoom` to 1 at the toggle
 * instant (matching whatever the perspective distance was), so the bounds
 * are symmetric in LOG space around zoom = 1: `[1/ratio, ratio]` with
 * `ratio = sqrt(maxDistance / minDistance)`. That spans the exact same
 * total ratio perspective's own `maxDistance / minDistance` does (finest to
 * coarsest reachable world-per-pixel), split evenly either side of the
 * toggle-instant framing rather than favoring zooming in or out.
 */
export function orthoZoomBounds(
  minDistance: number,
  maxDistance: number,
): { minZoom: number; maxZoom: number } {
  const ratio = Math.sqrt(Math.max(maxDistance, 1e-9) / Math.max(minDistance, 1e-9))
  return { minZoom: 1 / ratio, maxZoom: ratio }
}

/**
 * `screenConstantWorldHalf`, parameterized on a precomputed `worldPerPixel`
 * instead of a raw `dist`/`tanHalfFov` pair. `worldPerPixel` already folds in
 * dist/fov/zoom/viewportHeight (whichever the active projection uses), so the
 * formula collapses to `desiredPixels · worldPerPixel / 2` — half the pixel
 * span becomes half the world span. Projection-agnostic: the caller computes
 * `worldPerPixel` once via `CameraRig.worldPerPixel` and this never needs to
 * know which projection is active.
 */
export function screenConstantWorldHalfFromWorldPerPixel(
  desiredPixels: number,
  worldPerPixel: number,
  minWorldHalf = 0,
): number {
  return Math.max(desiredPixels * worldPerPixel * 0.5, minWorldHalf)
}

/** `axisDashGapWorld`'s projection-agnostic form — see
 * `screenConstantWorldHalfFromWorldPerPixel`. */
export function axisDashGapWorldFromWorldPerPixel(
  dashScreenPx: number,
  gapScreenPx: number,
  worldPerPixel: number,
  minWorld = 0,
): { dashSize: number; gapSize: number } {
  return {
    dashSize: screenConstantWorldHalfFromWorldPerPixel(dashScreenPx, worldPerPixel, minWorld),
    gapSize: screenConstantWorldHalfFromWorldPerPixel(gapScreenPx, worldPerPixel, minWorld),
  }
}

/**
 * What a kernel snap query's pick tolerance should be derived from, for the
 * active projection — `SnapService` threads this through instead of a raw
 * `fovYDeg` so the same query code works in both projections (docs/design/
 * camera.md §1, "Snap aperture").
 *
 * `'perspective'` is the kernel's natural angular CONE (`ApertureMode::Cone`
 * — see `inference`'s crate docs): apparent size shrinks with depth, so the
 * pick tolerance should too. `'parallel'` is the kernel's CYLINDER mode
 * (`ApertureMode::Cylinder`): a constant world-space radius around the ray,
 * independent of depth — the right shape under orthographic projection,
 * where apparent size does NOT shrink with depth, so neither should the
 * pick tolerance. `worldPerPixel` (`CameraRig.worldPerPixel`) is already
 * distance-independent under parallel projection, so this basis needs
 * nothing else — no reference/target distance, unlike a synthesized cone
 * would (see `apertureForPixelRadius`).
 */
export type ApertureBasis =
  | { kind: 'perspective'; fovYDeg: number }
  | { kind: 'parallel'; worldPerPixel: number }

/**
 * Converts a pixel snap radius to a kernel pick-tolerance value for the
 * given `basis` (`SnapService` passes this straight through as
 * `Scene.snap`'s `aperture` argument, alongside `apertureModeFor`'s
 * `cylinder` flag): perspective returns a half-angle in radians
 * (`pixelRadiusToAperture`); parallel returns a world-space radius in
 * meters (`pixelRadius * worldPerPixel`) — the two units `ApertureMode`
 * (`inference`) expects for its `Cone`/`Cylinder` variants respectively.
 */
export function apertureForPixelRadius(
  basis: ApertureBasis,
  pixelRadius: number,
  viewportHeightPx: number,
): number {
  if (basis.kind === 'perspective') {
    return pixelRadiusToAperture(pixelRadius, viewportHeightPx, basis.fovYDeg)
  }
  return pixelRadius * basis.worldPerPixel
}

/** Whether `Scene.snap`'s `cylinder` flag should be set for `basis` — `true`
 * under parallel projection (`ApertureMode::Cylinder`), `false` under
 * perspective (`ApertureMode::Cone`, the kernel's default). */
export function apertureModeFor(basis: ApertureBasis): 'cone' | 'cylinder' {
  return basis.kind === 'parallel' ? 'cylinder' : 'cone'
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
