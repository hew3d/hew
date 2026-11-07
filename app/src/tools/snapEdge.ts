/**
 * snapEdge — the world endpoints of the straight edge a snap lies on, shared
 * by the tools that take a whole edge from one click: Tape Measure's parallel
 * guide and Dimension's edge dimension.
 */
import type { Snap, EditContext } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { V3 } from '../viewport/geoHelpers'
import { instanceOf } from './drawPlane'

/**
 * World endpoints `[a, b]` of the edge `snap` names, or null when it names
 * none (a point, a face, an axis or guide) or the edge is stale.
 *
 * Two sources qualify:
 * - a live Object edge (`elementKind === 'edge'`),
 * - a committed sketch edge (`elementKind === 'sketch-edge'`).
 *
 * Component-instance members: a plain `edge_endpoints`/`sketch_edge_
 * endpoints` call refuses a definition-owned object/sketch (it isn't world
 * geometry with a single world-space pose of its own). When the snap names a
 * placing `instance` — or, for a sketch edge, `editContext` is an
 * `'instance'` context — the `*_in_instance` variant is tried first,
 * composing that instance's pose into the endpoints. A group member needs
 * none of this: it's still `is_world()` in the kernel, so it already
 * resolves via the plain calls below.
 */
export function snapEdgeEndpoints(
  wasmScene: WasmScene,
  snap: Snap,
  editContext: EditContext,
): [V3, V3] | null {
  let endpoints: Float64Array | number[] | undefined
  if (snap.elementKind === 'edge' && snap.object !== undefined && snap.element !== undefined) {
    endpoints = snap.instance !== undefined
      ? wasmScene.edge_endpoints_in_instance(snap.instance, snap.object, snap.element)
      : wasmScene.edge_endpoints(snap.object, snap.element)
  } else if (
    snap.elementKind === 'sketch-edge' &&
    snap.sketch !== undefined &&
    snap.element !== undefined
  ) {
    // `snap.instance` (if the hover carried one) wins over the edit
    // context's instance; either way, a definition-owned sketch edge needs a
    // placing instance composed in to resolve world-space endpoints at all.
    const instance = snap.instance ?? instanceOf(editContext) ?? undefined
    endpoints = instance !== undefined
      ? wasmScene.sketch_edge_endpoints_in_instance(instance, snap.sketch, snap.element)
      : undefined
    if (endpoints === undefined) {
      // No instance resolved at all, OR the in-instance call refused (e.g.
      // hovering a plain world sketch while inside an unrelated instance
      // edit context — a legitimate case, not an error): fall back to the
      // ordinary world-space lookup.
      endpoints = wasmScene.sketch_edge_endpoints(snap.sketch, snap.element)
    }
  }
  if (endpoints === undefined) return null
  const [ax, ay, az, bx, by, bz] = endpoints
  return [[ax, ay, az], [bx, by, bz]]
}
