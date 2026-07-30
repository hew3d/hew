/**
 * TextTool — SketchUp-style leader text (docs/design/dimensions-text.md
 * "Tools & UX"). Kernel entity: `Annotation::LeaderText`
 * (crates/kernel/src/annotation.rs); rendered by
 * `SceneRenderer.refreshAnnotations()`.
 *
 * Gesture:
 *   1. Click a face/edge/point (any inference snap) to anchor the leader.
 *   2. Drag out the leader's offset (a live rubber-band preview). The
 *      offset point comes from intersecting the cursor ray with a
 *      CAMERA-FACING plane through the anchor (normal = the camera's view
 *      direction) — dimensions-playtest2.md §3: the text lands where it
 *      looks like it is under the cursor, at the anchor's own depth,
 *      instead of wherever the general snap system's cursor point happens
 *      to resolve (which, before this fix, commonly fell back to the
 *      ground plane in free space — "the leader strongly wants to jump
 *      down to the ground"). Falls back to the plain resolved cursor point
 *      when no camera has been registered yet (`updateCamera` never
 *      called) or the ray is parallel to that plane.
 *   3. Second click finalizes the leader's placement and hands off to the
 *      in-viewport text editor (`AnnotationEditor`, the `InferenceTooltip`
 *      DOM-positioning pattern) via `onPlaceLeader` — this tool does not own
 *      keystroke entry itself; `Viewport.tsx` opens the editor at the
 *      returned screen position, and only calls `add_leader_text` once the
 *      user actually commits non-empty text (Enter/blur), never on Esc.
 *
 * Esc while dragging cancels the gesture with no annotation created.
 */
import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { V3 } from '../viewport/geoHelpers'
import { rayPlaneIntersect } from '../viewport/geoHelpers'

/** Rubber-band preview color — matches `DimensionTool`'s / the draw tools'
 * gesture-preview blue (`fatLine.ts`'s `PREVIEW_LINE_STYLE`). */
const PREVIEW_COLOR = 0x4d90ff

export interface PlacedLeader {
  anchorNode: { kind: number; id: bigint } | null
  anchorPoint: V3
  offset: V3
}

export type OnPlaceLeader = (leader: PlacedLeader) => void

function anchorNodeFromSnap(snap: Snap): { kind: number; id: bigint } | null {
  if (snap.instance !== undefined) return { kind: 2, id: snap.instance }
  if (snap.object !== undefined) return { kind: 0, id: snap.object }
  return null
}

function sub(a: V3, b: V3): V3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

type Stage =
  | { kind: 'idle' }
  | { kind: 'dragging'; anchorNode: { kind: number; id: bigint } | null; anchorPoint: V3; offset: V3 }

export class TextTool implements Tool {
  readonly name = 'Text'

  statusHint(): string {
    return this.stage.kind === 'dragging'
      ? 'Drag out the leader, then click to place and type.'
      : 'Click a face, edge, or point to anchor the leader text.'
  }

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private previewLine: THREE.LineSegments | null = null
  private onPlace: OnPlaceLeader
  /** The current camera's view (forward) direction, refreshed once per
   * render frame via `updateCamera` — mirrors `DimensionTool`'s own field
   * and its "no camera registered yet -> old behavior" fallback (see that
   * tool's doc comment). */
  private _viewDir: V3 | null = null

  constructor(previewGroup: THREE.Group, onPlace: OnPlaceLeader) {
    this.preview = previewGroup
    this.onPlace = onPlace
  }

  /** Live camera feed, called once per render frame while this tool is
   * active (feature-detected by `Viewport.tsx`, mirroring
   * `DimensionTool.updateCamera`/`ScaleTool.updateGripScale`). */
  updateCamera(camera: THREE.Camera): void {
    const dir = new THREE.Vector3()
    camera.getWorldDirection(dir)
    this._viewDir = [dir.x, dir.y, dir.z]
  }

  capturingInput(): boolean {
    return this.stage.kind !== 'idle'
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (snap === null || this.stage.kind !== 'dragging') return
    let cursor: V3 = [snap.x, snap.y, snap.z]
    if (this._viewDir !== null) {
      const hit = rayPlaneIntersect(ray.origin, ray.direction, this.stage.anchorPoint, this._viewDir)
      if (hit !== null) cursor = hit
    }
    this.stage.offset = sub(cursor, this.stage.anchorPoint)
    this._updatePreview()
  }

  onPointerDown(snap: Snap | null, _ray: Ray): void {
    if (snap === null) return
    const point: V3 = [snap.x, snap.y, snap.z]

    if (this.stage.kind === 'idle') {
      this.stage = { kind: 'dragging', anchorNode: anchorNodeFromSnap(snap), anchorPoint: point, offset: [0, 0, 0] }
      this._updatePreview()
      return
    }

    // Second click: finalize the leader and hand off to the text editor.
    const { anchorNode, anchorPoint, offset } = this.stage
    this.cancel()
    this.onPlace({ anchorNode, anchorPoint, offset })
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') this.cancel()
  }

  cancel(): void {
    this.stage = { kind: 'idle' }
    this._clearPreview()
  }

  private _updatePreview(): void {
    this._clearPreview()
    if (this.stage.kind !== 'dragging') return
    const { anchorPoint, offset } = this.stage
    const end: V3 = [anchorPoint[0] + offset[0], anchorPoint[1] + offset[1], anchorPoint[2] + offset[2]]
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array([...anchorPoint, ...end]), 3))
    const mat = new THREE.LineBasicMaterial({ color: PREVIEW_COLOR, depthTest: false })
    this.previewLine = new THREE.LineSegments(geo, mat)
    this.preview.add(this.previewLine)
  }

  private _clearPreview(): void {
    if (this.previewLine === null) return
    this.previewLine.geometry.dispose()
    if (this.previewLine.material instanceof THREE.Material) this.previewLine.material.dispose()
    this.preview.remove(this.previewLine)
    this.previewLine = null
  }
}
