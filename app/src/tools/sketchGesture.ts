/**
 * Shared helper for bracketing a sketch draw-tool commit in one undo gesture
 * ("sketches are first-class interactable"). `sketch_begin_gesture` /
 * `sketch_end_gesture` group everything a tool draws in one commit — a whole
 * rectangle/circle/arc, or a single Line segment — into ONE undo step. The
 * first gesture on a freshly-created sketch also folds the sketch's own
 * creation into that step, so undoing it removes the sketch entirely.
 *
 * Handles the stale cached-handle case: RectangleTool/CircleTool/ArcTool/
 * LineTool lazily create a ground sketch via `begin_ground_sketch()` and
 * cache the handle. The Viewport hands all four draw tools ONE shared cache
 * (`makeSketchHandleCache`) so everything drawn at top level lands in the
 * same sketch — a chord drawn with the Line tool closes an Arc-tool arc into
 * a region, and mixed-tool profiles extrude. A cached handle can go bad two
 * ways, and both are caught by a PRE-CHECK before the gesture opens:
 *
 * - The gesture that created the sketch was undone: the sketch is hidden and
 *   the handle is stale (`sketch_plane` reads `undefined`).
 * - The sketch was rotated or moved off the ground plane (a whole-sketch
 *   transform keeps the handle LIVE, so liveness alone can't tell): the
 *   ground tools compute every point ON the ground plane, so reusing the
 *   handle would fail `PointOffPlane` — and NOT necessarily on the first
 *   segment. A gesture body submits several segments, each checked for
 *   planarity per call; a first edge along the rotation axis still lies on
 *   the tilted plane, so a failure-driven recovery would strand
 *   already-committed edges on the tilted sketch and record a spurious undo
 *   step. Checking the plane up front makes recovery decision happen before
 *   anything is submitted.
 *
 * With the pre-check in place, any error thrown inside the bracket is a
 * genuine kernel refusal of this gesture's own input and propagates to the
 * caller (the tool's toast path) untouched.
 */

import type { Scene as WasmScene } from '../wasm/loader'

/** Get/set access to a cached ground-sketch handle. One instance is shared
 * by every draw tool of a Viewport; `set(null)` drops a handle known to be
 * stale (the document was replaced). */
export interface SketchHandleCache {
  get(): bigint | null
  set(handle: bigint | null): void
}

/** A standalone `SketchHandleCache` boxing one nullable handle. */
export function makeSketchHandleCache(): SketchHandleCache {
  let handle: bigint | null = null
  return {
    get: () => handle,
    set: (h) => { handle = h },
  }
}

/** Tolerance for "this sketch still lies on the ground plane". Mirrors the
 * kernel's plane-membership tolerance (`kernel::tol::PLANE_DIST`); the exact
 * value is uncritical — undo re-alignment keeps an untouched ground sketch
 * exact, and any real transform moves the plane by whole meters/radians, so
 * it only needs to pass floating-point noise and fail every genuine move. */
const GROUND_PLANE_EPS = 1e-9

/** True iff `sketch` is live and still contains the ground plane (z = 0) as
 * a point set: its normal is parallel to Z and it passes through the origin.
 * Orientation-free on purpose — every ground-tool point (z = 0) lands on a
 * flipped-but-coincident plane just as well. `false` for a stale or hidden
 * handle (`sketch_plane` reads `undefined`). */
function isStillGroundSketch(wasmScene: WasmScene, sketch: bigint): boolean {
  const plane = wasmScene.sketch_plane(sketch)
  if (plane === undefined) return false
  const [px, py, pz, nx, ny, nz] = plane
  const offset = nx * px + ny * py + nz * pz
  return (
    Math.abs(nx) <= GROUND_PLANE_EPS &&
    Math.abs(ny) <= GROUND_PLANE_EPS &&
    Math.abs(offset) <= GROUND_PLANE_EPS
  )
}

/**
 * Run `body` bracketed in a sketch-drawing gesture, using (and lazily
 * creating, via `cache`) the tool's cached ground-sketch handle.
 *
 * - No cached handle yet: mints one with `begin_ground_sketch()`.
 * - Cached handle whose sketch is stale/hidden (its creating gesture was
 *   undone) or no longer on the ground plane (rotated/moved by a
 *   whole-sketch transform): retargeted up front — a fresh ground sketch is
 *   minted BEFORE the gesture opens, so nothing is ever submitted to, or
 *   recorded against, the departed sketch (see the pre-check note above).
 * - Errors from `body` — including `PointOffPlane` — are genuine kernel
 *   refusals of this gesture's input and propagate to the caller unchanged.
 * - The gesture is always closed via `sketch_end_gesture` in a `finally`,
 *   even if `body` throws — an unchanged gesture records nothing, so this is
 *   safe whether `body` fully succeeded, partially applied edits, or made
 *   none.
 *
 * `body` receives the live sketch handle and performs the actual
 * `sketch_add_segment` calls (and any `onCommit`); its return value is passed
 * through.
 */
export function runSketchGesture<T>(
  wasmScene: WasmScene,
  cache: SketchHandleCache,
  body: (sketch: bigint) => T,
): T {
  let handle = cache.get()
  if (handle !== null && !isStillGroundSketch(wasmScene, handle)) {
    handle = null // stale, hidden, or departed — retarget before opening
  }
  if (handle === null) {
    handle = wasmScene.begin_ground_sketch()
    cache.set(handle)
  }

  wasmScene.sketch_begin_gesture(handle)
  try {
    return body(handle)
  } finally {
    wasmScene.sketch_end_gesture(handle)
  }
}
