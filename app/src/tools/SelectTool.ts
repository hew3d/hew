/**
 * SelectTool — M1 minimal implementation, extended for sketch selection (
 * "sketches are first-class interactable").
 *
 * Hover-highlight via snap() provenance; click uses pick_face() for reliable
 * object-face detection (snap prefers vertices/edges and can miss face intent).
 * On a pick_face miss, falls back through two sketch pickers so a free-standing
 * (not-yet-extruded) sketch is selectable both by its edges and by clicking
 * inside a closed region it forms:
 *   1. pick_sketch_edge() — nearest live sketch edge within the pick
 *      aperture. Selects THAT edge (SketchUp-style): Delete then removes
 *      just the line, merging the regions it separated.
 *   2. pick_sketch_region() — the closed region under the ray (across ALL
 *      live sketches, including regions the standing-solid gate would
 *      refuse to extrude); clicking INSIDE a drawn rectangle/circle selects
 *      its owning sketch as a whole.
 * Fires onSelect(objectId) on a face hit; onSelect(null, undefined, sketchId,
 * edgeId) on an edge hit; onSelect(null, undefined, sketchId) on an interior
 * hit; onSelect(null) on a total miss.
 */

import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'

export type OnSelect = (
  objectId: bigint | null,
  instanceId?: bigint,
  sketchId?: bigint,
  sketchEdgeId?: bigint,
  sketchRegionId?: bigint,
) => void

export class SelectTool implements Tool {
  readonly name = 'Select'

  /** Last snap seen on hover (for status display) */
  lastSnap: Snap | null = null

  private wasmScene: WasmScene
  private onSelect: OnSelect

  constructor(wasmScene: WasmScene, onSelect: OnSelect) {
    this.wasmScene = wasmScene
    this.onSelect = onSelect
  }

  onPointerMove(snap: Snap | null, _ray: Ray): void {
    this.lastSnap = snap
  }

  onPointerDown(_snap: Snap | null, ray: Ray): void {
    // Use pick_face rather than snap — snap biases toward vertices/edges
    // so a face click may not return a face hit; pick_face always returns
    // the nearest surface the ray passes through.
    const pick = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (pick !== undefined) {
      try {
        const objectId = pick.object()
        const instanceId = pick.instance()
        console.log('[SelectTool] selected object:', objectId, 'instance:', instanceId, 'face:', pick.face())
        this.onSelect(objectId, instanceId)
      } finally {
        pick.free()
      }
      return
    }

    // No face hit — try a free-standing sketch edge before giving up.
    const edgePick = this.wasmScene.pick_sketch_edge(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (edgePick !== undefined) {
      try {
        const sketchId = edgePick.sketch()
        const edgeId = edgePick.edge()
        console.log('[SelectTool] selected sketch edge:', sketchId, edgeId)
        this.onSelect(null, undefined, sketchId, edgeId)
      } finally {
        edgePick.free()
      }
      return
    }

    // No edge hit either — try the interior of a closed region (clicking
    // inside a drawn rectangle/circle selects its owning sketch).
    const regionPick = this.wasmScene.pick_sketch_region(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (regionPick !== undefined) {
      try {
        const regionSketchId = regionPick.sketch()
        const regionId = regionPick.region()
        console.log('[SelectTool] selected sketch (interior pick):', regionSketchId, regionId)
        this.onSelect(null, undefined, regionSketchId, undefined, regionId)
      } finally {
        regionPick.free()
      }
      return
    }

    console.log('[SelectTool] click hit no object or sketch — clearing selection')
    this.onSelect(null)
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
    }
  }

  cancel(): void {
    this.lastSnap = null
  }
}
