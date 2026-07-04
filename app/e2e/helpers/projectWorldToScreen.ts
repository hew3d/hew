/**
 * Canvas-driving helper (docs/DEVELOPMENT.md, strategy 2 — "pixel interaction").
 *
 * The viewport is an opaque WebGL canvas, so DOM selectors cannot "click the
 * edge." The deterministic way to drive a real pixel interaction is:
 *
 *   1. pin the camera to a fixed matrix at test start (so the projection is
 *      stable across runs/machines), then
 *   2. project a *known world point* through that camera to a canvas pixel and
 *      `page.mouse.move(px, py)` there.
 *
 * This module is the pure-math half of that: world → NDC → canvas/page pixel.
 * It is intentionally dependency-free (no three.js) so it unit-tests in Vitest
 * and carries no renderer coupling. The matrix layout matches three.js'
 * `Matrix4.elements` (column-major, 16 numbers), so a caller can pass
 * `camera.projectionMatrix.clone().multiply(camera.matrixWorldInverse).elements`
 * straight in.
 *
 * The camera-pinning hook is the semantic harness's `setCamera`
 * (`window.__hew_test`); a pixel test pins the app camera with it and
 * builds the matching view-projection via `buildViewProjection` below —
 * `input-pipeline.spec.ts` is the pattern. See also PINNED_CAMERA.
 */

export interface Vec3 {
  x: number
  y: number
  z: number
}

export interface Vec2 {
  x: number
  y: number
}

/** A canvas's position+size in CSS pixels, as from `getBoundingClientRect()`. */
export interface CanvasRect {
  left: number
  top: number
  width: number
  height: number
}

/** A 4x4 matrix in three.js column-major order (`Matrix4.elements`). */
export type Mat4 = readonly number[] // length 16

/**
 * Project a world point to Normalized Device Coordinates through a
 * view-projection matrix. Returns NDC in [-1, 1]^3 plus the clip `w`; callers
 * that care about "is it in front of the camera" check `w > 0`. Returns null
 * when `w` is ~0 (point on the camera plane — projection is undefined).
 */
export function worldToNdc(
  world: Vec3,
  viewProjection: Mat4,
): { x: number; y: number; z: number; w: number } | null {
  const e = viewProjection
  const { x, y, z } = world
  // Column-major: e[col*4 + row]. Same arithmetic as three.js
  // Vector3.applyMatrix4 on a (x, y, z, 1) homogeneous point.
  const clipX = e[0] * x + e[4] * y + e[8] * z + e[12]
  const clipY = e[1] * x + e[5] * y + e[9] * z + e[13]
  const clipZ = e[2] * x + e[6] * y + e[10] * z + e[14]
  const clipW = e[3] * x + e[7] * y + e[11] * z + e[15]
  if (Math.abs(clipW) < 1e-12) return null
  const inv = 1 / clipW
  return { x: clipX * inv, y: clipY * inv, z: clipZ * inv, w: clipW }
}

/**
 * Convert NDC (x,y in [-1,1], +y up) to a page-pixel coordinate over a canvas,
 * which is exactly what Playwright `mouse.move/down/up` consume. NDC +y is up;
 * screen +y is down, hence the y flip. The result is offset by the canvas'
 * `left`/`top` so it is in page coordinates, not canvas-local.
 */
export function ndcToPagePixel(ndc: Vec2, rect: CanvasRect): Vec2 {
  return {
    x: rect.left + (ndc.x * 0.5 + 0.5) * rect.width,
    y: rect.top + (-ndc.y * 0.5 + 0.5) * rect.height,
  }
}

/**
 * Compose the two: world point + view-projection + canvas rect → page pixel,
 * or null if the point is behind/on the camera plane (`w <= 0`). This is the
 * function a pixel-drag test calls to turn a known model vertex into a
 * `mouse.move` target.
 */
export function worldToPagePixel(
  world: Vec3,
  viewProjection: Mat4,
  rect: CanvasRect,
): Vec2 | null {
  const ndc = worldToNdc(world, viewProjection)
  if (ndc === null || ndc.w <= 0) return null
  return ndcToPagePixel(ndc, rect)
}

/** Camera parameters a view-projection can be built from — the same shape the
 * harness's `setCamera` consumes (PINNED_CAMERA below is one of these). */
export interface CameraParams {
  position: Vec3
  target: Vec3
  up: Vec3
  fovDeg: number
  near: number
  far: number
}

/**
 * Build the view-projection matrix for a pinned camera, in three.js
 * conventions (column-major `Matrix4.elements`, vertical `fovDeg`, right-
 * handed camera space looking down −Z). This is the "pixel test builds the
 * view-projection from PINNED_CAMERA itself" half promised in the module
 * doc: `harness.setCamera(...)` pins the app's real camera to the same
 * parameters, and `worldToPagePixel(world, buildViewProjection(...), rect)`
 * then lands `mouse.move/click` on the pixel where the app rendered that
 * world point. Note NDC x/y (all a pixel test consumes) depend only on
 * fov + aspect — near/far shape only NDC z, so they need not match the
 * app's exact clip planes.
 */
export function buildViewProjection(cam: CameraParams, aspect: number): Mat4 {
  // --- view matrix: inverse of the camera's lookAt world transform ---
  const sub = (a: Vec3, b: Vec3): Vec3 => ({ x: a.x - b.x, y: a.y - b.y, z: a.z - b.z })
  const cross = (a: Vec3, b: Vec3): Vec3 => ({
    x: a.y * b.z - a.z * b.y,
    y: a.z * b.x - a.x * b.z,
    z: a.x * b.y - a.y * b.x,
  })
  const norm = (a: Vec3): Vec3 => {
    const l = Math.hypot(a.x, a.y, a.z)
    if (l < 1e-12) throw new Error('buildViewProjection: degenerate camera basis')
    return { x: a.x / l, y: a.y / l, z: a.z / l }
  }
  const dot = (a: Vec3, b: Vec3): number => a.x * b.x + a.y * b.y + a.z * b.z

  const zAxis = norm(sub(cam.position, cam.target)) // camera looks down −Z
  const xAxis = norm(cross(cam.up, zAxis))
  const yAxis = cross(zAxis, xAxis)

  // Column-major view matrix: rows are the camera basis vectors, translation
  // column is −basis·position.
  const view: number[] = [
    xAxis.x, yAxis.x, zAxis.x, 0,
    xAxis.y, yAxis.y, zAxis.y, 0,
    xAxis.z, yAxis.z, zAxis.z, 0,
    -dot(xAxis, cam.position), -dot(yAxis, cam.position), -dot(zAxis, cam.position), 1,
  ]

  // --- perspective projection (three.js PerspectiveCamera layout) ---
  const f = 1 / Math.tan((cam.fovDeg * Math.PI) / 360)
  const { near, far } = cam
  const proj: number[] = [
    f / aspect, 0, 0, 0,
    0, f, 0, 0,
    0, 0, (far + near) / (near - far), -1,
    0, 0, (2 * far * near) / (near - far), 0,
  ]

  // vp = proj * view, both column-major (e[col*4 + row]).
  const vp = new Array<number>(16)
  for (let col = 0; col < 4; col++) {
    for (let row = 0; row < 4; row++) {
      let acc = 0
      for (let k = 0; k < 4; k++) acc += proj[k * 4 + row] * view[col * 4 + k]
      vp[col * 4 + row] = acc
    }
  }
  return vp
}

/**
 * The canonical pinned camera for pixel/visual E2E. Deterministic across runs
 * and machines so a projected world point lands on the same pixel every time.
 * A three.js-side helper (in the E2E setup) builds the actual view-projection
 * matrix from these parameters; keeping the parameters here — pure data — lets
 * both the harness and the pixel math agree on one source of truth.
 *
 * Values mirror a standard iso-ish framing of the origin; feed them to both
 * `harness.setCamera` and `buildViewProjection` so the app and the test agree.
 */
export const PINNED_CAMERA = {
  position: { x: 8, y: 6, z: 8 } as Vec3,
  target: { x: 0, y: 0, z: 0 } as Vec3,
  up: { x: 0, y: 0, z: 1 } as Vec3, // Hew world-up is +Z (camera commit 1ae7772)
  fovDeg: 50,
  near: 0.1,
  far: 1000,
} as const
