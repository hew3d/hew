/**
 * Shared helper for bracketing a sketch draw-tool commit in one undo gesture
 * ("sketches are first-class interactable"). `sketch_begin_gesture` /
 * `sketch_end_gesture` group everything a tool draws in one commit — a whole
 * rectangle/circle/arc, or a single Line segment — into ONE undo step. The
 * first gesture on a freshly-created sketch also folds the sketch's own
 * creation into that step, so undoing it removes the sketch entirely.
 *
 * Sketches on any plane (the sketch-planes design §4): every commit
 * targets exactly one sketch, chosen by a `SketchTarget`:
 *
 * - `{ kind: 'existing' }` — SKETCH MODE: the tool hovered a committed
 *   sketch's own (possibly non-ground) plane and adopts it. The target
 *   handle is fixed for the whole gesture; a vanished sketch is a genuine
 *   refusal (never silently retargeted — see below).
 * - `{ kind: 'plane' }` — PLANE MODE: empty-space drawing on a plane (the
 *   ground plane today; any locked plane from Phase 3). Handles are cached
 *   per plane (`SketchPlaneCache`, keyed by `planeKey`) so everything drawn
 *   on one plane at top level — mixed Line/Rectangle/Circle/Arc — lands in
 *   the same sketch, the same shared-ground-sketch feel Phase 1 had, now
 *   generalized to every plane.
 *
 * Handles a cached PLANE handle going bad two ways, both caught by a
 * PRE-CHECK before the gesture opens:
 *
 * - The gesture that created the sketch was undone: the sketch is hidden and
 *   the handle is stale (`sketch_plane` reads `undefined`).
 * - The sketch was rotated or moved off the target plane (a whole-sketch
 *   transform keeps the handle LIVE, so liveness alone can't tell): the
 *   plane-mode tools compute every point ON the target plane, so reusing the
 *   handle would fail `PointOffPlane` — and NOT necessarily on the first
 *   segment. A gesture body submits several segments, each checked for
 *   planarity per call; a first edge along the rotation axis still lies on
 *   the tilted plane, so a failure-driven recovery would strand
 *   already-committed edges on the tilted sketch and record a spurious undo
 *   step. Checking the plane up front makes recovery decision happen before
 *   anything is submitted.
 *
 * An EXISTING (sketch-mode) target gets the same up-front liveness check,
 * but on failure it's a genuine refusal — never a silent retarget onto a
 * fresh ground sketch, which would draw the user's next click somewhere they
 * didn't aim.
 *
 * With the pre-checks in place, any error thrown inside the bracket is a
 * genuine kernel refusal of this gesture's own input and propagates to the
 * caller (the tool's toast path) untouched.
 */

import type { DrawPlane } from './drawPlane'
import { planeKey } from './drawPlane'
import type { Scene as WasmScene } from '../wasm/loader'

/** Which sketch a draw-tool gesture targets — see the module doc. */
export type SketchTarget =
  | { kind: 'existing'; handle: bigint } // sketch mode: a hover-adopted sketch
  | { kind: 'plane'; plane: DrawPlane } // plane mode: cached per plane

/** Get/set/clear access to the cached plane-mode handles, keyed by
 * `planeKey`. One instance is shared by every draw tool of a Viewport, so
 * everything drawn on one plane at top level lands in the same sketch.
 * `set(key, null)` drops a handle known to be stale; `clear()` drops every
 * cached handle (the document was replaced). */
export interface SketchPlaneCache {
  get(key: string): bigint | null
  set(key: string, handle: bigint | null): void
  clear(): void
}

/** A standalone `SketchPlaneCache` boxing a `Map<planeKey, handle>`. */
export function makeSketchPlaneCache(): SketchPlaneCache {
  const handles = new Map<string, bigint>()
  return {
    get: (key) => handles.get(key) ?? null,
    set: (key, handle) => {
      if (handle === null) handles.delete(key)
      else handles.set(key, handle)
    },
    clear: () => handles.clear(),
  }
}

/** Tolerance for "this sketch still lies on the target plane". Mirrors the
 * kernel's plane-membership tolerance (`kernel::tol::PLANE_DIST`) — see
 * `drawPlane.ts`'s `GROUND_PLANE_EPS` (the same value; kept as a separate
 * constant here since this check is orientation- AND origin-free in a way
 * `isGroundPlane` isn't). */
const PLANE_EPS = 1e-9

/**
 * True iff `sketch` is live and its plane is still the SAME plane as
 * `plane` — any point on it, any way its normal faces. Orientation-free
 * (mirrors `isGroundPlane`'s rationale): a flipped-but-coincident sketch
 * plane still accepts every point plane-mode tools compute. `false` for a
 * stale or hidden handle (`sketch_plane` reads `undefined`).
 */
function isStillOnPlane(wasmScene: WasmScene, sketch: bigint, plane: DrawPlane): boolean {
  const sketchPlane = wasmScene.sketch_plane(sketch)
  if (sketchPlane === undefined) return false
  const [px, py, pz, nx, ny, nz] = sketchPlane

  // Parallel normals, orientation-free: |cross(sketchNormal, planeNormal)| ~ 0.
  const cx = ny * plane.normal[2] - nz * plane.normal[1]
  const cy = nz * plane.normal[0] - nx * plane.normal[2]
  const cz = nx * plane.normal[1] - ny * plane.normal[0]
  if (Math.sqrt(cx * cx + cy * cy + cz * cz) > PLANE_EPS) return false

  // Same plane, not just parallel: `plane.origin` must lie on the sketch's
  // actual plane (distance along the sketch's own normal ~ 0).
  const dx = plane.origin[0] - px
  const dy = plane.origin[1] - py
  const dz = plane.origin[2] - pz
  return Math.abs(dx * nx + dy * ny + dz * nz) <= PLANE_EPS
}

/**
 * Run `body` bracketed in a sketch-drawing gesture, targeting the sketch
 * `target` names.
 *
 * - `existing`: the handle is used as-is after confirming it's live
 *   (`sketch_plane` defined) — a vanished sketch throws an `Error` whose
 *   message starts with `"UnknownSketch"` (the kernel-error toast path's
 *   convention; see `kernelErrors.ts`) instead of drawing into a fresh
 *   ground sketch the user never aimed at.
 * - `plane`: looks up (and lazily creates, via `cache`) the handle cached
 *   for `planeKey(target.plane)`.
 *   - No cached handle yet, or the cached one is stale/hidden or no longer
 *     on the target plane: retargeted up front — a fresh sketch is minted
 *     BEFORE the gesture opens, so nothing is ever submitted to, or
 *     recorded against, a departed sketch (see the pre-check note above).
 *     `target.plane.ground` mints via `begin_ground_sketch()`; any other
 *     plane mints via `begin_sketch_on_plane` (Phase 3: the idle plane
 *     lock's first click, and only reachable that way — sketch mode targets
 *     an `existing` handle instead).
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
  cache: SketchPlaneCache,
  target: SketchTarget,
  body: (sketch: bigint) => T,
): T {
  let handle: bigint

  if (target.kind === 'existing') {
    if (wasmScene.sketch_plane(target.handle) === undefined) {
      throw new Error('UnknownSketch: the hovered sketch is no longer there')
    }
    handle = target.handle
  } else {
    const key = planeKey(target.plane)
    let cached = cache.get(key)
    if (cached !== null && !isStillOnPlane(wasmScene, cached, target.plane)) {
      cached = null // stale, hidden, or departed — retarget before opening
    }
    if (cached === null) {
      const { plane } = target
      cached = plane.ground
        ? wasmScene.begin_ground_sketch()
        : wasmScene.begin_sketch_on_plane(
            plane.origin[0], plane.origin[1], plane.origin[2],
            plane.normal[0], plane.normal[1], plane.normal[2],
          )
      cache.set(key, cached)
    }
    handle = cached
  }

  wasmScene.sketch_begin_gesture(handle)
  try {
    return body(handle)
  } finally {
    wasmScene.sketch_end_gesture(handle)
  }
}
