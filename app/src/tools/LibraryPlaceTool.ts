/**
 * LibraryPlaceTool — cursor placement of a library item (the Library
 * design's insert-and-place flow).
 *
 * Armed by the Library browser (or the command palette's straight-to-cursor
 * insert) with the item's bytes and a pre-built ghost mesh; the ghost then
 * rides the resolved inference snap — endpoints, midpoints, edges, faces,
 * the ground — exactly like Move's rubber-band, with the item's own origin
 * (its saved drawing-axes origin) pinned to the snap point and a small
 * origin crosshair marking it. A single click commits ONE
 * `Scene::insert_item` call: lossless kernel graft, one undo step,
 * provenance-stamped so re-inserting the same item reuses its definition.
 *
 * Unlike TextPlaceTool (which stays armed to stamp repeated text), this
 * tool is one-shot: a library insert duplicates real content, and repeated
 * placement is Move+Alt-copy's job — so a successful placement reports
 * `onPlaced` and the app returns to Select with the new roots selected
 * (the Zoom Window one-shot convention). Esc cancels via `onDone(false)`.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { V3 } from '../viewport/geoHelpers'
import { rayPlaneIntersect } from '../viewport/geoHelpers'
import { groundDrawPlane } from './drawPlane'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { clearPreview } from './transformPreview'

/** What one committed insert created — the app selects these roots. */
export interface LibraryInsertResult {
  /** Parallel arrays: kind 0 = object, 1 = group, 2 = instance. */
  rootKinds: number[]
  rootIds: bigint[]
  definitionsReused: number
  worldSketchesSkipped: number
  annotationsSkipped: number
}

/** Everything the browser hands the tool. The ghost group's ownership
 *  transfers to the tool (disposed via `clearPreview` on cancel/commit). */
export interface LibraryPlacement {
  bytes: Uint8Array
  /** `hew.library` provenance; both or neither. */
  sourceId: string | null
  contentHash: string | null
  displayName: string
  /** Ghost meshes in ITEM coordinates (origin = the item's own origin). */
  ghost: THREE.Group
  /** Item-space bounds (origin-marker scale + status). */
  bboxMin: V3
  bboxMax: V3
}

export type OnPlaced = (result: LibraryInsertResult) => void
/** Fired exactly once when the tool retires. `placed` is true after a
 *  successful placement (after `onPlaced`). `byUser` distinguishes an
 *  explicit finish (a commit, or the user's own Esc) — where the caller
 *  should restore the Select tool — from a ToolController takeover (another
 *  tool is already replacing this one; restoring Select there would
 *  clobber the incoming tool: `ToolController.setTool` cancels the
 *  outgoing tool BEFORE installing the new one). */
export type OnDone = (placed: boolean, byUser: boolean) => void
export type OnToast = (message: string, code?: string) => void

const GHOST_ORIGIN_COLOR = 0xf5d76a

export class LibraryPlaceTool implements Tool {
  readonly name = 'LibraryPlace'

  private wasmScene: WasmScene
  private preview: THREE.Group
  private placement: LibraryPlacement
  private onPlaced: OnPlaced
  private onDone: OnDone
  private onToast: OnToast

  private ghostRoot: THREE.Group | null = null
  private lastPoint: V3 | null = null
  private finished = false

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    placement: LibraryPlacement,
    onPlaced: OnPlaced,
    onDone: OnDone,
    onToast: OnToast,
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.placement = placement
    this.onPlaced = onPlaced
    this.onDone = onDone
    this.onToast = onToast
  }

  statusHint(): string {
    return `Click to place "${this.placement.displayName}" — full inference snapping, exactly like Move. Esc cancels.`
  }

  /** An armed placement is a live gesture: Escape must cancel IT — not pop
   *  an open edit context or close a session frame underneath it — until
   *  it commits or is cancelled (the Viewport Escape ladder's
   *  `toolHasArmedGesture` contract; adversarial review S14, matching the
   *  pill's own "Esc cancel" promise). */
  hasArmedGesture(): boolean {
    return !this.finished
  }

  /** Lazily mount the ghost (item mesh + origin crosshair) into the shared
   *  preview group the first time the cursor resolves a point. */
  private _ensureGhost(): THREE.Group {
    if (this.ghostRoot !== null) return this.ghostRoot
    const root = new THREE.Group()
    root.add(this.placement.ghost)
    root.add(this._originMarker())
    this.preview.add(root)
    this.ghostRoot = root
    return root
  }

  /** Small crosshair at the item origin — the design's yellow origin cue
   *  ("insertion point = the item's saved axes origin"). World-sized from
   *  the item's own bounds so it reads at any item scale. */
  private _originMarker(): THREE.Object3D {
    const [ax, ay, az] = this.placement.bboxMin
    const [bx, by, bz] = this.placement.bboxMax
    const diag = Math.hypot(bx - ax, by - ay, bz - az)
    const r = Math.min(Math.max(diag * 0.06, 0.015), 0.5)
    const pts = [
      -r, 0, 0, r, 0, 0,
      0, -r, 0, 0, r, 0,
      0, 0, -r, 0, 0, r,
    ]
    const geom = new THREE.BufferGeometry()
    geom.setAttribute('position', new THREE.Float32BufferAttribute(pts, 3))
    const mat = new THREE.LineBasicMaterial({
      color: GHOST_ORIGIN_COLOR,
      transparent: true,
      opacity: 0.95,
      depthTest: false,
    })
    const lines = new THREE.LineSegments(geom, mat)
    lines.renderOrder = 3
    return lines
  }

  /** The placement point this event resolves: the inference snap when one
   *  is live (Move parity), else the ground-plane hit, else the last known
   *  point (horizon rays keep the ghost where it was rather than vanishing
   *  it). */
  private _resolvePoint(snap: Snap | null, ray: Ray): V3 | null {
    if (snap !== null) return [snap.x, snap.y, snap.z]
    const ground = groundDrawPlane()
    const hit = rayPlaneIntersect(ray.origin, ray.direction, ground.origin, ground.normal)
    if (hit !== null) return hit
    return this.lastPoint
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    const point = this._resolvePoint(snap, ray)
    if (point === null) return
    this.lastPoint = point
    const root = this._ensureGhost()
    root.position.set(point[0], point[1], point[2])
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (this.finished) return
    const point = this._resolvePoint(snap, ray)
    if (point === null) return
    const affine = new Float64Array([
      1, 0, 0, point[0],
      0, 1, 0, point[1],
      0, 0, 1, point[2],
    ])
    let raw: unknown
    try {
      raw = this.wasmScene.insert_item(
        this.placement.bytes,
        affine,
        this.placement.sourceId ?? undefined,
        this.placement.contentHash ?? undefined,
      )
    } catch (err) {
      // A refusal (open component session, unloadable item) leaves the
      // document untouched; report it and stay armed so the user can Esc
      // or try another spot deliberately.
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
      return
    }
    const result = parseInsertResult(raw)
    this._teardown()
    this.finished = true
    this.onPlaced(result)
    this.onDone(true, true)
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.escPressed = true
      this.cancel()
    }
  }

  private escPressed = false

  /** Also invoked by the ToolController when another tool takes over —
   *  `finished` keeps the post-placement `setTool(Select)` switch from
   *  double-reporting, and `escPressed` tells a user Esc from a takeover
   *  (see [`OnDone`]). */
  cancel(): void {
    this._teardown()
    if (this.finished) return
    this.finished = true
    this.onDone(false, this.escPressed)
  }

  private _teardown(): void {
    if (this.ghostRoot !== null) {
      clearPreview(this.preview)
      this.ghostRoot = null
    }
  }
}

/** Parse `Scene::insert_item`'s plain-JS report (root ids cross as decimal
 *  strings — the harness convention). Exported for tests. */
export function parseInsertResult(raw: unknown): LibraryInsertResult {
  const obj = (raw ?? {}) as Record<string, unknown>
  const kinds = Array.isArray(obj.rootKinds) ? (obj.rootKinds as unknown[]) : []
  const ids = Array.isArray(obj.rootIds) ? (obj.rootIds as unknown[]) : []
  const num = (v: unknown): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0)
  return {
    rootKinds: kinds.map((k) => num(k)),
    rootIds: ids.map((v) => BigInt(typeof v === 'string' ? v : 0)),
    definitionsReused: num(obj.definitionsReused),
    worldSketchesSkipped: num(obj.worldSketchesSkipped),
    annotationsSkipped: num(obj.annotationsSkipped),
  }
}
