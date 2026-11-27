/**
 * previewLines — one slot for a fat-line overlay in the shared preview group.
 *
 * Revolve and Deform each draw a few cue lines (the hovered profile, the
 * axis, a hovered box face). Each cue owns a `LineSlot`: `set` replaces what
 * it shows, `clear` removes it, and both dispose through `fatLine.ts` so the
 * material leaves its resolution registry.
 */

import type * as THREE from 'three'
import { makeFatSegments, disposeFatSegments, PREVIEW_LINE_STYLE } from '../viewport/fatLine'

export class LineSlot {
  private line: ReturnType<typeof makeFatSegments> | null = null
  private key: string | null = null
  private color = 0

  constructor(private readonly preview: THREE.Group, private readonly renderOrder = 995) {}

  /** The identity of what is showing (null when empty). */
  get shownKey(): string | null {
    return this.key
  }

  /** Show `points` (flat segment endpoint pairs) in `color`. A repeat call
   *  with the same non-null `key` and `color` keeps the existing line. */
  set(points: number[], color: number, key: string | null = null): void {
    if (key !== null && key === this.key && this.line !== null && this.color === color) return
    this.clear()
    if (points.length === 0) return
    this.line = makeFatSegments(new Float32Array(points), {
      color,
      widthPx: PREVIEW_LINE_STYLE.widthPx,
      depthTest: false,
      renderOrder: this.renderOrder,
    })
    this.color = color
    this.key = key
    this.preview.add(this.line)
  }

  clear(): void {
    if (this.line !== null) {
      this.preview.remove(this.line)
      disposeFatSegments(this.line)
    }
    this.line = null
    this.key = null
  }
}
