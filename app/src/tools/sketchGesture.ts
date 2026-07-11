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
 * a region, and mixed-tool profiles extrude. If the user undoes the gesture
 * that created the sketch, it is hidden and the cached handle is stale —
 * `sketch_begin_gesture` throws (`UnknownSketch`). `runSketchGesture` detects
 * that failure at the bracket's first FFI call and recovers by minting a
 * fresh ground sketch and retrying once; a second failure is a genuine error
 * and propagates to the caller as usual.
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

/**
 * Run `body` bracketed in a sketch-drawing gesture, using (and lazily
 * creating, via `cache`) the tool's cached ground-sketch handle.
 *
 * - No cached handle yet: mints one with `begin_ground_sketch()`.
 * - `sketch_begin_gesture` throws (stale handle — its creating gesture was
 *   undone since caching): mints a fresh ground sketch and retries once.
 * - The gesture is always closed via `sketch_end_gesture` in a `finally`, even
 *   if `body` throws — an unchanged gesture records nothing, so this is safe
 *   whether `body` fully succeeded, partially applied edits, or made none.
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
  if (handle === null) {
    handle = wasmScene.begin_ground_sketch()
    cache.set(handle)
  }

  try {
    wasmScene.sketch_begin_gesture(handle)
  } catch {
    // Stale cached handle — mint a fresh ground sketch and retry once.
    handle = wasmScene.begin_ground_sketch()
    cache.set(handle)
    wasmScene.sketch_begin_gesture(handle)
  }

  try {
    return body(handle)
  } finally {
    wasmScene.sketch_end_gesture(handle)
  }
}
