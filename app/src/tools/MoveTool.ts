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
 * Copy: tapping Option/Alt toggles copy mode — a DURABLE toggle, not
 * hold-to-copy, so an exact distance can be typed with the modifier long
 * released (SketchUp's Ctrl/Option semantics). While on, the readout is
 * prefixed "Copy ·", the cursor grows a `+` badge, and the commit becomes a
 * duplicate: one `duplicate_selection_array` call (count 1, ONE undo step for
 * the whole selection's copies), the clones becoming the new selection so
 * follow-up moves chain copies. Object/group copies are independent baked
 * geometry; an instance copy shares its definition at the offset pose.
 * Sketch selections copy too: each planned island replays into its sketch at
 * the offset (`duplicateSketchSelection`) — one gesture per sketch, one undo
 * step per sketch, curve identity preserved. Tapping Alt again returns to
 * plain Move. Holding Alt through a drag still works: the keydown on entry
 * toggles copy on.
 *
 * Array copy (SketchUp's N× / N÷): immediately after a copy commits, typing
 * `3x` (or `x3`, `*3` — both token orders are accepted) + Enter re-resolves
 * the commit into 3 total copies at the same spacing continuing along the
 * vector; `3/` (or `/3`) + Enter into 3 copies evenly dividing the
 * committed distance. While the gesture stays "hot" (no
 * new action begun) a different `xN`/`/N` can be re-entered and the array
 * re-resolves — implemented as one scene undo of the previous array commit
 * plus a fresh `duplicate_selection_array`, so the final history holds ONE
 * step for the whole array. The retracting undo is guarded by the document's
 * HISTORY GENERATION (undo-stack identity, not a content hash): any recorded
 * action, undo, or redo in between ends the window with a toast, while
 * non-undoable view-state toggles (hide, tag visibility) leave it open. The
 * armed window counts as capturing input so Delete/Backspace cannot destroy
 * the just-made copies while the status bar invites "Type ×3…".
 *
 * Numeric VCB:
 *   Type digits / . / - while in 'base' stage → builds a buffer shown as the
 *   "Length" measurement.  Press Enter to commit that exact distance along
 *   the current direction (locked axis or cursor direction) — identical in
 *   move and copy modes.
 *
 * If nothing is selected, the first click auto-selects whatever is under the
 * cursor (via the Viewport-injected selection acquirer — same context-aware
 * resolution a Select click uses) and starts the move on it in the same
 * gesture; only a click over empty space shows a hint toast and stays idle.
 * On commit: one kernel transform call, then handleSceneRefresh + onDocumentChanged.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { translationAffine, affineToFloat64 } from './transformMath'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { clearPreview } from './transformPreview'
import {
  commitSelectionTransform,
  buildSelectionPreview,
  duplicateSketchSelection,
} from './transformSelection'
import {
  arrowToAxis,
  editArrayBuffer,
  editLengthBuffer,
  isLengthInputKey,
  parseArraySpec,
  pointAlong,
} from './moveInput'
import type { NodeRef } from '../panels/treeModel'
import { nodeKindToNumber, nodeRefFromJs } from '../panels/treeModel'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'

export type OnMoveCommit = (nodes: NodeRef[]) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void
export type OnCopyModeChange = (on: boolean) => void

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

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    if (this.stage.kind === 'idle') {
      if (this.arrayHot !== null) {
        return 'Type 3x to make 3 copies, or 3/ to divide the distance — Enter applies.'
      }
      if (this.selection.length === 0) {
        return 'Click the object you want to move.'
      }
      return this.copyMode
        ? 'Copy is on — click a base point to start the copy. Tap Alt to move instead.'
        : 'Click a base point to start the move.'
    }
    return this.copyMode
      ? 'Click where the copy lands — type an exact distance, arrow keys lock an axis, tap Alt to move instead.'
      : 'Click the destination — type an exact distance, arrow keys lock an axis, tap Alt to place a copy.'
  }

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
  /** Durable copy toggle (tap Option/Alt) — while true, commits duplicate. */
  private copyMode: boolean = false
  /** VCB buffer — raw string being typed by the user */
  private typed: string = ''
  /**
   * The just-committed copy gesture, kept "hot" for an ×N / /N array
   * refinement. `sources` are the ORIGINAL duplicable nodes, `vector` the
   * committed offset, and `historyGen` the document's history generation
   * right after the commit — the undo-STACK identity that proves the
   * re-resolve's scene undo will retract exactly our action. A content
   * hash cannot stand in here: a net-zero pair of undoable edits restores
   * the hash while burying our action two entries deep (the undo would
   * silently eat the unrelated edit and a second array would stack on the
   * first), while a non-undoable view-state toggle changes the hash
   * without invalidating the window at all.
   */
  private arrayHot: {
    sources: NodeRef[]
    vector: [number, number, number]
    historyGen: string
  } | null = null
  /** Array-copy VCB buffer ("x3" / "/3"), live only while `arrayHot` is set. */
  private arrayTyped: string = ''

  /** THREE.js LineSegments for the axis guide drawn in the preview group */
  private guideLine: THREE.LineSegments | null = null

  /** The full selection at tool activation (set by Viewport from selectedIds). */
  private selection: NodeRef[] = []
  /**
   * Auto-select fallback, injected by the Viewport: given the click ray,
   * pick + context-resolve the node under the cursor, lift it into the app
   * selection, and return it — or null on a miss. Lets a click on an object
   * with an empty selection select AND start moving it in one gesture.
   */
  private acquireSelection: ((ray: Ray) => NodeRef[] | null) | null = null
  setSelectionAcquirer(acquire: ((ray: Ray) => NodeRef[] | null) | null): void {
    this.acquireSelection = acquire
  }
  /**
   * Keep the cached targets in step with the app selection (Tool.
   * setSelection) — the Viewport pushes every change, including the
   * undo/redo prune, so the next gesture always starts from live handles
   * instead of committing against dead ones. An in-flight gesture keeps
   * the nodes it started with (its stage holds its own copy); the armed
   * array window is untouched — undo already disarms it explicitly.
   */
  setSelection(nodes: NodeRef[]): void {
    this.selection = nodes
  }
  /** THREE.js object group from the SceneRenderer (read-only reference for cloning). */
  private objectsGroup: THREE.Group | null = null
  /** THREE.js instances group from the SceneRenderer (read-only reference for cloning). */
  private instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null = null
  /** Notifies the Viewport when the durable copy toggle flips (cursor badge). */
  private onCopyModeChange: OnCopyModeChange
  /**
   * Commit callback for an ×N / /N array re-resolve. Unlike `onCommit`'s
   * targeted refresh, this must trigger a FULL scene refresh: the re-resolve
   * scene-undoes the previous copies (their meshes must vanish) before the
   * new ones land. Defaults to `onCommit` for tests/callers that don't care.
   */
  private onArrayCommit: OnMoveCommit

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    objectsGroup: THREE.Group | null,
    selection: NodeRef[],
    onCommit: OnMoveCommit,
    onToast: OnToast,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
    instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null = null,
    onCopyModeChange: OnCopyModeChange = () => { /* no-op */ },
    onArrayCommit: OnMoveCommit | null = null,
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.objectsGroup = objectsGroup
    this.selection = selection
    this.onCommit = onCommit
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
    this.instanceGroupGetter = instanceGroupGetter
    this.onCopyModeChange = onCopyModeChange
    this.onArrayCommit = onArrayCommit ?? onCommit
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
    // The ARMED ×N / /N window captures even before anything is typed —
    // deliberate semantics: while the status bar is inviting "Type 3x…",
    // no single KEYSTROKE may destroy the just-made copies (App.tsx's
    // Delete/Backspace handler defers to this check, refined per key by
    // `capturesKey` below, and the copies ARE the current selection).
    // Backspace edits the possibly-empty array buffer instead; Esc remains
    // the way out, and any pointer action ends the window naturally.
    //
    // An EXPLICIT delete command (Edit ▸ Delete, the dock's Erase) is the
    // deliberate counterpart: it executes — the Viewport's delete path
    // first calls `disarmArray()` so the window closes cleanly, then
    // deletes. Only the ambiguous bare keystroke is guarded here.
    return this.stage.kind === 'base' || this.arrayHot !== null
  }

  /**
   * Per-key refinement of the capture (see Tool.capturesKey). A live
   * two-click gesture keeps the whole keyboard — its length VCB legitimately
   * eats letters (unit suffixes) and Space ("5' 3"). The ARMED array window
   * takes only what its buffer needs — digits, the x/X/* and / mode tokens,
   * Backspace, Enter — plus the bare Delete keystroke, which stays guarded
   * so no single keypress can destroy the just-made copies. Everything else
   * falls through to its global meaning; in particular Space is NEVER
   * captured — it performs its normal reset-to-Select, and the tool switch
   * cancels this tool, quietly ending the window like the other explicit
   * exits.
   */
  capturesKey(key: string): boolean {
    if (this.stage.kind === 'base') return true
    if (this.arrayHot === null) return false
    return (
      (key >= '0' && key <= '9') ||
      key === 'x' || key === 'X' || key === '*' || key === '/' ||
      key === 'Backspace' || key === 'Delete' || key === 'Enter'
    )
  }

  /**
   * Cleanly close the armed ×N / /N window without resolving it — called by
   * the Viewport before an explicit delete command removes the selection
   * (which is the just-made copies), so no hot state points at deleted
   * nodes and a later Enter can't fire a wrong-action undo or a confusing
   * toast. Quiet by design: the user asked for the delete; the window
   * simply no longer exists.
   */
  disarmArray(): void {
    if (this.arrayHot === null && this.arrayTyped === '') return
    this.arrayHot = null
    this.arrayTyped = ''
    this.onMeasurementCb('')
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

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'idle') {
      let nodes = this.selection
      if (nodes.length === 0 && this.acquireSelection !== null) {
        // Empty selection: auto-select whatever the click landed on and
        // start the move on it in the same gesture (no separate Select step).
        const acquired = this.acquireSelection(ray)
        if (acquired !== null && acquired.length > 0) {
          this.selection = acquired
          nodes = acquired
        }
      }
      if (nodes.length === 0) {
        this.onToast('Click an object to move it')
        return
      }

      // Starting a new gesture ends the ×N / /N refinement window.
      this.arrayHot = null
      this.arrayTyped = ''

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

    // ── Durable copy toggle: TAP Alt/Option to flip, any stage ──
    // A toggle rather than hold-to-copy so an exact distance can be typed
    // afterwards (macOS Option+digit would otherwise mangle the keystrokes).
    // Modifier keys don't autorepeat everywhere, but guard anyway.
    if (ev.key === 'Alt') {
      if (!ev.repeat) {
        ev.preventDefault() // keep the browser's Alt menu-focus behavior out
        this.copyMode = !this.copyMode
        this.onCopyModeChange(this.copyMode)
        if (this.stage.kind === 'base') {
          this._reportMeasurement(this.stage.base, this.stage.dest)
        }
      }
      return
    }

    // ── Array-copy VCB (×N / /N), while a copy commit is "hot" ──
    if (this.stage.kind === 'idle') {
      if (this.arrayHot !== null) {
        if (ev.key === 'Enter') {
          this._resolveArray()
          return
        }
        const next = editArrayBuffer(this.arrayTyped, ev.key)
        if (next !== this.arrayTyped) {
          this.arrayTyped = next
          this.onMeasurementCb(this._arrayReadout())
        }
      }
      return
    }

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
    this.arrayHot = null
    this.arrayTyped = ''
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
      const copyables = nodes.filter(
        (n) => n.kind === 'object' || n.kind === 'group' || n.kind === 'instance',
      )
      const hasSketchNodes = nodes.some(
        (n) =>
          n.kind === 'sketch' ||
          n.kind === 'sketch-island' ||
          n.kind === 'sketch-edge' ||
          n.kind === 'sketch-curve',
      )
      if (this.copyMode && (copyables.length > 0 || hasSketchNodes)) {
        // Copy mode: duplicate at the offset instead of moving. Each copy is
        // the same kind as its source; the copies become the selection so a
        // follow-up move chains off them.
        //
        // Sketch geometry copies by replaying its islands into the same
        // sketch at the offset (`duplicateSketchSelection`): one drawing
        // gesture per sketch, so one undo removes that sketch's whole copy,
        // and curve chains keep their analytic identity (a copied circle is
        // a true circle). It goes FIRST, then all duplicable nodes clone in
        // ONE `duplicate_selection_array` call (count 1) — the array action
        // lands last on the undo stack, so an ×N / /N refinement can retract
        // exactly it with one scene undo, and a single Cmd+Z after a
        // multi-copy removes every clone at once.
        //
        // A failed sketch replay cancels its own gesture but leaves earlier
        // sketches' copies committed — the finally block refreshes and
        // reselects whatever landed, so the viewport never renders a scene
        // that diverges from the kernel (the error still surfaces as a
        // toast). The duplicate call itself is atomic (strong guarantee).
        const committed: NodeRef[] = []
        try {
          const sketchCopies = duplicateSketchSelection(this.wasmScene, nodes, [tx, ty, tz])
          committed.push(...sketchCopies)
          if (copyables.length > 0) {
            const created = this._duplicateArray(copyables, affineF64, 1)
            committed.push(...created)
            // The copy gesture is now "hot" for an ×N / /N array refinement
            // — but only when the copy was purely objects/groups/instances:
            // the array retracts and re-issues exactly ONE
            // duplicate_selection_array step, and sketch copies live in
            // separate gesture steps it cannot retract, so arraying a mixed
            // copy would multiply the objects while the sketch copies stayed
            // at one. Sketch ×N arrays are out of scope until a kernel-side
            // sketch duplicate op exists.
            if (sketchCopies.length === 0) {
              this.arrayHot = {
                sources: copyables,
                vector: [tx, ty, tz],
                historyGen: this.wasmScene.history_generation().toString(),
              }
              this.arrayTyped = ''
            } else {
              this.arrayHot = null
            }
          } else {
            this.arrayHot = null
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

  /** One `duplicate_selection_array` call over `nodes`, mapped to NodeRefs. */
  private _duplicateArray(
    nodes: readonly NodeRef[],
    affineF64: Float64Array,
    count: number,
  ): NodeRef[] {
    const created = this.wasmScene.duplicate_selection_array(
      new Uint8Array(nodes.map((n) => nodeKindToNumber(n.kind))),
      new BigUint64Array(nodes.map((n) => n.id)),
      affineF64,
      count,
    )
    return created.map(nodeRefFromJs)
  }

  /** The array buffer as a readout, with the display glyph for `x` (the
   * buffer holds at most one, leading `x5` or trailing `5x`). */
  private _arrayReadout(): string {
    return this.arrayTyped.replace('x', '×')
  }

  /**
   * Resolve the typed ×N / /N array against the hot copy commit: one scene
   * undo retracts the previous array commit (guarded by the HISTORY
   * GENERATION, so the undo provably pops our own action and can never
   * clobber an intervening one), then ONE `duplicate_selection_array` call
   * places the whole array — leaving a single undo step for all N copies.
   * The gesture stays hot afterwards so a different count can be re-entered.
   */
  private _resolveArray(): void {
    const hot = this.arrayHot
    const spec = parseArraySpec(this.arrayTyped)
    this.arrayTyped = ''
    this.onMeasurementCb('')
    if (hot === null || spec === null) return

    const cap = this.wasmScene.max_array_count()
    if (spec.count > cap) {
      this.onToast(`Array copy is limited to ${cap} copies`)
      return
    }

    // History-identity guard: re-resolve only if the undo stack is exactly
    // as our commit left it (top of stack = our action). The generation
    // moves on every recorded action, undo, and redo — a mutation in
    // between means the refinement window is over, and the user is told so
    // rather than Enter silently doing nothing. View-state toggles (hide/
    // tag visibility) don't move it, so decluttering the scene mid-gesture
    // keeps the window open.
    if (this.wasmScene.history_generation().toString() !== hot.historyGen) {
      this.arrayHot = null
      this.onToast('Array entry ended — the model changed since the copy')
      return
    }

    const [tx, ty, tz] = hot.vector
    const step: [number, number, number] =
      spec.mode === 'divide'
        ? [tx / spec.count, ty / spec.count, tz / spec.count]
        : [tx, ty, tz]

    // Retract the previous array commit (count 1 on the first refinement).
    this.wasmScene.scene_undo().free()
    let created: NodeRef[]
    try {
      created = this._duplicateArray(
        hot.sources,
        affineToFloat64(translationAffine(step[0], step[1], step[2])),
        spec.count,
      )
    } catch (err) {
      // Put the retracted copies back so a refused refinement never eats
      // the committed copy. The undo+redo pair moved the history
      // generation, so re-stamp the token — the state is the recorded one
      // again and another count can be tried.
      try {
        this.wasmScene.scene_redo().free()
        this.arrayHot = {
          ...hot,
          historyGen: this.wasmScene.history_generation().toString(),
        }
      } catch {
        this.arrayHot = null
      }
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
      return
    }

    this.arrayHot = {
      sources: hot.sources,
      vector: hot.vector,
      historyGen: this.wasmScene.history_generation().toString(),
    }
    this.selection = created
    this.onArrayCommit(created)
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
