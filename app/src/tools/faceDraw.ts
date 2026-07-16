/**
 * faceDraw — the shared "may this face be drawn on directly?" policy for the
 * draw tools (Line / Rectangle / Circle / Arc), plus the per-pointer-event
 * pick cache they all use to avoid re-running the O(faces) `pick_face`
 * raycast two or three times for the same ray (snapConstraint and the
 * pointer-move/down dispatcher are called back-to-back by the Viewport).
 *
 * Policy ("plain objects are immediately editable"):
 *   - Inside an entered object context: only that object's faces.
 *   - Top level: any PLAIN object's face — ungrouped and not part of a
 *     component instance. Groups and Components keep their explicit
 *     double-click editing step, so a face belonging to instanced geometry
 *     or to an object nested in a group is not directly drawable.
 *
 * The Viewport can inject a richer predicate (via each tool's
 * `setFaceEligibility`) that also understands group/instance editing
 * contexts — the tools themselves only know the entered-object id.
 *
 * What happens on an INELIGIBLE face differs by tool class, deliberately:
 *
 * - The DRAW tools fall through to ground mode and draw on the plane
 *   beneath. Drawing is additive and fully previewed — the rubber-band
 *   shows exactly where the ink will land before anything commits — and
 *   drawing on the ground plane through whatever stands above it is these
 *   tools' long-standing top-level behavior (SketchUp's too). Pinned in
 *   RectangleTool.test.ts / LineTool.test.ts ("grouped → ground mode").
 * - PUSH/PULL instead CONSUMES the click with an explanatory toast
 *   (PushPullTool). Its fallthrough target would be an existing sketch
 *   region along the same ray — falling through would silently start a
 *   drag on, and then extrude, geometry the user did not aim at. A
 *   mutation of existing geometry must fail closed where new, previewed
 *   ink may fall through.
 */

import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'

/** May the face on `object` (hit through `instance`, when the ray struck
 *  instanced geometry) be drawn on directly? */
export type FaceEligible = (object: bigint, instance: bigint | undefined) => boolean

/** An eligible face under the cursor: the pick's object + face handles. */
export interface EligibleFacePick {
  object: bigint
  face: bigint
}

/**
 * The default tool-local policy (used when the Viewport hasn't injected one):
 * scoped to the entered object inside a context; at top level, plain
 * (ungrouped, non-instanced) objects only.
 */
export function defaultFaceEligible(
  wasmScene: WasmScene,
  activeContext: bigint | null,
  object: bigint,
  instance: bigint | undefined,
): boolean {
  if (activeContext !== null) {
    return instance === undefined && object === activeContext
  }
  if (instance !== undefined) return false
  // kind 0 = object; a defined parent means it lives inside a group.
  return wasmScene.node_parent(0, object) === undefined
}

/**
 * Memoizes the single `pick_face` raycast for the CURRENT pointer event.
 * Keyed by reference equality on the `Ray` passed in (the Viewport builds one
 * Ray object per event); a miss just falls back to a fresh pick.
 * `eligible: null` means either nothing was hit, or a face was hit but the
 * eligibility predicate rejected it.
 */
export class FacePickCache {
  private cache: { ray: Ray; eligible: EligibleFacePick | null } | null = null

  pickFor(
    wasmScene: WasmScene,
    ray: Ray,
    isEligible: FaceEligible,
  ): EligibleFacePick | null {
    if (this.cache !== null && this.cache.ray === ray) {
      return this.cache.eligible
    }
    const pick = wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    let eligible: EligibleFacePick | null = null
    if (pick !== undefined) {
      try {
        const object = pick.object()
        const instance = pick.instance()
        if (isEligible(object, instance)) {
          eligible = { object, face: pick.face() }
        }
      } finally {
        pick.free()
      }
    }
    this.cache = { ray, eligible }
    return eligible
  }

  clear(): void {
    this.cache = null
  }
}
