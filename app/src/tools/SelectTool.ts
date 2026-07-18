/**
 * SelectTool — the Select tool's pointer/keyboard handling.
 *
 * Selection RESOLUTION (what is under the cursor) is not done here: the click
 * forwards the occlusion-aware inference snap the hover cue already resolved,
 * plus the ray, to `onSelect`, which runs the shared `resolveSelectableRef`
 * (provenance + editing-context scoping + the far-plane depth bound) — the
 * exact same resolver the drag-move arm uses, so click, drag, and hover agree
 * on what is under the cursor by construction. This tool only holds the hover
 * snap (for the status bar) and handles Escape.
 */

import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'

/** Select a click: the host resolves the snap+ray into a selectable node
 * (through `resolveSelectableRef`) and updates the selection. `null` snap is a
 * genuine empty click (which may still resolve a solid under the ray). */
export type OnSelect = (snap: Snap | null, ray: Ray) => void

export class SelectTool implements Tool {
  readonly name = 'Select'

  /** Live status-bar guidance (see Tool.statusHint). */
  statusHint(): string {
    return 'Click to select — drag an object to move it, drag empty space for a marquee, double-click to enter a group or component.'
  }

  /** Last snap seen on hover (for status display) */
  lastSnap: Snap | null = null

  private onSelect: OnSelect

  constructor(_wasmScene: WasmScene, onSelect: OnSelect) {
    this.onSelect = onSelect
  }

  onPointerMove(snap: Snap | null, _ray: Ray): void {
    this.lastSnap = snap
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    // Hand the resolved snap and ray to the host, which runs the shared
    // resolver — identical to the drag-move arm's resolution.
    this.onSelect(snap, ray)
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
