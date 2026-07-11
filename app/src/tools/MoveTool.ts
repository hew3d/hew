/**
 * MoveTool — SketchUp-style two-click object translate, with axis-lock and
 * numeric VCB entry.
 *
 * Gesture:
 *   1. First click  : set the base point (snapped).
 *   2. Move         : rubber-band preview by moving a three.js clone of the
 *                     selection's meshes (no kernel calls mid-drag).
 *   3. Second click : commit translation = dest − base (one node → the
 *                     per-kind transform method; a multi-selection → one
 *                     transform_selection call, one undo step).
 *   4. Esc          : cancel.
 *
 * Axis lock (while in 'base' stage):
 *   ArrowRight → lock X (red guide line)
 *   ArrowLeft  → lock Y (green guide line)
 *   ArrowUp    → lock Z (blue guide line)
 *   ArrowDown or same arrow again → clear lock
 *   Shift (held) → lock to the axis the drag is currently moving along;
 *                  releasing Shift clears that lock (an arrow lock overrides it).
 *
 * Copy: holding Option/Alt during the move turns the commit into a
 * duplicate — the dragged result is a deep clone (`duplicate_node`) placed at the
 * offset, and the copy becomes the new selection so further Alt-drags chain
 * copies. Object/group copies are independent baked geometry; an instance copy
 * shares its definition at the offset pose.
 *
 * Numeric VCB:
 *   Type digits / . / - while in 'base' stage → builds a buffer shown as the
 *   "Length" measurement.  Press Enter to commit that exact distance along
 *   the current direction (locked axis or cursor direction).
 *
 * If nothing is selected, the tool shows a hint toast and remains idle.
 * On commit: one kernel transform call, then handleSceneRefresh + onDocumentChanged.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { translationAffine, affineToFloat64 } from './transformMath'
import { parseKernelErrorCode, kernelErrorMessage } from '../viewport/geoHelpers'
import { clearPreview } from './transformPreview'
import { commitSelectionTransform, buildSelectionPreview } from './transformSelection'
import { arrowToAxis, editLengthBuffer, isLengthInputKey, pointAlong } from './moveInput'
import type { NodeRef } from '../panels/treeModel'
import { nodeKindToNumber, nodeRefFromJs } from '../panels/treeModel'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'

export type OnMoveCommit = (nodes: NodeRef[]) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/** Axis guide colours matching the world axis convention */
const AXIS_COLOR: Record<0 | 1 | 2, number> = {
  0: 0xff0000,   // X — red
  1: 0x00aa00,   // Y — green
  2: 0x0000ff,   // Z — blue
}

/** Unit direction for each locked axis */
const AXIS_DIR: Record<0 | 1 | 2, [number, number, number]> = {
  0: [1, 0, 0],
  1: [0, 1, 0],
  2: [0, 0, 1],
}

/** Half-extent of the axis guide line drawn through the base point */
const GUIDE_HALF_LENGTH = 50

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'base'
      nodes: NodeRef[]
      base: [number, number, number]
      previewMesh: THREE.Object3D | null
      /** Last snapped/computed destination (updated every pointer move). */
      dest: [number, number, number]
    }

export class MoveTool implements Tool {
  readonly name = 'Move'

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnMoveCommit
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** Current axis lock: 0=X, 1=Y, 2=Z, null=free */
  private lockAxis: 0 | 1 | 2 | null = null
  /** True when the *current* axis lock was set by holding Shift (vs. an arrow). */
  private shiftAxisLock: boolean = false
  /** True when Option/Alt is held — the move commits as a copy. */
  private copyMode: boolean = false
  /** VCB buffer — raw string being typed by the user */
  private typed: string = ''

  /** THREE.js LineSegments for the axis guide drawn in the preview group */
  private guideLine: THREE.LineSegments | null = null

  /** The full selection at tool activation (set by Viewport from selectedIds). */
  private selection: NodeRef[] = []
  /** THREE.js object group from the SceneRenderer (read-only reference for cloning). */
  private objectsGroup: THREE.Group | null = null
  /** THREE.js instances group from the SceneRenderer (read-only reference for cloning). */
  private instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    objectsGroup: THREE.Group | null,
    selection: NodeRef[],
    onCommit: OnMoveCommit,
    onToast: OnToast,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
    instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null = null,
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.objectsGroup = objectsGroup
    this.selection = selection
    this.onCommit = onCommit
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
    this.instanceGroupGetter = instanceGroupGetter
  }

  // ── Optional Tool interface extensions ─────────────────────────────────────

  snapConstraint(): { anchor: [number, number, number]; lockAxis?: 0 | 1 | 2 } | null {
    if (this.stage.kind !== 'base') return null
    const result: { anchor: [number, number, number]; lockAxis?: 0 | 1 | 2 } = {
      anchor: this.stage.base,
    }
    if (this.lockAxis !== null) {
      result.lockAxis = this.lockAxis
    }
    return result
  }

  capturingInput(): boolean {
    return this.stage.kind === 'base'
  }

  /**
   * Set by the Viewport from the pointer/key Alt modifier: while true, the
   * next commit duplicates instead of moving. Live-tracked so releasing Alt
   * mid-drag drops back to a plain move.
   */
  setCopyMode(on: boolean): void {
    if (this.copyMode === on) return
    this.copyMode = on
    if (this.stage.kind === 'base') {
      this._reportMeasurement(this.stage.base, this.stage.dest)
    }
  }

  /**
   * Shift-held axis lock: pressing Shift while the drag is already
   * moving along a dominant axis locks to it; releasing Shift clears that lock.
   * An explicit arrow lock takes precedence and is left alone.
   */
  setShiftHeld(held: boolean): void {
    if (this.stage.kind !== 'base') return
    if (held) {
      if (this.lockAxis !== null) return
      const axis = this._dominantAxis()
      if (axis === null) return
      this.lockAxis = axis
      this.shiftAxisLock = true
      this._updateGuideLine()
      this._reportMeasurement(this.stage.base, this.stage.dest)
    } else if (this.shiftAxisLock) {
      this.lockAxis = null
      this.shiftAxisLock = false
      this._updateGuideLine()
      this._reportMeasurement(this.stage.base, this.stage.dest)
    }
  }

  /** The world axis the current base→dest drag is most aligned with, or null. */
  private _dominantAxis(): 0 | 1 | 2 | null {
    if (this.stage.kind !== 'base') return null
    const { base, dest } = this.stage
    const d = [dest[0] - base[0], dest[1] - base[1], dest[2] - base[2]]
    const ax = Math.abs(d[0]), ay = Math.abs(d[1]), az = Math.abs(d[2])
    const max = Math.max(ax, ay, az)
    if (max < 1e-9) return null
    if (max === ax) return 0
    if (max === ay) return 1
    return 2
  }

  // ── Tool interface ──────────────────────────────────────────────────────────

  onPointerMove(snap: Snap | null, _ray: Ray): void {
    if (this.stage.kind !== 'base' || snap === null) return
    const { base, previewMesh } = this.stage

    const dest: [number, number, number] = [snap.x, snap.y, snap.z]
    this.stage.dest = dest

    if (previewMesh !== null) {
      const dx = snap.x - base[0]
      const dy = snap.y - base[1]
      const dz = snap.z - base[2]
      previewMesh.position.set(dx, dy, dz)
    }

    this._reportMeasurement(base, dest)
  }

  onPointerDown(snap: Snap | null, _ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'idle') {
      const nodes = this.selection
      if (nodes.length === 0) {
        this.onToast('Select an object first, then use Move')
        return
      }

      const previewMesh = this._buildPreview(nodes)
      const base: [number, number, number] = [snap.x, snap.y, snap.z]
      if (previewMesh !== null) {
        previewMesh.position.set(0, 0, 0)
        this.preview.add(previewMesh)
      }

      this.stage = { kind: 'base', nodes, base, previewMesh, dest: [...base] }
      this._updateGuideLine()
    } else if (this.stage.kind === 'base') {
      const { nodes, base } = this.stage
      const tx = snap.x - base[0]
      const ty = snap.y - base[1]
      const tz = snap.z - base[2]

      // Degenerate: no movement
      if (Math.abs(tx) < 1e-9 && Math.abs(ty) < 1e-9 && Math.abs(tz) < 1e-9) {
        this._resetToIdle()
        return
      }

      this._commitAndReset(nodes, tx, ty, tz)
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
      return
    }

    if (this.stage.kind !== 'base') return

    // Keep copy-mode in sync with the Alt modifier on this key event (the VCB
    // path commits from here, so it must see whether Alt is held).
    this.copyMode = ev.altKey

    // ── Axis lock via arrow keys ──
    if (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft' || ev.key === 'ArrowUp' || ev.key === 'ArrowDown') {
      const requested = arrowToAxis(ev.key)
      if (requested === null || requested === this.lockAxis) {
        // ArrowDown, or pressing same arrow again → clear lock
        this.lockAxis = null
      } else {
        this.lockAxis = requested
      }
      // An explicit arrow lock supersedes any Shift-held lock.
      this.shiftAxisLock = false
      this._updateGuideLine()
      this._reportMeasurement(this.stage.base, this.stage.dest)
      return
    }

    // ── Numeric VCB ──
    if (ev.key === 'Enter') {
      const meters = parseLengthToMeters(this.typed)
      if (meters !== null) {
        this._commitFromTyped(meters)
      }
      return
    }

    // Feed length-input keys (digits, dot, minus, feet/inch/fraction marks,
    // explicit unit-suffix letters, Backspace) into the buffer.
    if (isLengthInputKey(ev.key)) {
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
      // Report the typed buffer as the measurement readout, tagged with the
      // current display unit so the user knows what they're typing in.
      this.onMeasurementCb(this._decorate(this._typedReadout()))
    }
  }

  /** The typed-buffer readout, suffixed for metric formats (imperial tokens
   * like `'`/`"` are already visible in the buffer itself). */
  private _typedReadout(): string {
    return typedReadout(this.typed)
  }

  cancel(): void {
    this._resetToIdle()
  }

  // ── Private helpers ─────────────────────────────────────────────────────────

  /** Commit the move via translationAffine, then reset to idle. */
  private _commitAndReset(nodes: NodeRef[], tx: number, ty: number, tz: number): void {
    this._resetToIdle()
    this._commit(nodes, tx, ty, tz)
  }

  /**
   * Commit from the typed VCB buffer.  The direction is:
   *   - The locked axis (signed by which side of the base the cursor is on),
   *   - or the vector from base → last dest if no axis is locked.
   */
  private _commitFromTyped(dist: number): void {
    if (this.stage.kind !== 'base') return
    const { nodes, base, dest } = this.stage

    let dir: [number, number, number]
    if (this.lockAxis !== null) {
      // Signed direction: match the cursor side so typing "2" means "2 in the
      // direction the cursor is pointing".
      const axisDir = AXIS_DIR[this.lockAxis]
      const dotSign = (dest[0] - base[0]) * axisDir[0]
                    + (dest[1] - base[1]) * axisDir[1]
                    + (dest[2] - base[2]) * axisDir[2]
      const sign = dotSign < 0 ? -1 : 1
      dir = [axisDir[0] * sign, axisDir[1] * sign, axisDir[2] * sign]
    } else {
      dir = [dest[0] - base[0], dest[1] - base[1], dest[2] - base[2]]
    }

    const endpoint = pointAlong(base, dir, dist)
    const tx = endpoint[0] - base[0]
    const ty = endpoint[1] - base[1]
    const tz = endpoint[2] - base[2]

    if (Math.abs(tx) < 1e-9 && Math.abs(ty) < 1e-9 && Math.abs(tz) < 1e-9) {
      this._resetToIdle()
      return
    }

    this._commitAndReset(nodes, tx, ty, tz)
  }

  private _resetToIdle(): void {
    this.stage = { kind: 'idle' }
    this.lockAxis = null
    this.shiftAxisLock = false
    this.typed = ''
    clearPreview(this.preview)
    this.guideLine = null   // clearPreview removed it
    this.onMeasurementCb('')
  }

  private _buildPreview(nodes: NodeRef[]): THREE.Object3D | null {
    return buildSelectionPreview(this.wasmScene, this.objectsGroup, this.instanceGroupGetter, nodes)
  }

  private _commit(nodes: NodeRef[], tx: number, ty: number, tz: number): void {
    try {
      const affineF64 = affineToFloat64(translationAffine(tx, ty, tz))
      if (
        this.copyMode &&
        nodes.some((n) => n.kind === 'object' || n.kind === 'group' || n.kind === 'instance')
      ) {
        // Option/Alt: duplicate at the offset instead of moving. Each copy is
        // the same kind as its source; the copies become the selection so a
        // follow-up Alt-drag chains off them. Sketches have no
        // `duplicate_node` support — they fall through to a plain move, as
        // they always have in single-selection copy mode.
        //
        // Per-node kernel calls mean a mid-loop failure leaves the earlier
        // copies committed in the document — the finally block refreshes and
        // reselects whatever landed, so the viewport never renders a scene
        // that diverges from the kernel (the error still surfaces as a toast).
        const committed: NodeRef[] = []
        try {
          for (const node of nodes) {
            if (node.kind === 'sketch-edge' || node.kind === 'sketch-curve') {
              continue // lines/curves are not movable/copyable (v1)
            }
            if (node.kind === 'sketch-island' && node.sketch !== undefined) {
              // Islands MOVE under copy-drag like whole sketches do — sketch
              // geometry has no duplicate path yet.
              this.wasmScene.transform_sketch_island(node.sketch, node.id, affineF64)
              committed.push(node)
              continue
            }
            if (node.kind === 'sketch') {
              this.wasmScene.transform_sketch(node.id, affineF64)
              committed.push(node)
            } else {
              const created = this.wasmScene.duplicate_node(
                nodeKindToNumber(node.kind), node.id, affineF64,
              )
              committed.push(nodeRefFromJs(created))
            }
          }
        } finally {
          if (committed.length > 0) {
            this.selection = committed
            this.onCommit(committed)
          }
        }
      } else {
        commitSelectionTransform(this.wasmScene, nodes, affineF64)
        this.onCommit(nodes)
      }
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
    }
  }

  /**
   * Report the live distance measurement.
   * When the user has typed something, that buffer is the readout; otherwise
   * compute the signed distance along the locked axis (or total distance).
   */
  private _reportMeasurement(base: [number, number, number], dest: [number, number, number]): void {
    if (this.typed !== '') {
      this.onMeasurementCb(this._decorate(this._typedReadout()))
      return
    }

    let dist: number
    if (this.lockAxis !== null) {
      const axisDir = AXIS_DIR[this.lockAxis]
      dist = (dest[0] - base[0]) * axisDir[0]
           + (dest[1] - base[1]) * axisDir[1]
           + (dest[2] - base[2]) * axisDir[2]
    } else {
      const dx = dest[0] - base[0]
      const dy = dest[1] - base[1]
      const dz = dest[2] - base[2]
      dist = Math.sqrt(dx * dx + dy * dy + dz * dz)
    }

    this.onMeasurementCb(this._decorate(formatLength(dist)))
  }

  /** Prefix a "Copy" tag onto the readout while Option/Alt is held. */
  private _decorate(text: string): string {
    return this.copyMode ? `Copy · ${text}` : text
  }

  /**
   * Rebuild the axis guide line in the preview group.
   * Called whenever lockAxis changes or the stage enters 'base'.
   * Removes the previous guide (if any) before adding a new one.
   */
  private _updateGuideLine(): void {
    // Remove old guide
    if (this.guideLine !== null) {
      this.guideLine.geometry.dispose()
      if (this.guideLine.material instanceof THREE.Material) {
        this.guideLine.material.dispose()
      }
      this.preview.remove(this.guideLine)
      this.guideLine = null
    }

    if (this.stage.kind !== 'base' || this.lockAxis === null) return

    const [bx, by, bz] = this.stage.base
    const dir = AXIS_DIR[this.lockAxis]
    const color = AXIS_COLOR[this.lockAxis]

    const pts = new Float32Array([
      bx - dir[0] * GUIDE_HALF_LENGTH,
      by - dir[1] * GUIDE_HALF_LENGTH,
      bz - dir[2] * GUIDE_HALF_LENGTH,
      bx + dir[0] * GUIDE_HALF_LENGTH,
      by + dir[1] * GUIDE_HALF_LENGTH,
      bz + dir[2] * GUIDE_HALF_LENGTH,
    ])

    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(pts, 3))
    const mat = new THREE.LineBasicMaterial({ color, depthTest: false })
    const line = new THREE.LineSegments(geo, mat)
    this.preview.add(line)
    this.guideLine = line
  }
}
