/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * filletLabels — the inline value beside each gizmo marker of the Fillet
 * tool: the radius by the handle, a stop's distance by its diamond, a radius
 * point's radius by its sphere. In-scene `TextBillboard`s (screen-constant,
 * crisp through their DPR canvas) rather than DOM chips, so they move with
 * the model on every frame without a React pass. Without a `document` (the
 * node test runner) nothing is drawn and every call is a no-op.
 */

import * as THREE from 'three'
import type { V3 } from './filletMath'
import { TextBillboard } from '../viewport/TextBillboard'

export interface FilletLabel {
  key: string
  /** The marker the label sits beside. */
  pos: V3
  /** Which way (unit) the label is pushed off the marker. */
  dir: V3
  text: string
  active: boolean
}

const LABEL_PX = 13
/** How far the label's centre sits from the marker, in screen pixels. */
const LABEL_OFFSET_PX = 26
const LABEL_COLOR = 0xffffff
const ACTIVE_LABEL_COLOR = 0xffb347

export class FilletLabels {
  readonly group = new THREE.Group()
  private items: FilletLabel[] = []
  private boards = new Map<string, TextBillboard>()
  private readonly enabled = typeof document !== 'undefined'

  constructor() {
    this.group.name = 'FilletLabels'
  }

  /** Replaces the labels; billboards are reused by key so the text only
   *  re-rasterizes when it changes. */
  set(items: FilletLabel[]): void {
    this.items = items
    if (!this.enabled) return
    const keep = new Set(items.map((i) => i.key))
    for (const [key, board] of this.boards) {
      if (keep.has(key)) continue
      this.group.remove(board.mesh)
      board.dispose()
      this.boards.delete(key)
    }
    for (const item of items) {
      if (this.boards.has(item.key)) continue
      const board = new TextBillboard(LABEL_PX, 1001)
      board.mesh.material.depthTest = false
      board.mesh.name = `FilletLabel:${item.key}`
      this.group.add(board.mesh)
      this.boards.set(item.key, board)
    }
  }

  clear(): void {
    this.set([])
  }

  /** The current labels (tests read the text here). */
  get all(): readonly FilletLabel[] {
    return this.items
  }

  /** Per frame, from the tool's `updateGripScale`: place each label off its
   *  marker by a screen-constant offset and size its quad. */
  update(camera: THREE.Camera, worldPerPixel: (dist: number) => number): void {
    if (!this.enabled) return
    this.group.updateWorldMatrix(true, false)
    const dpr = typeof window === 'undefined' ? 1 : window.devicePixelRatio || 1
    const world = new THREE.Vector3()
    const tip = new THREE.Vector3()
    const right = new THREE.Vector3()
    const up = new THREE.Vector3()
    camera.matrixWorld.extractBasis(right, up, new THREE.Vector3())
    for (const item of this.items) {
      const board = this.boards.get(item.key)
      if (board === undefined) continue
      world.set(item.pos[0], item.pos[1], item.pos[2]).applyMatrix4(this.group.matrixWorld)
      const dist = camera.position.distanceTo(world)
      const wpp = worldPerPixel(dist)
      // The push direction as it reads on screen: the axis's projection,
      // or straight up when the axis points at the eye. The offset is then
      // a fixed number of pixels, whatever the view.
      tip.set(item.pos[0] + item.dir[0], item.pos[1] + item.dir[1], item.pos[2] + item.dir[2]).applyMatrix4(this.group.matrixWorld)
      const dx = tip.dot(right) - world.dot(right)
      const dy = tip.dot(up) - world.dot(up)
      const len = Math.hypot(dx, dy)
      const sx = len > 1e-6 ? dx / len : 0
      const sy = len > 1e-6 ? dy / len : 1
      const off = LABEL_OFFSET_PX * wpp
      board.mesh.position.set(
        item.pos[0] + (right.x * sx + up.x * sy) * off,
        item.pos[1] + (right.y * sx + up.y * sy) * off,
        item.pos[2] + (right.z * sx + up.z * sy) * off,
      )
      board.setContent(item.text, item.active ? ACTIVE_LABEL_COLOR : LABEL_COLOR, dpr, dist)
      board.update(camera, viewportHeightFor(camera, dist, wpp))
    }
  }

  dispose(): void {
    this.clear()
  }
}

/** The viewport height that makes `TextBillboard.update`'s screen-constant
 *  formula agree with `worldPerPixel` at `dist` (the tool is handed the
 *  latter, not the former). */
export function viewportHeightFor(camera: THREE.Camera, dist: number, worldPerPixel: number): number {
  if (!(worldPerPixel > 0)) return 0
  if (camera instanceof THREE.PerspectiveCamera) {
    return (2 * dist * Math.tan((camera.fov * Math.PI) / 360)) / worldPerPixel
  }
  if (camera instanceof THREE.OrthographicCamera) {
    return (camera.top - camera.bottom) / camera.zoom / worldPerPixel
  }
  return 0
}
