/**
 * Tape Measure's "resize the model?" (design tool-parity §3) view-limit fix,
 * exercised against the REAL `OrbitControls` and `THREE.PerspectiveCamera` —
 * not a hand-rolled stand-in — because the bug this fixes (delta-review
 * Findings 1 & 2) lives entirely inside `OrbitControls.update()`'s own
 * distance clamp and `THREE.Frustum`'s own containment test. A pure-math
 * assertion on `scaleViewLimits` alone (see `math.test.ts`) cannot prove the
 * clamp is actually avoided in the library that owns it.
 *
 * `.test.tsx` (not `.test.ts`) so vitest runs it under jsdom — `OrbitControls`
 * needs a real DOM element to attach its listeners to, even though nothing
 * here dispatches a pointer event.
 */
import { describe, expect, it } from 'vitest'
import * as THREE from 'three'
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js'
import { InfiniteGrid } from './InfiniteGrid'
import {
  scaleCameraAboutOrigin,
  scaleViewLimits,
  zoomExtentsViewLimits,
  MOUNT_LIMITS,
  MOUNT_FIT_DISTANCE,
  HOME_EYE_OFFSET,
  type CameraViewLimits,
} from './math'

// Mirrors Viewport.tsx's actual mount-time construction — as of the
// tool-parity delta-review Finding 3 fix, `Viewport.tsx` consumes
// `MOUNT_LIMITS`/`HOME_EYE_OFFSET` directly (`new THREE.PerspectiveCamera(45,
// …, MOUNT_LIMITS.near, MOUNT_LIMITS.far)`, `controls.minDistance =
// MOUNT_LIMITS.minDistance`, …), so importing the SAME constants here (rather
// than re-typing the numbers a third time) ties this test to that source —
// see the "Finding 3" describe block below for the regression test that
// exercises this tie directly.
const BASE_LIMITS: CameraViewLimits = MOUNT_LIMITS

import { AXIS_HALF_LENGTH_DEFAULT } from './Viewport'

function makeCameraAndControls(
  target: [number, number, number] = [0, 0, 0],
): { camera: THREE.PerspectiveCamera; controls: OrbitControls } {
  const camera = new THREE.PerspectiveCamera(45, 1, BASE_LIMITS.near, BASE_LIMITS.far)
  camera.up.set(0, 0, 1)
  // Home-framing-ish pose (Viewport.tsx's default 3/4 view, `HOME_EYE_OFFSET`
  // at home scale 1), offset from `target` the same way, well inside
  // [minDistance, maxDistance] at the base scale.
  camera.position.set(target[0] + HOME_EYE_OFFSET[0], target[1] + HOME_EYE_OFFSET[1], target[2] + HOME_EYE_OFFSET[2])
  const dom = document.createElement('canvas')
  const controls = new OrbitControls(camera, dom)
  controls.target.set(target[0], target[1], target[2])
  controls.minDistance = BASE_LIMITS.minDistance
  controls.maxDistance = BASE_LIMITS.maxDistance
  controls.update()
  return { camera, controls }
}

/**
 * Mirrors `Viewport.tsx`'s shared `syncWorldLengthViewState` (delta-review
 * Finding 1): rescales `near`/`far`/`minDistance`/`maxDistance` alongside a
 * real `InfiniteGrid`'s plane footprint and a plain-number stand-in for the
 * closure-local `axesHalfLength`, all by the same `ratio` — the exact
 * quantities and order `applyRescaleToView`/`zoomExtents`/`setStandardView`
 * now all route through.
 */
function syncWorldLengthViewState(
  camera: THREE.PerspectiveCamera,
  controls: OrbitControls,
  grid: InfiniteGrid,
  axes: { halfLength: number },
  ratio: number,
): void {
  if (ratio === 1) return
  const limits = scaleViewLimits(
    { near: camera.near, far: camera.far, minDistance: controls.minDistance, maxDistance: controls.maxDistance },
    ratio,
  )
  camera.near = limits.near
  camera.far = limits.far
  controls.minDistance = limits.minDistance
  controls.maxDistance = limits.maxDistance
  camera.updateProjectionMatrix()
  grid.scaleAboutOrigin(ratio)
  axes.halfLength *= ratio
}

/** Re-pose eye/target by `factor` (mirrors `applyRescaleToView`'s first
 * step) and, if `limits` is given, also apply the (already-scaled) view
 * limits BEFORE the pose — matching the fixed ordering in Viewport.tsx: the
 * limits must move first, or `controls.update()`'s clamp fires against the
 * stale bound. Omitting `limits` reproduces the PRE-fix behavior (d0db79a):
 * the pose moves but minDistance/maxDistance/near/far stay fixed. */
function applyRescale(
  camera: THREE.PerspectiveCamera,
  controls: OrbitControls,
  factor: number,
  limits?: CameraViewLimits,
): void {
  if (limits !== undefined) {
    camera.near = limits.near
    camera.far = limits.far
    controls.minDistance = limits.minDistance
    controls.maxDistance = limits.maxDistance
  }
  const { eye, target } = scaleCameraAboutOrigin(
    [camera.position.x, camera.position.y, camera.position.z],
    [controls.target.x, controls.target.y, controls.target.z],
    factor,
  )
  camera.position.set(eye[0], eye[1], eye[2])
  controls.target.set(target[0], target[1], target[2])
  camera.updateProjectionMatrix()
  controls.update()
}

/** Mirrors `Viewport.tsx`'s `zoomExtents()`/`setStandardView()` re-pose: keep
 * the current view direction, re-target at `target`, and place the camera
 * `distance` away along it. Returns the resulting eye→target distance AFTER
 * `controls.update()`'s clamp, so a still-stale `minDistance` shows up as a
 * floored result instead of the intended `distance`. Module-scope (not
 * nested in one describe block) — shared by the zoomExtents AND
 * setStandardView test suites below, since both re-frame the same way. */
function reposeAtDistance(
  camera: THREE.PerspectiveCamera,
  controls: OrbitControls,
  target: [number, number, number],
  distance: number,
): number {
  const dir = new THREE.Vector3().subVectors(camera.position, controls.target).normalize()
  controls.target.set(target[0], target[1], target[2])
  camera.position.copy(controls.target).addScaledVector(dir, distance)
  camera.updateProjectionMatrix()
  controls.update()
  return camera.position.distanceTo(controls.target)
}

describe('tape-measure rescale — view limits, real OrbitControls (delta-review Finding 1)', () => {
  it('PRE-FIX repro: with minDistance/maxDistance left at their base values, a 100x rescale pins the eye→target distance at the OLD maxDistance instead of the intended 100x distance', () => {
    const { camera, controls } = makeCameraAndControls()
    const before = camera.position.distanceTo(controls.target)
    applyRescale(camera, controls, 100) // no `limits` arg — reproduces d0db79a's unfixed behavior
    const after = camera.position.distanceTo(controls.target)
    // The intended distance is 100x the original (~524.4, matching the
    // refuters' own repro number); the clamp instead pins it at the fixed
    // maxDistance of 50.
    expect(before * 100).toBeGreaterThan(BASE_LIMITS.maxDistance)
    expect(after).toBeCloseTo(BASE_LIMITS.maxDistance, 6)
    expect(after).not.toBeCloseTo(before * 100, 1)
  })

  it('FIXED: scaling minDistance/maxDistance by the same factor (scaleViewLimits) lets the eye→target distance reach the full intended 100x with no clamp', () => {
    const { camera, controls } = makeCameraAndControls()
    const before = camera.position.distanceTo(controls.target)
    const factor = 100
    const limits = scaleViewLimits(
      { near: camera.near, far: camera.far, minDistance: controls.minDistance, maxDistance: controls.maxDistance },
      factor,
    )
    applyRescale(camera, controls, factor, limits)
    const after = camera.position.distanceTo(controls.target)
    expect(after).toBeCloseTo(before * factor, 6)
  })

  it('FIXED: a 0.01x (shrink) rescale scales near DOWN and preserves the eye→target distance proportionally, with no minDistance clamp', () => {
    const { camera, controls } = makeCameraAndControls()
    const before = camera.position.distanceTo(controls.target)
    const factor = 0.01
    const limits = scaleViewLimits(
      { near: camera.near, far: camera.far, minDistance: controls.minDistance, maxDistance: controls.maxDistance },
      factor,
    )
    expect(limits.near).toBeLessThan(BASE_LIMITS.near)
    applyRescale(camera, controls, factor, limits)
    expect(camera.near).toBeCloseTo(limits.near, 10)
    const after = camera.position.distanceTo(controls.target)
    expect(after).toBeCloseTo(before * factor, 9)
  })

  // Off-origin orbit target — mirrors the refuters' proof (blank viewport
  // after undo, factor 40) that this bug needs an off-origin target to show
  // up: a target sitting exactly at the origin scales to itself, masking
  // the frustum being centered on the wrong region.
  const PRE_RESCALE_TARGET: [number, number, number] = [1, 1, 0.5]
  const PRE_RESCALE_MODEL_RADIUS = 3 // comfortably inside the base far=100
  const RESCALE_FACTOR = 40 // matches the refuters' proven undo/blank-viewport repro factor

  it('FIXED: camera.far scales so the rescaled model bounding sphere (real THREE.Frustum containment) stays inside the frustum right after the rescale commits', () => {
    const { camera, controls } = makeCameraAndControls(PRE_RESCALE_TARGET)
    // The kernel's rescale_document(factor) scales every object about the
    // WORLD ORIGIN by `factor` — the model's bounding sphere grows and
    // recenters right along with it, exactly like the camera target does
    // via scaleCameraAboutOrigin.
    const rescaledModelSphere = new THREE.Sphere(
      new THREE.Vector3(...PRE_RESCALE_TARGET).multiplyScalar(RESCALE_FACTOR),
      PRE_RESCALE_MODEL_RADIUS * RESCALE_FACTOR,
    )

    const limits = scaleViewLimits(
      { near: camera.near, far: camera.far, minDistance: controls.minDistance, maxDistance: controls.maxDistance },
      RESCALE_FACTOR,
    )
    applyRescale(camera, controls, RESCALE_FACTOR, limits)

    const projScreen = new THREE.Matrix4().multiplyMatrices(camera.projectionMatrix, camera.matrixWorldInverse)
    const frustum = new THREE.Frustum().setFromProjectionMatrix(projScreen)
    expect(frustum.intersectsSphere(rescaledModelSphere)).toBe(true)
  })

  it('FIXED vs UNFIXED, delta-review Finding 2 (post-undo blank viewport): after undo reverts the model to its tiny PRE-rescale bounding sphere, the camera (left at its post-rescale pose — camera moves are outside undo by design) still frames it when far was scaled, but far-clips it entirely when far was left at its base value', () => {
    // Both cameras go through the exact same forward-rescale re-pose (the
    // one delta-review Finding 1 already covers) — the only difference is
    // whether the view LIMITS were scaled alongside eye/target. This is
    // exactly what a rescale commit followed by an undo leaves behind: the
    // pose below is never touched by the undo (camera state is outside undo
    // by design), only the model's bounding sphere reverts.
    const fixed = makeCameraAndControls(PRE_RESCALE_TARGET)
    const fixedLimits = scaleViewLimits(
      { near: fixed.camera.near, far: fixed.camera.far, minDistance: fixed.controls.minDistance, maxDistance: fixed.controls.maxDistance },
      RESCALE_FACTOR,
    )
    applyRescale(fixed.camera, fixed.controls, RESCALE_FACTOR, fixedLimits)

    const unfixed = makeCameraAndControls(PRE_RESCALE_TARGET)
    applyRescale(unfixed.camera, unfixed.controls, RESCALE_FACTOR) // limits left at their base values

    // Undo reverts the model back to its ORIGINAL (tiny, pre-rescale)
    // bounding sphere — the camera poses above are whatever the forward
    // rescale left them at, untouched by the undo.
    const originalModelSphere = new THREE.Sphere(new THREE.Vector3(...PRE_RESCALE_TARGET), PRE_RESCALE_MODEL_RADIUS)

    function containsOriginalModel(camera: THREE.PerspectiveCamera): boolean {
      const projScreen = new THREE.Matrix4().multiplyMatrices(camera.projectionMatrix, camera.matrixWorldInverse)
      return new THREE.Frustum().setFromProjectionMatrix(projScreen).intersectsSphere(originalModelSphere)
    }

    expect(containsOriginalModel(fixed.camera)).toBe(true)
    expect(containsOriginalModel(unfixed.camera)).toBe(false)
  })

  // Zoom Extents itself is the universal recovery action (delta-review
  // Finding 1): after a rescale is UNDONE, the model reverts to its tiny
  // pre-rescale size but `controls.minDistance`/`maxDistance` (and
  // `camera.near`/`far`) stay at their scaled values — undo deliberately
  // never touches camera/view state. `OrbitControls.update()`'s own clamp
  // then floors any dolly-in attempt at that stale bound, no matter how
  // close the real fit distance is. The refuters' exact repro: a 100x
  // rescale scales `minDistance` to 0.1 * 100 = 10; undoing it reverts the
  // model to a small bounding sphere whose true Zoom Extents fit distance is
  // 2.51 — well inside that stale floor.
  const ZOOM_EXTENTS_FACTOR = 100
  const ZOOM_EXTENTS_STALE_MIN_DISTANCE = BASE_LIMITS.minDistance * ZOOM_EXTENTS_FACTOR // 10
  const ZOOM_EXTENTS_TRUE_FIT_DISTANCE = 2.51 // the refuters' exact repro number
  const ZOOM_EXTENTS_TARGET: [number, number, number] = [1, 1, 0.5] // off-origin, same reasoning as PRE_RESCALE_TARGET above

  it('PRE-FIX repro: after a 100x rescale + undo, re-posing at the TRUE (small) post-undo fit distance without resyncing the limits first still gets clamped to the stale minDistance of 10, not 2.51', () => {
    const { camera, controls } = makeCameraAndControls(ZOOM_EXTENTS_TARGET)
    const limits = scaleViewLimits(
      { near: camera.near, far: camera.far, minDistance: controls.minDistance, maxDistance: controls.maxDistance },
      ZOOM_EXTENTS_FACTOR,
    )
    applyRescale(camera, controls, ZOOM_EXTENTS_FACTOR, limits)
    expect(controls.minDistance).toBeCloseTo(ZOOM_EXTENTS_STALE_MIN_DISTANCE, 9)

    // Undo: camera/controls are left exactly as the forward rescale set
    // them (outside undo by design) — only the model bounding sphere
    // reverts, to one whose real Zoom Extents fit distance is 2.51.
    expect(ZOOM_EXTENTS_TRUE_FIT_DISTANCE).toBeLessThan(ZOOM_EXTENTS_STALE_MIN_DISTANCE)
    const result = reposeAtDistance(camera, controls, ZOOM_EXTENTS_TARGET, ZOOM_EXTENTS_TRUE_FIT_DISTANCE)
    expect(result).toBeCloseTo(ZOOM_EXTENTS_STALE_MIN_DISTANCE, 6)
    expect(result).not.toBeCloseTo(ZOOM_EXTENTS_TRUE_FIT_DISTANCE, 1)
  })

  it('FIXED: zoomExtents resyncs minDistance/maxDistance/near/far from its OWN just-computed fit distance BEFORE the re-pose, reaching the true 2.51 with no clamp', () => {
    const { camera, controls } = makeCameraAndControls(ZOOM_EXTENTS_TARGET)
    const limits = scaleViewLimits(
      { near: camera.near, far: camera.far, minDistance: controls.minDistance, maxDistance: controls.maxDistance },
      ZOOM_EXTENTS_FACTOR,
    )
    applyRescale(camera, controls, ZOOM_EXTENTS_FACTOR, limits)
    expect(controls.minDistance).toBeCloseTo(ZOOM_EXTENTS_STALE_MIN_DISTANCE, 9)

    // Undo reverts the model, not the camera/view state (same as above) —
    // Zoom Extents' fit-distance formula measures the REVERTED (small)
    // model, landing on 2.51.
    const resynced = zoomExtentsViewLimits(ZOOM_EXTENTS_TRUE_FIT_DISTANCE)
    camera.near = resynced.near
    camera.far = resynced.far
    controls.minDistance = resynced.minDistance
    controls.maxDistance = resynced.maxDistance

    const result = reposeAtDistance(camera, controls, ZOOM_EXTENTS_TARGET, ZOOM_EXTENTS_TRUE_FIT_DISTANCE)
    expect(result).toBeCloseTo(ZOOM_EXTENTS_TRUE_FIT_DISTANCE, 6)
    expect(result).not.toBeCloseTo(ZOOM_EXTENTS_STALE_MIN_DISTANCE, 0)
  })

  it('FIXED: two 100x rescale+undo cycles in a row (limits compounding to 0.1 * 100^2 = 1000) still let zoomExtents reach the true fit distance with no clamp', () => {
    const { camera, controls } = makeCameraAndControls(ZOOM_EXTENTS_TARGET)
    for (let cycle = 0; cycle < 2; cycle++) {
      const limits = scaleViewLimits(
        { near: camera.near, far: camera.far, minDistance: controls.minDistance, maxDistance: controls.maxDistance },
        ZOOM_EXTENTS_FACTOR,
      )
      applyRescale(camera, controls, ZOOM_EXTENTS_FACTOR, limits)
      // Undo never resets the limits (outside undo by design) — this is
      // exactly the "compounds across repeated rescales" the finding calls
      // out; each cycle's undo leaves the PRIOR cycle's scaled limits in
      // place for the next one to compound on top of.
    }
    expect(controls.minDistance).toBeCloseTo(ZOOM_EXTENTS_STALE_MIN_DISTANCE * ZOOM_EXTENTS_FACTOR, 3)

    const resynced = zoomExtentsViewLimits(ZOOM_EXTENTS_TRUE_FIT_DISTANCE)
    camera.near = resynced.near
    camera.far = resynced.far
    controls.minDistance = resynced.minDistance
    controls.maxDistance = resynced.maxDistance

    const result = reposeAtDistance(camera, controls, ZOOM_EXTENTS_TARGET, ZOOM_EXTENTS_TRUE_FIT_DISTANCE)
    expect(result).toBeCloseTo(ZOOM_EXTENTS_TRUE_FIT_DISTANCE, 6)
  })
})

// A model radius large enough to reproduce the desync the finding describes
// (roughly > 2.7 m on this app's default 45° fov): the resync's fresh `far`
// overshoots the fixed 150 m axis half-length once the fit distance's ratio
// to `MOUNT_FIT_DISTANCE` exceeds `AXIS_HALF_LENGTH_DEFAULT / MOUNT_LIMITS.far`
// (~1.5).
const LOCKSTEP_SCENE_RADIUS = 4

function fitDistanceForRadius(camera: THREE.PerspectiveCamera, radius: number): number {
  const fovRad = (camera.fov * Math.PI) / 180
  return (radius * 1.2) / Math.tan(fovRad / 2)
}

describe('zoomExtents — grid + axes lockstep, real InfiniteGrid (delta-review Finding 1)', () => {
  it('FIXED: zoomExtents syncs axesHalfLength and the grid footprint by the SAME ratio as camera.far — the beyond-far invariant (axesHalfLength > camera.far) holds after the resync on a radius-4m scene', () => {
    const { camera, controls } = makeCameraAndControls()
    const grid = new InfiniteGrid(0, 0, 0)
    const gridScaleBefore = grid.mesh.scale.x
    const axes = { halfLength: AXIS_HALF_LENGTH_DEFAULT }
    const farBefore = camera.far

    // Mirrors zoomExtents' own fit-distance formula, then its resync.
    const distance = fitDistanceForRadius(camera, LOCKSTEP_SCENE_RADIUS)
    const limits = zoomExtentsViewLimits(distance)
    const ratio = limits.far / camera.far
    syncWorldLengthViewState(camera, controls, grid, axes, ratio)

    // Sanity: this is the exact scenario the delta review reproduced — far
    // grows well past the OLD fixed axes half-length of 150.
    expect(farBefore).toBeLessThan(AXIS_HALF_LENGTH_DEFAULT)
    expect(camera.far).toBeGreaterThan(AXIS_HALF_LENGTH_DEFAULT)
    expect(camera.far).toBeCloseTo(limits.far, 9)

    // Grid and axes grew by the SAME ratio as far — not left at their old
    // (now comparatively tiny) size.
    expect(axes.halfLength).toBeCloseTo(AXIS_HALF_LENGTH_DEFAULT * ratio, 9)
    expect(grid.mesh.scale.x).toBeCloseTo(gridScaleBefore * ratio, 9)

    // The beyond-far invariant this fix exists to hold BY CONSTRUCTION: the
    // "infinite" axes/grid never terminate inside the frustum.
    expect(axes.halfLength).toBeGreaterThan(camera.far)
  })

  it('PRE-FIX repro (5cf4e83): with the resync applied to camera/controls ONLY (grid/axes never touched, as the shipped zoomExtents does), the beyond-far invariant is violated on the same radius-4m scene', () => {
    const { camera, controls } = makeCameraAndControls()
    const distance = fitDistanceForRadius(camera, LOCKSTEP_SCENE_RADIUS)

    // Exactly 5cf4e83's zoomExtents: assign the freshly-derived limits
    // straight to camera/controls. No InfiniteGrid or axesHalfLength exists
    // in this reproduction because the shipped code never reaches for them.
    const limits = zoomExtentsViewLimits(distance)
    camera.near = limits.near
    camera.far = limits.far
    controls.minDistance = limits.minDistance
    controls.maxDistance = limits.maxDistance
    camera.updateProjectionMatrix()

    // The fixed 150m axes half-length (AXIS_HALF_LENGTH_DEFAULT), left
    // untouched by this unfixed resync, no longer clears the new far — this
    // is the desync itself: the "infinite" axes terminate mid-scene.
    expect(camera.far).toBeGreaterThan(AXIS_HALF_LENGTH_DEFAULT)
  })
})

describe('setStandardView — resync after rescale+undo (delta-review Finding 2)', () => {
  // Reuses the same 100x rescale + undo repro as the zoomExtents suite
  // above: a rescale scales minDistance to 0.1 * 100 = 10, undo reverts the
  // model (not the camera/limits) to a bounding sphere whose true fit
  // distance is 2.51 — well inside that stale floor.
  const FACTOR = 100
  const STALE_MIN_DISTANCE = BASE_LIMITS.minDistance * FACTOR // 10
  const TRUE_FIT_DISTANCE = 2.51
  const TARGET: [number, number, number] = [1, 1, 0.5]

  it('PRE-FIX repro: setStandardView never resyncs the limits — after a 100x rescale + undo, reframing at the TRUE fit distance still clamps to the stale minDistance of 10, not 2.51', () => {
    const { camera, controls } = makeCameraAndControls(TARGET)
    const limits = scaleViewLimits(
      { near: camera.near, far: camera.far, minDistance: controls.minDistance, maxDistance: controls.maxDistance },
      FACTOR,
    )
    applyRescale(camera, controls, FACTOR, limits)
    expect(controls.minDistance).toBeCloseTo(STALE_MIN_DISTANCE, 9)

    // setStandardView, pre-fix: re-poses at the fit distance with NO resync
    // step at all (unlike zoomExtents, which at least recomputed `limits`
    // before this fix — setStandardView duplicated the framing pipeline but
    // dropped that part entirely).
    const result = reposeAtDistance(camera, controls, TARGET, TRUE_FIT_DISTANCE)
    expect(result).toBeCloseTo(STALE_MIN_DISTANCE, 6)
    expect(result).not.toBeCloseTo(TRUE_FIT_DISTANCE, 1)
  })

  it('FIXED: setStandardView routes through the same resync helper as zoomExtents, reaching the true fit distance of 2.51 with no clamp', () => {
    const { camera, controls } = makeCameraAndControls(TARGET)
    const limits = scaleViewLimits(
      { near: camera.near, far: camera.far, minDistance: controls.minDistance, maxDistance: controls.maxDistance },
      FACTOR,
    )
    applyRescale(camera, controls, FACTOR, limits)
    expect(controls.minDistance).toBeCloseTo(STALE_MIN_DISTANCE, 9)

    // setStandardView, fixed: same fit-distance derivation it already
    // computed for the re-pose, now ALSO feeding the shared resync (the
    // grid/axes stand-ins are irrelevant to this particular assertion, but
    // `syncWorldLengthViewState` always moves all three together).
    const resynced = zoomExtentsViewLimits(TRUE_FIT_DISTANCE)
    const ratio = resynced.far / camera.far
    syncWorldLengthViewState(camera, controls, new InfiniteGrid(0, 0, 0), { halfLength: AXIS_HALF_LENGTH_DEFAULT }, ratio)

    const result = reposeAtDistance(camera, controls, TARGET, TRUE_FIT_DISTANCE)
    expect(result).toBeCloseTo(TRUE_FIT_DISTANCE, 6)
    expect(result).not.toBeCloseTo(STALE_MIN_DISTANCE, 0)
  })
})

describe('mount-time constants — MOUNT_LIMITS / MOUNT_FIT_DISTANCE tie (delta-review Finding 3)', () => {
  it('a camera/controls built from MOUNT_LIMITS and HOME_EYE_OFFSET (the same exported constants Viewport.tsx\'s mount effect now consumes directly) has a fit distance that equals MOUNT_FIT_DISTANCE exactly', () => {
    const { camera, controls } = makeCameraAndControls()
    expect(camera.near).toBe(MOUNT_LIMITS.near)
    expect(camera.far).toBe(MOUNT_LIMITS.far)
    expect(controls.minDistance).toBe(MOUNT_LIMITS.minDistance)
    expect(controls.maxDistance).toBe(MOUNT_LIMITS.maxDistance)
    // The mount pose's own eye→target distance — built from HOME_EYE_OFFSET,
    // the single source MOUNT_FIT_DISTANCE is ALSO derived from — must equal
    // MOUNT_FIT_DISTANCE exactly, not merely approximately: if either constant
    // were ever re-typed as a disconnected literal instead of sharing this
    // one source, this is the assertion that would catch the drift.
    // Not exact equality: distanceTo (sqrt-based) and Math.hypot disagree
    // by 1 ulp on the raw pose, and strict equality would couple the test
    // to OrbitControls' internal spherical round-trip. The literal-drift
    // guarantee lives in the hypot identity asserted below.
    expect(camera.position.distanceTo(controls.target)).toBeCloseTo(MOUNT_FIT_DISTANCE, 12)
  })

  it('MOUNT_FIT_DISTANCE is derived from HOME_EYE_OFFSET\'s own hypot, not a separately-typed literal', () => {
    expect(MOUNT_FIT_DISTANCE).toBe(Math.hypot(HOME_EYE_OFFSET[0], HOME_EYE_OFFSET[1], HOME_EYE_OFFSET[2]))
  })
})
