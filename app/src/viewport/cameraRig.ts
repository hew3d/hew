/**
 * CameraRig — the projection abstraction behind Parallel Projection
 * (docs/design/camera.md §1).
 *
 * Before this, the viewport owned a single closure-local `PerspectiveCamera`
 * and everything screen-constant derived from `camera.fov` (see the survey
 * at the top of `camera.md`). `CameraRig` owns BOTH a perspective and an
 * orthographic camera, keeps them pose-synchronized, and exposes the two
 * primitives every screen-constant/framing consumer needs so none of them
 * has to know which projection is active:
 *
 *   - `worldPerPixel(dist, viewportHeightPx)` — replaces every
 *     `tan(fov/2)`-based screen-constant formula.
 *   - `effectiveDistance(controlsDistance)` — a distance-driven consumer
 *     (guide dashing, the grid shader) feeds this instead of
 *     `controls.getDistance()` directly, so the same call works in both
 *     projections without its own branch.
 *
 * Only one of `perspective`/`orthographic` is ever the *live* render camera
 * (`active`); the other is kept fully configured (pose, near/far) so a
 * toggle back is instant and stable rather than reconstructed from scratch.
 */
import * as THREE from 'three'
import {
  tanHalfFovRad,
  worldPerPixelPerspective,
  worldPerPixelOrtho,
  apertureForPixelRadius,
  type ApertureBasis,
} from './math'

export type Projection = 'perspective' | 'parallel'

/** Clamp bounds for `setFov` (design §2) — SketchUp's own Field of View
 * dialog clamps to a similar range; degenerate values (0, 180+) make the
 * screen-constant math and the frustum itself meaningless. */
export const MIN_FOV_DEG = 1
export const MAX_FOV_DEG = 120

/**
 * Reference vertical fov (degrees) `effectiveDistance` uses to translate an
 * orthographic frustum/zoom into a "distance" a perspective-shaped consumer
 * can use. Arbitrary but must be a fixed constant: it cancels out of the
 * ratio ANY distance-driven consumer computes from the returned value (they
 * always re-derive a screen-constant size via the SAME reference), so its
 * exact numeric choice does not matter as long as it never changes.
 */
const EFFECTIVE_DISTANCE_REFERENCE_FOV_DEG = 45

export class CameraRig {
  readonly perspective: THREE.PerspectiveCamera
  readonly orthographic: THREE.OrthographicCamera
  /** Whichever of `perspective`/`orthographic` is the live render camera.
   * Typed as the concrete union (not bare `THREE.Camera`) so callers that
   * only ever hold `active` — never one of the two named fields — still get
   * `.near`/`.far`/`.position`/`.quaternion` etc. without a cast, while a
   * projection-specific member (`.fov`, `.zoom`'s ortho meaning) still forces
   * an explicit `projection` check. That's deliberate: a call site reaching
   * for `.fov` on `active` is almost always a bug (docs/design/camera.md §1
   * — the whole point of the rig is that generic consumers never need to
   * know which camera is live). */
  active: THREE.PerspectiveCamera | THREE.OrthographicCamera
  projection: Projection

  constructor(aspect: number, near = 0.01, far = 100) {
    this.perspective = new THREE.PerspectiveCamera(45, aspect, near, far)
    // Placeholder frustum — `toggleProjection`/`frameOrthoToRadius` size it
    // for real before it ever becomes the active camera; only its aspect
    // ratio matters before that (kept in sync by `setAspect`).
    const halfH = 1
    this.orthographic = new THREE.OrthographicCamera(-halfH * aspect, halfH * aspect, halfH, -halfH, near, far)
    this.active = this.perspective
    this.projection = 'perspective'
  }

  /**
   * Toggle between perspective and parallel projection, keeping the view
   * visually stable AT THE ORBIT TARGET (design §1):
   *
   * - perspective → parallel: size the ortho frustum to match what's on
   *   screen at `target` right now — `halfH = dist · tan(fovY/2)`.
   * - parallel → perspective: derive the distance that reproduces the
   *   current ortho `halfH` at the (persisted) perspective fov, and dolly
   *   the eye to it along the unchanged view direction.
   *
   * Pose (quaternion/up) and near/far are carried over unconditionally; only
   * position (parallel→perspective) and the frustum (perspective→parallel)
   * change. Round-tripping twice from the same pose reproduces it exactly
   * (up to floating-point) — the toggle is its own inverse.
   */
  toggleProjection(target: THREE.Vector3): void {
    if (this.projection === 'perspective') {
      this.matchOrthoToPerspective(target)
      this.orthographic.updateProjectionMatrix()
      this.active = this.orthographic
      this.projection = 'parallel'
    } else {
      this.matchPerspectiveToOrtho(target)
      this.perspective.updateProjectionMatrix()
      this.active = this.perspective
      this.projection = 'perspective'
    }
  }

  /**
   * Sync the currently INACTIVE camera's pose — and, when it's the
   * orthographic one, its frustum — to the ACTIVE camera's pose right now,
   * WITHOUT flipping `active`/`projection` (unlike `toggleProjection`,
   * which flips both). For a caller that re-poses the active camera
   * OUTSIDE `toggleProjection` (`Viewport.setHomeFraming`, re-homing the
   * view) — so a LATER toggle starts the newly-active camera from THIS
   * pose/frustum instead of whatever stale placeholder or leftover-zoom
   * pose the long-inactive camera was last left at. Reuses the exact same
   * pose/frustum math `toggleProjection` itself uses (`matchOrthoToPerspective`/
   * `matchPerspectiveToOrtho`), just without the active/projection flip.
   */
  syncInactiveCamera(target: THREE.Vector3): void {
    if (this.projection === 'perspective') {
      this.matchOrthoToPerspective(target)
      this.orthographic.updateProjectionMatrix()
    } else {
      this.matchPerspectiveToOrtho(target)
      this.perspective.updateProjectionMatrix()
    }
  }

  /** Size+pose the orthographic camera from the perspective camera's
   * CURRENT pose/fov (the perspective→parallel half of `toggleProjection`,
   * factored out so `syncInactiveCamera` can reuse it without also
   * flipping `active`/`projection`). Does not call
   * `updateProjectionMatrix()` — callers do that themselves after,
   * matching the two use sites' own sequencing. */
  private matchOrthoToPerspective(target: THREE.Vector3): void {
    // Degenerate only if the eye sits exactly ON the target (never true in
    // practice — OrbitControls enforces `minDistance` — but a zero `dist`
    // would otherwise collapse the ortho frustum to a point and NaN its
    // projection matrix). Floored, not special-cased, so the frustum stays
    // merely tiny rather than invalid.
    const dist = Math.max(this.perspective.position.distanceTo(target), 1e-6)
    const halfH = dist * tanHalfFovRad(this.perspective.fov)
    const halfW = halfH * this.perspective.aspect
    this.orthographic.left = -halfW
    this.orthographic.right = halfW
    this.orthographic.top = halfH
    this.orthographic.bottom = -halfH
    this.orthographic.zoom = 1
    this.orthographic.near = this.perspective.near
    this.orthographic.far = this.perspective.far
    this.orthographic.position.copy(this.perspective.position)
    this.orthographic.quaternion.copy(this.perspective.quaternion)
    this.orthographic.up.copy(this.perspective.up)
  }

  /** Pose (and dolly-distance) the perspective camera from the orthographic
   * camera's CURRENT frustum/pose (the parallel→perspective half of
   * `toggleProjection`, factored out — see `matchOrthoToPerspective`).
   * Does not call `updateProjectionMatrix()`. */
  private matchPerspectiveToOrtho(target: THREE.Vector3): void {
    const halfH = (this.orthographic.top - this.orthographic.bottom) / (2 * this.orthographic.zoom)
    const dist = halfH / tanHalfFovRad(this.perspective.fov)
    const dir = this.orthographic.position.clone().sub(target)
    // Degenerate only if the eye sits exactly ON the target (never true in
    // practice — OrbitControls enforces `minDistance` — but a zero-length
    // direction would otherwise NaN the new position outright).
    if (dir.lengthSq() < 1e-12) dir.copy(this.orthographic.up).negate()
    dir.normalize()
    this.perspective.near = this.orthographic.near
    this.perspective.far = this.orthographic.far
    this.perspective.position.copy(target).addScaledVector(dir, dist)
    this.perspective.quaternion.copy(this.orthographic.quaternion)
    this.perspective.up.copy(this.orthographic.up)
  }

  /** Set the perspective vertical fov (degrees), clamped to
   * [`MIN_FOV_DEG`, `MAX_FOV_DEG`]. No-op on the orthographic frustum — fov
   * only ever applies to (and persists across toggles on) `perspective`. */
  setFov(deg: number): void {
    this.perspective.fov = Math.min(MAX_FOV_DEG, Math.max(MIN_FOV_DEG, deg))
    this.perspective.updateProjectionMatrix()
  }

  /** Keep both cameras' frustums matched to a new viewport aspect ratio.
   * Perspective just sets `.aspect`; orthographic rescales its half-WIDTH
   * only (half-height, i.e. current zoom level, is preserved) — a resize
   * must not change what's already framed vertically. */
  setAspect(aspect: number): void {
    this.perspective.aspect = aspect
    this.perspective.updateProjectionMatrix()
    const halfH = (this.orthographic.top - this.orthographic.bottom) / 2
    const halfW = halfH * aspect
    this.orthographic.left = -halfW
    this.orthographic.right = halfW
    this.orthographic.updateProjectionMatrix()
  }

  /**
   * World size of one screen pixel (vertical), for the ACTIVE projection —
   * replaces every `tan(fov/2)`-based screen-constant formula (design §1).
   * `dist` is the Euclidean camera→point distance; ignored under parallel
   * projection (apparent size there is distance-independent by definition —
   * see `worldPerPixelOrtho`).
   */
  worldPerPixel(dist: number, viewportHeightPx: number): number {
    if (this.projection === 'perspective') {
      return worldPerPixelPerspective(dist, this.perspective.fov, viewportHeightPx)
    }
    const frustumHeight = this.orthographic.top - this.orthographic.bottom
    return worldPerPixelOrtho(frustumHeight, this.orthographic.zoom, viewportHeightPx)
  }

  /**
   * A "distance" a perspective-shaped consumer (guide-dash sizing, the grid
   * shader's LOD) can use regardless of projection: BOTH branches return the
   * distance at which the reference fov (`EFFECTIVE_DISTANCE_REFERENCE_FOV_DEG`)
   * would produce the SAME `worldPerPixel` the camera actually has right now
   * — so a zoom-in/out (either projection) rescales these consumers
   * identically, and — the property that matters here — a `toggleProjection`
   * call never jumps the returned value on its own.
   *
   * The design (camera.md §1) describes the perspective branch as simply
   * `controls.getDistance()` verbatim; that only agrees with the parallel
   * branch's reference-fov normalization when `perspective.fov` happens to
   * equal the reference. At any other fov the two branches would disagree by
   * `tanHalf(actualFov)/tanHalf(referenceFov)` — e.g. toggling projection at
   * fov=90° with a 10 m distance jumped the returned value from 10 to ~24.14
   * with nothing on screen having changed, snapping guide-dash/grid sizing
   * right at the toggle instant. Normalizing the perspective branch the same
   * way closes that gap; it also happens to leave the common case (default
   * 45° fov) numerically identical to `controlsDistance`, so this is a
   * strict correctness fix, not a behavior change at the shipped default.
   */
  effectiveDistance(controlsDistance: number): number {
    const referenceTanHalf = tanHalfFovRad(EFFECTIVE_DISTANCE_REFERENCE_FOV_DEG)
    if (this.projection === 'perspective') {
      return (controlsDistance * tanHalfFovRad(this.perspective.fov)) / referenceTanHalf
    }
    // Same degenerate-zoom guard as `worldPerPixelOrtho` (math.ts) — an
    // unclamped wheel-zoom could otherwise drive `orthographic.zoom` toward
    // 0 and this toward Infinity (`minZoom`/`maxZoom` on OrbitControls now
    // prevent that in practice, but this stays defensive against any other
    // caller that sets `zoom` directly, e.g. a future Scenes/persistence
    // load).
    if (this.orthographic.zoom <= 0) return 0
    const frustumHeight = this.orthographic.top - this.orthographic.bottom
    return frustumHeight / (this.orthographic.zoom * 2 * referenceTanHalf)
  }

  /** Perspective distance that fits a sphere of `radius` within the vertical
   * fov with `margin` slack (e.g. 1.2 — SketchUp's own Zoom Extents/standard-
   * view margin). Framing (`zoomExtents`/`setStandardView`) uses this for
   * BOTH projections' camera *position* (see module doc) — only the
   * orthographic *frustum* additionally needs `frameOrthoToRadius`. */
  perspectiveFramingDistance(radius: number, margin: number): number {
    return (radius * margin) / tanHalfFovRad(this.perspective.fov)
  }

  /** Size the orthographic frustum (half-height `radius · margin`, half-width
   * scaled by `aspect`, zoom reset to 1) to fit a sphere of `radius` — the
   * parallel-projection analogue of `perspectiveFramingDistance` (design §1,
   * "Framing"). Does not touch camera position/orientation. */
  frameOrthoToRadius(radius: number, margin: number, aspect: number): void {
    const halfH = radius * margin
    const halfW = halfH * aspect
    this.orthographic.left = -halfW
    this.orthographic.right = halfW
    this.orthographic.top = halfH
    this.orthographic.bottom = -halfH
    this.orthographic.zoom = 1
    this.orthographic.updateProjectionMatrix()
  }

  /** Rescale the orthographic frustum by `factor` around its current
   * center (Zoom Window's parallel-projection path, design §3): `factor < 1`
   * zooms in (frustum shrinks), matching a perspective dolly-in by the same
   * factor applied to distance. Implemented as an inverse `zoom` change so
   * `left`/`right`/`top`/`bottom` (and therefore the pan/target framing)
   * stay untouched — only apparent size changes. */
  scaleOrthoFrustum(factor: number): void {
    if (factor <= 0) return
    this.orthographic.zoom = this.orthographic.zoom / factor
    this.orthographic.updateProjectionMatrix()
  }

  /** The `ApertureBasis` (see `math.ts`) for the active projection — the
   * single call site every `SnapService.resolve` caller builds from, so
   * snapping works in both projections with no branch at the call site.
   * No distance parameter (unlike phase 1's interim synthesis): a true
   * cylindrical tolerance (design camera.md §1) needs only `worldPerPixel`,
   * which is itself distance-independent under parallel projection — `dist`
   * passed to `worldPerPixel` below is ignored for exactly that reason. */
  apertureBasis(viewportHeightPx: number): ApertureBasis {
    if (this.projection === 'perspective') {
      return { kind: 'perspective', fovYDeg: this.perspective.fov }
    }
    return {
      kind: 'parallel',
      worldPerPixel: this.worldPerPixel(0, viewportHeightPx),
    }
  }
}

/** Pixel-radius → kernel pick-tolerance value (radians under perspective,
 * world-meters under parallel — see `math.ts`'s `apertureForPixelRadius`)
 * for the active projection — thin wrapper so viewport call sites don't need
 * to import `math.ts` AND `cameraRig.ts` just to build a basis then convert
 * it. */
export function apertureForPixelRadiusOn(rig: CameraRig, pixelRadius: number, viewportHeightPx: number): number {
  return apertureForPixelRadius(rig.apertureBasis(viewportHeightPx), pixelRadius, viewportHeightPx)
}

/**
 * Whether world point `p` is "behind" `camera` — projection-agnostic, via
 * camera-space z sign (through `camera.matrixWorldInverse`, the same matrix
 * `Vector3.project` itself applies). three.js cameras look down their own
 * local -Z regardless of projection, so anything AT or behind the eye has
 * camera-space z >= 0 in EITHER projection.
 *
 * Replaces the old NDC-space `v.z > 1` heuristic (`Viewport.tsx`'s
 * `worldToPixels`): that was a perspective-only proxy for "beyond the eye"
 * riding the perspective divide's sign flip, and it INVERTS under
 * orthographic — there is no perspective divide there, so NDC z is just a
 * linear near/far remap that never exceeds 1 for a point behind the eye
 * (those map to NDC z < -1 instead), while genuinely visible near/far
 * points still read as "in front". This check is uniform across both.
 *
 * Requires `camera.matrixWorldInverse` to be current — exactly the same
 * requirement `.project()` already imposes on every established call site,
 * so this adds no new staleness risk.
 */
export function isBehindCamera(p: THREE.Vector3, camera: THREE.Camera): boolean {
  return p.clone().applyMatrix4(camera.matrixWorldInverse).z >= 0
}
