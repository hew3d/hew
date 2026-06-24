/**
 * SelectTool — M1 minimal implementation, extended for sketch selection.
 *
 * Hover-highlight via snap() provenance; click uses pick_face() for reliable
 * object-face detection (snap prefers vertices/edges and can miss face intent).
 * On a pick_face miss, falls back to pick_sketch() so a free-standing
 * (not-yet-extruded) sketch's edges are selectable too — whole-sketch
 * granularity, fired as the third `sketchId` arg since a sketch has no
 * object/instance id of its own.
 * Fires onSelect(objectId) on a face hit, onSelect(null, undefined, sketchId)
 * on a sketch hit, onSelect(null) on a total miss.
 */

import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'

export type OnSelect = (
  objectId: bigint | null,
  instanceId?: bigint,
  sketchId?: bigint,
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
    const sketchId = this.wasmScene.pick_sketch(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (sketchId !== undefined) {
      console.log('[SelectTool] selected sketch:', sketchId)
      this.onSelect(null, undefined, sketchId)
    } else {
      console.log('[SelectTool] click hit no object or sketch — clearing selection')
      this.onSelect(null)
    }
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
