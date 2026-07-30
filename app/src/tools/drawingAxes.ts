/**
 * drawingAxes — the document-level MOVABLE DRAWING AXES frame (tool-parity
 * design §4): a document-wide origin + red(X)/green(Y)/blue(Z) direction
 * triple that reorients drawing and inference, independent of the world
 * frame the ground grid stays pinned to.
 *
 * The kernel is the single source of truth (`Scene::axes` /
 * `Scene::set_axes`, wasm-api) — this module is the thin app-side reshaping
 * layer every consumer reads instead of hard-coding world X/Y/Z, mirroring
 * `planeFromSketch` in `drawPlane.ts` (one raw wasm call, reshaped into a
 * typed JS object). Inference (axis snapping/locking) already reads the
 * kernel frame on its own — nothing here pushes anything into it.
 */

import type { V3 } from '../viewport/geoHelpers'
import type { Scene as WasmScene } from '../wasm/loader'

/** The current drawing axes: an orthonormal, right-handed frame (z = x×y,
 *  derived kernel-side — never set directly). */
export interface DrawingAxes {
  origin: V3
  /** Red axis. */
  x: V3
  /** Green axis. */
  y: V3
  /** Blue axis (derived, x × y). */
  z: V3
}

/** The world-identity frame — origin at (0,0,0), axes aligned with world
 *  X/Y/Z. Every consumer's legacy behavior (pre-movable-axes) is this frame,
 *  so callers use it both as the literal default and as the comparison
 *  target for `isWorldIdentity`. */
export const WORLD_DRAWING_AXES: DrawingAxes = {
  origin: [0, 0, 0],
  x: [1, 0, 0],
  y: [0, 1, 0],
  z: [0, 0, 1],
}

/**
 * Read the document's current drawing axes from the kernel
 * (`Scene::axes` — 12 floats: origin, unit x, unit y, derived unit z).
 * Reshapes the flat buffer into `DrawingAxes`; does no validation of its
 * own (the kernel guarantees an orthonormal, right-handed result, since
 * `set_axes` is the only way to move it and it refuses anything else).
 */
export function getDrawingAxes(wasmScene: WasmScene): DrawingAxes {
  const a = wasmScene.axes()
  return {
    origin: [a[0], a[1], a[2]],
    x: [a[3], a[4], a[5]],
    y: [a[6], a[7], a[8]],
    z: [a[9], a[10], a[11]],
  }
}

/** Float tolerance for `isWorldIdentity` — comfortably above float64 noise
 *  from a round-trip through the kernel's own storage, comfortably below
 *  any real user-placed frame. */
const WORLD_IDENTITY_EPS = 1e-9

/**
 * True iff `frame` is (within float tolerance of) the world-identity frame
 * — the fast-path test every consumer that wants to preserve BIT-IDENTICAL
 * legacy behavior (e.g. `axisDrawPlane`'s ground fast path) checks first,
 * so an untouched document draws/infers exactly as it always has.
 */
export function isWorldIdentity(frame: DrawingAxes): boolean {
  const close = (a: number, b: number): boolean => Math.abs(a - b) <= WORLD_IDENTITY_EPS
  return (
    close(frame.origin[0], 0) && close(frame.origin[1], 0) && close(frame.origin[2], 0) &&
    close(frame.x[0], 1) && close(frame.x[1], 0) && close(frame.x[2], 0) &&
    close(frame.y[0], 0) && close(frame.y[1], 1) && close(frame.y[2], 0) &&
    close(frame.z[0], 0) && close(frame.z[1], 0) && close(frame.z[2], 1)
  )
}
