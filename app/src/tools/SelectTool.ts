/**
 * SelectTool — M1 minimal implementation.
 *
 * Hover-highlight via snap() provenance; click uses pick_face() for reliable
 * object-face detection (snap prefers vertices/edges and can miss face intent).
 * Fires onSelect(objectId) on hit, onSelect(null) on miss.  Real selection
 * model (move, multi-select, delete) is M2.
 */

import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'

export type OnSelect = (objectId: bigint | null) => void

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
        console.log('[SelectTool] selected object:', objectId, 'face:', pick.face())
        this.onSelect(objectId)
      } finally {
        pick.free()
      }
    } else {
      console.log('[SelectTool] click hit no object — clearing selection')
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
