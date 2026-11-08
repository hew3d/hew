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
 * Copy: tapping the platform copy modifier toggles copy mode — a DURABLE
 * toggle, not hold-to-copy, so an exact distance can be typed with the
 * modifier long released (SketchUp's own Ctrl/Option split, see
 * `platform.ts`'s `COPY_MODIFIER_KEY`): Option on macOS (`onKey`'s bare
 * keydown toggle below), Ctrl on Windows/Linux (a Viewport-level Control
 * clean tap that calls `toggleCopyMode` directly, so a Ctrl+Z/Ctrl+C chord
 * never triggers it). While on, the readout is prefixed "Copy ·", the cursor
 * grows a `+` badge, and the commit becomes a duplicate: one
 * `duplicate_selection_array` call (count 1, ONE undo step for the whole
 * selection's copies), the clones becoming the new selection so follow-up
 * moves chain copies. Object/group copies are independent baked geometry; an
 * instance copy shares its definition at the offset pose. Drawn shapes
 * copy too: one draft per plane context copies the shapes at the offset
 * (`duplicateSketchSelection`, on a face or over the void alike), curve
 * identity preserved; the whole copy — drawing and objects — is ONE undo
 * step, atomic. Tapping the modifier again returns to plain Move. Holding it through a drag still works: the keydown/clean-tap
 * on entry toggles copy on.
 *
 * Array copy (SketchUp's N× / N÷): immediately after a copy commits, typing
 * `3x` (or `x3`, `*3` — both token orders are accepted) + Enter re-resolves
 * the commit into 3 total copies at the same spacing continuing along the
 * vector; `3/` (or `/3`) + Enter into 3 copies evenly dividing the
 * committed distance. While the gesture stays "hot" (no
 * new action begun) a different `xN`/`/N` can be re-entered and the array
 * re-resolves — implemented as one scene undo of the previous array commit
 * plus a fresh `duplicate_selection_array` (drawn shapes:
 * `duplicateSketchSelectionByAffineArray` over cumulative translations, all
 * reps of one plane context in ONE draft), the whole array one history
 * bracket, so the final history holds ONE step for it. The retracting undo is guarded by the document's
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
 *
 * Guides: a selected construction guide (the app keeps it apart from the
 * node selection, `setGuideTarget`) moves the same way — preview, axis
 * locks, typed and retyped distances — committing one `transform_guide`;
 * copy mode and its ×N / /N array place copies through one
 * `duplicate_guide_array`, the last copy becoming the selected guide. With
 * no nodes selected, a click on a guide starts moving that guide
 * (`setGuideHooks`), before the node acquirer runs — as a Select click
 * takes a guide over the object beneath. A guide taken that way is
 * selected only for the gesture (guideTarget.ts): afterwards the selection
 * is what it was before the click.
 * On commit: one kernel transform call, then handleSceneRefresh + onDocumentChanged.
 */

import { formatNumber } from '../i18n/format'
import { plural, t } from '@lingui/core/macro'
import * as THREE from 'three'
import type { Tool, Snap, EditContext, SnapConstraint } from './types'
import { editContextEq } from './types'
import { allFree, isDraftRef, planeOfCtx } from './planeUnits'
import { worldFacePlane } from './faceDraw'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { getDrawingAxes } from './drawingAxes'
import { translationAffine, affineToFloat64 } from './transformMath'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { clearPreview } from './transformPreview'
import {
  commitSelectionTransform,
  buildSelectionPreview,
  dropRidingDrafts,
  copyOwnersAndDrawing,
} from './transformSelection'
import {
  arrowToAxis,
  axisLockColorName,
  editArrayBuffer,
  editLengthBuffer,
  isLengthInputKey,
  parseArraySpec,
  pointAlong,
  type ArraySpec,
} from './moveInput'
import { RetypeWindow, retypeStaleMessage } from './retypeWindow'
import { buildGuidePreview } from './guidePreview'
import { GuideTarget, type OnGuideCommit } from './guideTarget'
import type { NodeRef } from '../panels/treeModel'
import { isSketchScopedKind, nodeKindToNumber, nodeRefFromJs } from '../panels/treeModel'
import { displayedLength, isFreePick } from './displayedValue'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { isMac, COPY_MODIFIER_LABEL } from '../platform'
import { nextCopyState, linkedFor, type CopyState, type OnCopyModeChange } from './copyMode'
import { getCopiesLinked } from '../settings/editing'
import { parseLinkedResult, type LinkedWrapRef } from '../viewport/linkedWraps'

export type OnMoveCommit = (nodes: NodeRef[]) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void
export type { OnCopyModeChange } from './copyMode'

/** Axis guide colours matching the world axis convention */
const AXIS_COLOR: Record<0 | 1 | 2, number> = {
  0: 0xff0000,   // X — red
  1: 0x00aa00,   // Y — green
  2: 0x0000ff,   // Z — blue
}

/** Half-extent of the axis guide line drawn through the base point */
const GUIDE_HALF_LENGTH = 50

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'base'
      nodes: NodeRef[]
      /** The construction guide being moved instead of `nodes` (then empty). */
      guide: bigint | null
      base: [number, number, number]
      previewMesh: THREE.Object3D | null
      /** Last snapped/computed destination (updated every pointer move). */
      dest: [number, number, number]
    }

/**
 * The move (or copy) just committed, kept in a `RetypeWindow`
 * (retypeWindow.ts) so a distance typed AFTER the commit redoes it at that
 * distance along the same direction — a negative value flips it. `copy`
 * is the copy toggle in force at the commit, re-applied for the redo. Lives
 * beside the ×N / /N array window: while idle, one typed buffer serves
 * both — a buffer that parses as an array spec (`3x`, `/3`) resolves the
 * array, anything else that parses as a length retypes the distance.
 */
/** What an array resolve placed: node copies, or a guide's (the last one). */
type ArrayLanding = { nodes: NodeRef[] } | { guide: bigint }

type RetypeSpec = { nodes: NodeRef[]; guide: bigint | null; vector: [number, number, number]; copyState: CopyState; linked: boolean }

export class MoveTool implements Tool {
  readonly name = 'Move'
  /** Click-drag-release works as well as click-click (Tool.dragCommits). */
  readonly dragCommits = true

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    if (this.stage.kind === 'idle') {
      if (this.retype.isOpen) {
        return this.arrayHot !== null
          ? this._armedArrayHint(true)
          : t`Type an exact distance to redo the move you just made — or click a base point to start another.`
      }
      if (this.arrayHot !== null) {
        return this._armedArrayHint()
      }
      if (this.selection.length === 0 && this.guides.selected === null) {
        return t`Click the object you want to move.`
      }
      const kind = this._copyKindLabel()
      const next = this._cycleNextLabel()
      return this.copyState === 'off'
        ? t`Click a base point to start the move.`
        : t`${kind} is on — click a base point to start the copy. Tap ${COPY_MODIFIER_LABEL} to ${next}.`
    }
    const next = this._cycleNextLabel()
    return this.copyState === 'off'
      ? t`Click the destination — type an exact distance, arrow keys lock an axis, tap ${COPY_MODIFIER_LABEL} to place a copy.`
      : t`Click where the copy lands — type an exact distance, arrow keys lock an axis, tap ${COPY_MODIFIER_LABEL} to ${next}.`
  }

  /** Whether `key` is an arrow Move owns right now: only with a grid laid or
   *  a two-count spec being typed. Otherwise arrows keep their global meaning,
   *  as they did before grids. */
  private _arrowsAreGridKeys(key: string): boolean {
    if (this.arrayHot === null) return false
    if (arrowToAxis(key) === null && key !== 'ArrowDown') return false
    return this.arrayLast?.count2 !== undefined || this.typed.includes(',')
  }

  /** The hint while the array window is armed: what to type, and, once a
   *  grid is laid, which way its rows run. */
  private _armedArrayHint(redoable = false): string {
    const hot = this.arrayHot
    if (hot !== null && this.arrayLast?.count2 !== undefined) {
      // Always the axis of the grid that is laid, never a pending choice.
      const axis = this._gridAxisIndex(hot)
      if (axis !== null) {
        const color = axisLockColorName(axis)
        return t`Rows along ${color} — press an arrow key to change them, or type another count. Enter applies.`
      }
      return t`Rows across the move — press an arrow key to change them, or type another count. Enter applies.`
    }
    return redoable
      ? t`Type an exact distance to redo the copy you just made, 3x to add copies, 3/ to divide it, or 4x,3x for a grid — Enter applies.`
      : t`Type 3x to make 3 copies, 3/ to divide the distance, or 4x,3x for a grid — Enter applies.`
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
  /** Durable copy toggle (tap Option on macOS, Ctrl elsewhere — see
   *  `toggleCopyMode`) — while true, commits duplicate. */
  private copyState: CopyState = 'off'
  /** Whether a copy commits right now (any state but plain Move). */
  private get copyMode(): boolean {
    return this.copyState !== 'off'
  }
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
    /** The ORIGINAL object/group/instance sources (one `duplicate_selection_array` call). */
    sources: NodeRef[]
    /** The ORIGINAL drawn shapes / sketches (`duplicateSketchSelectionByAffineArray`). */
    sketchSources: NodeRef[]
    /** The ORIGINAL guide (`duplicate_guide_array`), when a guide was copied. */
    guide: bigint | null
    vector: [number, number, number]
    historyGen: string
    /** Whether the copy this window follows was linked; the array repeats it. */
    linked: boolean
    /** The copy state the copy was made under, re-armed on the retype window. */
    copyState: CopyState
    /** The second-direction axis of the grid that is laid (null = the default direction). */
    axis2: 0 | 1 | 2 | null
    /** An arrow choice not yet laid (typed text was in the way); undefined = none. */
    pendingAxis2?: 0 | 1 | 2 | null
  } | null = null
  /** Array-copy VCB buffer ("x3" / "/3"), live only while `arrayHot` is set. */
  private arrayTyped: string = ''

  /** The ×N / /N spec the array window last resolved, or null while the hot
   *  copy is still a single copy — a distance typed afterwards re-spaces the
   *  whole array rather than collapsing it back to one copy. */
  private arrayLast: ArraySpec | null = null
  /** The move/copy just committed, redoable at a typed distance until the
   *  next pointer action, Escape, tool switch, or other document mutation
   *  (see `RetypeSpec` and retypeWindow.ts). */
  private readonly retype: RetypeWindow<RetypeSpec>

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

  /** The construction guide this tool acts on instead of nodes (see
   *  guideTarget.ts). */
  private readonly guides = new GuideTarget()
  /** The app's selected guide, pushed by the Viewport like `setSelection`. */
  setGuideTarget(guide: bigint | null): void {
    this.guides.selected = guide
  }
  /** The Viewport's guide click pick and post-commit refresh. */
  setGuideHooks(acquire: ((ray: Ray) => bigint | null) | null, onCommit: OnGuideCommit): void {
    this.guides.setHooks(acquire, onCommit)
  }

  /** The current editing context (component-edit-parity.md phase A1). When
   *  it's an INSTANCE, selected members commit through `transform_def_member`
   *  instead of the world `transform_selection` (phase A2) — see
   *  `commitSelectionTransform`'s doc. */
  private _editContext: EditContext = { kind: 'top' }
  setEditContext(ctx: EditContext): void {
    if (editContextEq(ctx, this._editContext)) return
    this._editContext = ctx
    this.cancel()
  }
  private get _activeInstance(): bigint | null {
    return this._editContext.kind === 'instance' ? this._editContext.id : null
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
    this.retype = new RetypeWindow(wasmScene)
    this.instanceGroupGetter = instanceGroupGetter
    this.onCopyModeChange = onCopyModeChange
    this.onArrayCommit = onArrayCommit ?? onCommit
  }

  /**
   * Flip the durable copy toggle. Called from `onKey`'s bare Option/Alt tap
   * on macOS, and — duck-typed by method name — from the Viewport's
   * window-scope Control clean tap on Windows/Linux (see `platform.ts`'s
   * `COPY_MODIFIER_KEY`). Same effect either way: advances the copy cycle (Move, Copy, the other kind of copy, Move), notifies
   * the cursor badge, and refreshes the mid-gesture measurement readout so
   * the "Copy · " prefix appears/disappears immediately rather than waiting
   * for the next pointer move.
   */
  toggleCopyMode(): void {
    this.copyState = nextCopyState(this.copyState)
    this.onCopyModeChange(this.copyState, linkedFor(this.copyState, getCopiesLinked()))
    if (this.stage.kind === 'base') {
      this._reportMeasurement(this.stage.base, this.stage.dest)
    }
  }

  /** The loose nodes the latest linked copy wrapped as sets; a wrap removes
   *  a world node, so the host needs a full refresh rather than a targeted one. */
  private linkedWraps: readonly LinkedWrapRef[] = []
  /** While a retype redoes a copy: the kind that copy was, whatever the preference says now. */
  private retypeLinked: boolean | null = null
  /** The copy state a typed value would redo or apply: the one the open retype window holds, else the current one. */
  private _shownCopyState(): CopyState {
    return this.retype.isOpen && this.retype.spec !== null ? this.retype.spec.copyState : this.copyState
  }
  /** The current copy state, for the host to badge the cursor when the preference changes mid-gesture. */
  currentCopyState(): CopyState {
    return this.copyState
  }
  /** Hand over (and forget) the wraps the latest copy made. */
  takeLinkedWraps(): readonly LinkedWrapRef[] {
    const w = this.linkedWraps
    this.linkedWraps = []
    return w
  }

  /** What a copy made right now is called: "Copy" for the preferred kind,
   *  "Linked copy" or "Independent copy" when the tap chose the other one. */
  private _copyKindLabel(state: CopyState = this.copyState): string {
    if (state === 'alternate') {
      return getCopiesLinked() ? t`Independent copy` : t`Linked copy`
    }
    return t`Copy`
  }

  /** What the next tap of the copy key switches to, as an action phrase. */
  private _cycleNextLabel(): string {
    if (this.copyState === 'primary') {
      return getCopiesLinked() ? t`make an independent copy` : t`make a linked copy`
    }
    return t`move instead`
  }

  // ── Optional Tool interface extensions ─────────────────────────────────────

  snapConstraint(): SnapConstraint | null {
    if (this.stage.kind !== 'base') return null
    const result: SnapConstraint = {
      anchor: this.stage.base,
    }
    if (this.lockAxis !== null) {
      result.lockAxis = this.lockAxis
    }
    // A selection of drawn shapes with a face edge slides ON its plane: constrain the destination snap to it so a corner
    // snapped on some other face never lands the shape off its own (which the
    // kernel would refuse `NotInPlane`). Mixed selections and shapes on
    // different planes stay unconstrained — the kernel still refuses honestly.
    const plane = this._drawingPlane(this.selection)
    if (plane !== null) result.constraintPlane = plane
    return result
  }

  /** The one world plane every drawn shape in `nodes` lies on, when the
   *  selection is drawn shapes only and they share it (one plane context);
   *  else null. */
  private _drawingPlane(nodes: readonly NodeRef[]): { point: [number, number, number]; normal: [number, number, number] } | null {
    if (nodes.length === 0 || !nodes.every(isDraftRef)) return null
    const ctx = nodes[0].ctx
    if (ctx === undefined || !nodes.every((n) => n.ctx === ctx)) return null
    // A shape drawn wholly over the void may leave its plane (it detaches);
    // only a selection with a face edge in it is held to the plane.
    if (nodes.every((n) => allFree(this.wasmScene, n))) return null
    const pl = planeOfCtx(ctx)
    return pl === null ? null : { point: [...pl.point], normal: [...pl.normal] }
  }

  /** The two-click move itself, without the post-commit array window that
   *  `capturingInput` also covers (the drag gate, Tool.gestureInFlight). */
  gestureInFlight(): boolean {
    return this.stage.kind === 'base'
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
    if (this.arrayHot === null && !this.retype.isOpen) return false
    const arrayKey = key === 'x' || key === 'X' || key === '*' || key === '/'
    if ((key >= '0' && key <= '9') || key === '-' || key === '.') return true
    // The array window's own set, unchanged: its tokens and the guarded
    // Delete keystroke, whatever the buffer holds.
    if (this.arrayHot !== null && (arrayKey || key === ',' || key === 'Backspace' || key === 'Delete' || key === 'Enter')) return true
    // The grid's second direction is chosen with the arrow keys while the window is armed.
    if (this._arrowsAreGridKeys(key)) return true
    // The retype window: with an empty buffer every other key keeps its
    // global meaning; once something is typed the length grammar follows.
    if (this.typed === '' || key === ' ') return false // Space always resets to Select
    return key === 'Enter' || key === 'Backspace' || arrayKey || isLengthInputKey(key)
  }

  /** Armed for Escape's purposes while a gesture is live, the array window
   *  is open, or a post-commit value is being typed. */
  hasArmedGesture(): boolean {
    return this.stage.kind === 'base' || this.arrayHot !== null || this.typed !== ''
  }

  /** Quietly close the retype window — the host calls this before an
   *  explicit undo/redo/delete (`disarmActivePostCommitWindow`), alongside
   *  `disarmArray`. */
  disarmRetype(): void {
    this.retype.close()
    if (this.stage.kind === 'idle' && this.typed !== '') {
      this.typed = ''
      this.onMeasurementCb('')
    }
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
    // An explicit document command ends the retype window too — same reason.
    this.retype.close()
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

  /**
   * Unit direction of drawing axis `axis` (0=red/X, 1=green/Y, 2=blue/Z) in
   * the CURRENT frame (tool-parity design §4 — movable drawing axes). Reads
   * the kernel's live frame every call rather than caching it: cheap (a
   * handful of float reads through `getDrawingAxes`) and always current,
   * even mid-gesture if a concurrent undo/redo moved the frame under it.
   * Replaces the old world-constant `AXIS_DIR` lookup table.
   */
  private axisDir(axis: 0 | 1 | 2): [number, number, number] {
    const f = getDrawingAxes(this.wasmScene)
    return axis === 0 ? f.x : axis === 1 ? f.y : f.z
  }

  /** The drawing axis (in the CURRENT frame) the base→dest drag is most
   *  aligned with, or null. Projects the drag vector onto each of the
   *  frame's three directions (dot product) and picks the largest |dot| —
   *  under a moved frame this locks Shift to the axis the drag visually
   *  looks closest to, not literal world XYZ. */
  private _dominantAxis(): 0 | 1 | 2 | null {
    if (this.stage.kind !== 'base') return null
    const { base, dest } = this.stage
    const d: [number, number, number] = [dest[0] - base[0], dest[1] - base[1], dest[2] - base[2]]
    const f = getDrawingAxes(this.wasmScene)
    const dots: [number, number, number] = [
      Math.abs(d[0] * f.x[0] + d[1] * f.x[1] + d[2] * f.x[2]),
      Math.abs(d[0] * f.y[0] + d[1] * f.y[1] + d[2] * f.y[2]),
      Math.abs(d[0] * f.z[0] + d[1] * f.z[1] + d[2] * f.z[2]),
    ]
    const max = Math.max(dots[0], dots[1], dots[2])
    if (max < 1e-9) return null
    if (max === dots[0]) return 0
    if (max === dots[1]) return 1
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
    // Any pointer action ends the retype window — the next move begins —
    // and a half-typed post-commit value must not leak into the next
    // gesture's own length buffer.
    this.retype.close()
    if (this.stage.kind === 'idle' && this.typed !== '') {
      this.typed = ''
      this.onMeasurementCb('')
    }
    if (snap === null) return

    if (this.stage.kind === 'idle') {
      let nodes = this.selection
      // No nodes selected: a click on a guide takes THAT guide, whether
      // another one is selected or none — selecting and moving it in one
      // gesture. A click elsewhere moves the selected guide from there.
      // With nodes selected the click is their base point, a guide's snap
      // included.
      const guide = this.guides.start(ray, nodes.length === 0, true)
      if (nodes.length === 0 && guide === null && this.acquireSelection !== null) {
        // Empty selection: auto-select whatever the click landed on and
        // start the move on it in the same gesture (no separate Select step).
        const acquired = this.acquireSelection(ray)
        if (acquired !== null && acquired.length > 0) {
          this.selection = acquired
          nodes = acquired
        }
      }
      if (nodes.length === 0 && guide === null) {
        this.onToast(t`Click an object to move it`)
        return
      }

      // Starting a new gesture ends the ×N / /N refinement window.
      this.arrayHot = null
      this.arrayTyped = ''

      const previewMesh = guide !== null ? buildGuidePreview(this.wasmScene, guide) : this._buildPreview(nodes)
      const base: [number, number, number] = [snap.x, snap.y, snap.z]
      if (previewMesh !== null) {
        previewMesh.position.set(0, 0, 0)
        this.preview.add(previewMesh)
      }

      this.stage = { kind: 'base', nodes, guide, base, previewMesh, dest: [...base] }
      this._updateGuideLine()
    } else if (this.stage.kind === 'base') {
      const { nodes, guide, base } = this.stage
      const tx = snap.x - base[0]
      const ty = snap.y - base[1]
      const tz = snap.z - base[2]

      // Degenerate: no movement
      if (Math.abs(tx) < 1e-9 && Math.abs(ty) < 1e-9 && Math.abs(tz) < 1e-9) {
        this._resetToIdle()
        this.guides.ended()
        return
      }

      // A free pick commits the DISPLAYED distance through the typed path
      // (direction from base -> cursor, or the locked axis, as when typed).
      if (isFreePick(snap, base)) {
        const len = this.lockAxis !== null
          ? Math.abs(tx * this.axisDir(this.lockAxis)[0] + ty * this.axisDir(this.lockAxis)[1] + tz * this.axisDir(this.lockAxis)[2])
          : Math.hypot(tx, ty, tz)
        const shown = displayedLength(len)
        if (shown > 0) {
          this.stage.dest = [snap.x, snap.y, snap.z]
          this._commitFromTyped(shown)
          return
        }
      }

      this._commitAndReset(nodes, guide, tx, ty, tz)
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
      return
    }

    // ── Durable copy toggle: TAP Option to flip, any stage — macOS only ──
    // A toggle rather than hold-to-copy so an exact distance can be typed
    // afterwards (macOS Option+digit would otherwise mangle the keystrokes).
    // Modifier keys don't autorepeat everywhere, but guard anyway.
    // Windows/Linux use Ctrl instead (SketchUp's own split): a bare Control
    // keydown reports `ctrlKey: true` on itself, so the generic dispatch that
    // reaches `onKey` never carries it (see Viewport's `onKeyDown`, gated on
    // `!isMod`) — that platform's toggle is armed/fired entirely from the
    // Viewport's window-scope Control clean tap instead, which calls
    // `toggleCopyMode` directly (see `platform.ts`'s `COPY_MODIFIER_KEY` and
    // `cleanModifierTap.ts` — the clean tap is also what makes a Ctrl+Z/
    // Ctrl+C chord safe there, which a bare keydown toggle like this one
    // cannot guarantee).
    if (isMac && ev.key === 'Alt') {
      if (!ev.repeat) {
        ev.preventDefault() // keep the browser's Alt menu-focus behavior out
        this.toggleCopyMode()
      }
      return
    }

    // ── Idle: the array window (×N / /N while a copy is "hot") and the
    // retype window (a distance after any commit) share one buffer — see
    // `RetypeSpec`. Enter routes by what the buffer parses as.
    if (this.stage.kind === 'idle') {
      if (this.arrayHot === null && !this.retype.isOpen) return
      if (ev.key === 'Enter') {
        const buf = this.typed
        this.typed = ''
        this.onMeasurementCb('')
        if (this.arrayHot !== null && parseArraySpec(buf) !== null) {
          this.arrayTyped = buf
          this._resolveArray() // re-arms the retype window on the array
          return
        }
        const meters = parseLengthToMeters(buf)
        if (meters !== null && this.retype.isOpen) this._retypeDistance(meters)
        return
      }
      if (this._arrowsAreGridKeys(ev.key)) {
        this._chooseGridAxis(arrowToAxis(ev.key))
        return
      }
      const next = this._editIdleBuffer(this.typed, ev.key)
      if (next !== this.typed) {
        this.typed = next
        this.onMeasurementCb(this._idleReadout())
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
    const live = this.stage.kind === 'base'
    this._resetToIdle()
    if (live) this.guides.ended()
  }

  // ── Private helpers ─────────────────────────────────────────────────────────

  /** Commit the move via translationAffine, then reset to idle. */
  private _commitAndReset(nodes: NodeRef[], guide: bigint | null, tx: number, ty: number, tz: number): void {
    this._resetToIdle()
    this._commit(nodes, guide, tx, ty, tz)
  }

  /**
   * Commit from the typed VCB buffer.  The direction is:
   *   - The locked axis (signed by which side of the base the cursor is on),
   *   - or the vector from base → last dest if no axis is locked.
   */
  private _commitFromTyped(dist: number): void {
    if (this.stage.kind !== 'base') return
    const { nodes, guide, base, dest } = this.stage

    let dir: [number, number, number]
    if (this.lockAxis !== null) {
      // Signed direction: match the cursor side so typing "2" means "2 in the
      // direction the cursor is pointing".
      const lockedDir = this.axisDir(this.lockAxis)
      const dotSign = (dest[0] - base[0]) * lockedDir[0]
                    + (dest[1] - base[1]) * lockedDir[1]
                    + (dest[2] - base[2]) * lockedDir[2]
      const sign = dotSign < 0 ? -1 : 1
      dir = [lockedDir[0] * sign, lockedDir[1] * sign, lockedDir[2] * sign]
    } else {
      dir = [dest[0] - base[0], dest[1] - base[1], dest[2] - base[2]]
    }

    const endpoint = pointAlong(base, dir, dist)
    const tx = endpoint[0] - base[0]
    const ty = endpoint[1] - base[1]
    const tz = endpoint[2] - base[2]

    if (Math.abs(tx) < 1e-9 && Math.abs(ty) < 1e-9 && Math.abs(tz) < 1e-9) {
      // A distance typed before the cursor left the base point has no
      // direction to go in; say so rather than silently doing nothing.
      this._resetToIdle()
      if (dist !== 0) {
        this.onToast(t`Move the cursor the way it should go, or lock an axis with an arrow key, then type the distance.`)
      }
      return
    }

    this._commitAndReset(nodes, guide, tx, ty, tz)
  }

  private _resetToIdle(): void {
    this.retype.close()
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

  /** True when the kernel accepted the commit (false = refused, toasted).
   *  Opens the retype window on the result either way it commits — a plain
   *  move or a copy (whose array window opens alongside). */
  private _commit(allNodes: NodeRef[], guide: bigint | null, tx: number, ty: number, tz: number): boolean {
    if (guide !== null) return this._commitGuide(guide, tx, ty, tz)
    try {
      // Drawn shapes on a selected object ride with it (see `dropRidingDrafts`).
      const nodes = dropRidingDrafts(this.wasmScene, allNodes)
      this.arrayLast = null
      const affineF64 = affineToFloat64(translationAffine(tx, ty, tz))
      const copyables = nodes.filter(
        (n) => n.kind === 'object' || n.kind === 'group' || n.kind === 'instance',
      )
      const hasSketchNodes = nodes.some((n) => isSketchScopedKind(n.kind))
      if (this.copyMode && (copyables.length > 0 || hasSketchNodes)) {
        // Copy mode: duplicate at the offset instead of moving. Each copy is
        // the same kind as its source; the copies become the selection so a
        // follow-up move chains off them.
        //
        // The whole copy is ONE undo step and atomic (`copyOwnersAndDrawing`
        // brackets the history when drawing and objects copy together): the
        // drawn shapes copy through the drafting engine, then all duplicable
        // nodes clone in ONE `duplicate_selection_array` call (count 1), so a
        // single Cmd+Z removes everything and an ×N / /N refinement retracts
        // exactly one step. A refusal changes nothing; the finally block only
        // reselects whatever did land.
        const committed: NodeRef[] = []
        // The preference is read at the moment of the copy, not when the
        // tool was armed.
        const linked = this.retypeLinked ?? linkedFor(this.copyState, getCopiesLinked())
        try {
          const sketchSources = nodes.filter((n) => isSketchScopedKind(n.kind))
          committed.push(
            ...copyOwnersAndDrawing(this.wasmScene, copyables, sketchSources, [affineF64], () =>
              this._duplicateArray(copyables, affineF64, 1, linked),
            ),
          )
          // The copy gesture is now "hot" for an ×N / /N array refinement,
          // whichever kinds were copied; the resolve retracts its one step.
          this.arrayHot = {
            sources: copyables,
            sketchSources,
            guide: null,
            vector: [tx, ty, tz],
            historyGen: this.wasmScene.history_generation().toString(),
            linked,
            copyState: this.copyState,
            axis2: null,
          }
          this.arrayTyped = ''
        } finally {
          if (committed.length > 0) {
            this.selection = committed
            this.onCommit(committed)
          }
        }
        this.retype.arm({ nodes, guide: null, vector: [tx, ty, tz], copyState: this.copyState, linked })
      } else {
        // The whole selection: the planner leaves riding shapes to their
        // owner and hands them back re-found where they landed.
        const committed = commitSelectionTransform(this.wasmScene, allNodes, affineF64, this._activeInstance)
        this.onCommit(committed)
        // The window is armed with the PRE-commit `nodes`, not `committed`:
        // its apply() undoes the commit first, which restores the drawing
        // with its ORIGINAL unit keys (they are geometry hashes), and an
        // arranging landing's re-resolved refs name the moved geometry.
        this.retype.arm({ nodes: allNodes, guide: null, vector: [tx, ty, tz], copyState: 'off', linked: false })
      }
      return true
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
      return false
    }
  }

  /** `_commit` for a construction guide: one `transform_guide`, or in copy
   *  mode one `duplicate_guide_array` (count 1) whose copy becomes the
   *  selected guide and is hot for an ×N / /N array. */
  private _commitGuide(guide: bigint, tx: number, ty: number, tz: number): boolean {
    try {
      this.arrayLast = null
      const affineF64 = affineToFloat64(translationAffine(tx, ty, tz))
      if (this.copyMode) {
        const copy = this.wasmScene.duplicate_guide_array(guide, affineF64, 1)[0]
        this.arrayHot = {
          sources: [],
          sketchSources: [],
          guide,
          vector: [tx, ty, tz],
          historyGen: this.wasmScene.history_generation().toString(),
          linked: false,
          copyState: this.copyState,
          axis2: null,
        }
        this.arrayTyped = ''
        this.guides.landed(copy)
        this.retype.arm({ nodes: [], guide, vector: [tx, ty, tz], copyState: this.copyState, linked: false })
      } else {
        this.wasmScene.transform_guide(guide, affineF64)
        this.guides.landed(guide)
        this.retype.arm({ nodes: [], guide, vector: [tx, ty, tz], copyState: 'off', linked: false })
      }
      return true
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
      return false
    }
  }

  /**
   * Redo the move/copy just committed at |`d`| along its own direction (a
   * negative value flips it) through the shared window: undo the commit,
   * re-commit under the copy toggle it was made with, roll back on refusal.
   */
  private _retypeDistance(d: number): void {
    const spec = this.retype.spec
    if (spec === null) return
    const [vx, vy, vz] = spec.vector
    const len = Math.hypot(vx, vy, vz)
    if (len < 1e-12 || Math.abs(d) < 1e-9) return
    const k = d / len
    const v: [number, number, number] = [vx * k, vy * k, vz * k]
    const arrayed = this.arrayLast !== null && this.arrayHot !== null
    const outcome = this.retype.apply(
      (hot) => (arrayed ? this._relayArray(hot, v) : this._commitHot(hot, v)),
      (hot) => (arrayed ? this._relayArray(hot, hot.vector) : this._commitHot(hot, hot.vector)),
      (hot) => ({ ...hot, vector: v }),
    )
    if (outcome === 'stale') this.onToast(retypeStaleMessage('move'))
    else if (outcome === 'failed') this._restampArrayWindow()
  }

  /** After a refused retype whose window survived (the document was put
   *  back as it was), the array window's history stamp is stale: the attempt's
   *  undo/redo moved the generation. Re-stamp it so the next ×N is not
   *  rejected as 'model changed'. */
  private _restampArrayWindow(): void {
    if (this.arrayHot === null || !this.retype.isOpen) return
    this.arrayHot = { ...this.arrayHot, historyGen: this.wasmScene.history_generation().toString() }
  }

  /**
   * Re-space the array the window last resolved so its single-copy vector
   * is `v` (SketchUp: after `3x`, typing a new distance moves all three
   * copies), keeping the resolved count and multiply/divide mode. The array
   * window is re-stamped on the new vector so a further `Nx` continues from
   * the new spacing.
   */
  private _relayArray(hot: RetypeSpec, v: [number, number, number]): boolean {
    const spec = this.arrayLast
    if (spec === null) return this._commitHot(hot, v)
    const arr = this.arrayHot
    if (arr === null) return this._commitHot(hot, v)
    try {
      const created = this._arrayAt(arr, v, spec)
      this.arrayHot = {
        ...arr,
        vector: v,
        historyGen: this.wasmScene.history_generation().toString(),
      }
      this._landArray(created)
      return true
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
      return false
    }
  }

  /** Lay the hot move down along `v`, under the copy toggle it was committed with. */
  private _commitHot(hot: RetypeSpec, v: [number, number, number]): boolean {
    const prev = this.copyState
    this.copyState = hot.copyState
    this.retypeLinked = hot.linked
    try {
      return this._commit(hot.nodes, hot.guide, v[0], v[1], v[2])
    } finally {
      this.copyState = prev
      this.retypeLinked = null
    }
  }

  /** The idle buffer's editor: an array token (`x`, `/`) or a buffer already
   *  holding one follows the array grammar; everything else the length
   *  grammar. */
  private _editIdleBuffer(buf: string, key: string): string {
    const multiplyKey = key === 'x' || key === 'X' || key === '*'
    if (this.arrayHot !== null) {
      // A copy is hot: the full array grammar, `/` included (divide).
      if (/[x/]/.test(buf) || multiplyKey || key === '/' || key === ',') return editArrayBuffer(buf, key)
    } else if (multiplyKey || /x/.test(buf)) {
      // No copy to array: an `x3` attempt is still kept as typed — so Enter
      // finds no length in it and stays inert — rather than being read as
      // the distance `3`. `/` here is an imperial fraction bar (`1/2"`).
      return editArrayBuffer(buf, key)
    }
    if (isLengthInputKey(key)) return editLengthBuffer(buf, key, getLengthUnit())
    return buf
  }

  /** The idle buffer's readout: an array spec as typed (`3×`), a length in
   *  display units. */
  private _idleReadout(): string {
    if (this.typed === '') return ''
    const arrayish = /x/.test(this.typed) || (this.arrayHot !== null && /\//.test(this.typed))
    return arrayish ? this.typed.replace(/x/g, '×').replace(',', ', ') : this._decorate(this._typedReadout())
  }

  /**
   * A refused x N resolve. The previous commit's one entry was already
   * retracted and the refused attempt aborted its own history bracket, which
   * restores the document, redo stack included, exactly: one redo brings the
   * previous array (or the single copy) back.
   */
  private _restoreAfterRefusedArray(hot: NonNullable<MoveTool['arrayHot']>, genBefore: bigint): void {
    try {
      if (this.wasmScene.history_generation() !== genBefore) throw new Error('history moved during a refused array')
      this.wasmScene.scene_redo().free()
      this.arrayHot = { ...hot, historyGen: this.wasmScene.history_generation().toString() }
      // The undo + redo moved the generation: re-arm the retype window on
      // the restored entry, or a distance typed next reports 'stale'.
      this.retype.arm({ nodes: [...hot.sources, ...hot.sketchSources], guide: hot.guide, vector: hot.vector, copyState: hot.copyState, linked: hot.linked })
    } catch {
      this.arrayHot = null
    }
  }

  /** The unit direction of a grid's second axis for a move `v`: the arrow
   *  key's drawing axis when one was chosen, else the horizontal direction
   *  most nearly perpendicular to the move (world Z crossed with it; world X
   *  when the move is vertical). Null when the chosen axis runs along the move. */
  private _gridDir2(v: readonly [number, number, number], axis2: 0 | 1 | 2 | null): [number, number, number] | null {
    const len = Math.hypot(v[0], v[1], v[2])
    if (len < 1e-12) return null
    if (axis2 !== null) {
      const d = this.axisDir(axis2)
      const cx = d[1] * v[2] - d[2] * v[1]
      const cy = d[2] * v[0] - d[0] * v[2]
      const cz = d[0] * v[1] - d[1] * v[0]
      // Near-parallel (under about 6 degrees) would stack the rows on the columns.
      return Math.hypot(cx, cy, cz) < 0.1 * len ? null : [d[0], d[1], d[2]]
    }
    // z cross v
    const hx = -v[1]
    const hy = v[0]
    const h = Math.hypot(hx, hy)
    if (h < 1e-9 * len) return [1, 0, 0]
    return [hx / h, hy / h, 0]
  }

  /** The drawing axis (0 red, 1 green, 2 blue) a laid grid's rows run along,
   *  or null when they run along none of them. */
  private _gridAxisIndex(hot: NonNullable<MoveTool['arrayHot']>): 0 | 1 | 2 | null {
    if (hot.axis2 !== null) return hot.axis2
    const d = this._gridDir2(hot.vector, null)
    if (d === null) return null
    for (const a of [0, 1, 2] as const) {
      const f = this.axisDir(a)
      if (Math.abs(f[0] * d[0] + f[1] * d[1] + f[2] * d[2]) > 0.999) return a
    }
    return null
  }

  /** An arrow key while the array window is armed (Down clears): chooses
   *  the grid's second direction. A laid grid is re-laid along it at once;
   *  otherwise the choice waits for the next two-count Enter. */
  private _chooseGridAxis(axis: 0 | 1 | 2 | null): void {
    const hot = this.arrayHot
    if (hot === null) return
    const laid = this.arrayLast?.count2 !== undefined
    if (laid && axis !== null && this._gridDir2(hot.vector, axis) === null) {
      this.onToast(t`Rows cannot run along the move — pick another direction.`)
      return
    }
    const last = this.arrayLast
    if (laid && last !== null && this.typed === '') {
      if (axis !== hot.axis2) this._resolveArray(last, axis)
    } else {
      // Held back until Enter: the laid grid, and the hint, stay as they are.
      this.arrayHot = { ...hot, pendingAxis2: axis }
    }
  }

  /** The translations an array of `spec` lays down for the move `v`, in
   *  placement order. A grid is the lattice `i*v + j*s2` (`s2` one move long
   *  along the second direction) with the original's slot left out. */
  private _placementSteps(
    arr: NonNullable<MoveTool['arrayHot']>,
    v: [number, number, number],
    spec: ArraySpec,
  ): [number, number, number][] {
    if (spec.count2 === undefined) {
      const step: [number, number, number] =
        spec.mode === 'divide' ? [v[0] / spec.count, v[1] / spec.count, v[2] / spec.count] : v
      const out: [number, number, number][] = []
      for (let k = 1; k <= spec.count; k += 1) out.push([step[0] * k, step[1] * k, step[2] * k])
      return out
    }
    const len = Math.hypot(v[0], v[1], v[2])
    const d = this._gridDir2(v, arr.axis2) ?? this._gridDir2(v, null) ?? [1, 0, 0]
    const out: [number, number, number][] = []
    for (let i = 0; i <= spec.count; i += 1) {
      for (let j = 0; j <= spec.count2; j += 1) {
        if (i === 0 && j === 0) continue
        out.push([i * v[0] + j * len * d[0], i * v[1] + j * len * d[1], i * v[2] + j * len * d[2]])
      }
    }
    return out
  }

  /** Lay the array `spec` describes for the move `v` on the hot sources:
   *  the drawn shapes through the same translations, the objects through
   *  one call — `duplicate_selection_array` for a row, `duplicate_selection_placements`
   *  for a grid — or a hot guide's copies through one `duplicate_guide_array` call. */
  private _arrayAt(
    arr: NonNullable<MoveTool['arrayHot']>,
    v: [number, number, number],
    spec: ArraySpec,
  ): ArrayLanding {
    if (arr.guide !== null) {
      const step: [number, number, number] =
        spec.mode === 'divide' ? [v[0] / spec.count, v[1] / spec.count, v[2] / spec.count] : v
      const affine = affineToFloat64(translationAffine(step[0], step[1], step[2]))
      const copies = this.wasmScene.duplicate_guide_array(arr.guide, affine, spec.count)
      return { guide: copies[copies.length - 1] }
    }
    const steps = this._placementSteps(arr, v, spec)
    const affines = steps.map((s) => affineToFloat64(translationAffine(s[0], s[1], s[2])))
    return {
      nodes: copyOwnersAndDrawing(this.wasmScene, arr.sources, arr.sketchSources, affines, () =>
        spec.count2 === undefined
          ? this._duplicateArray(arr.sources, affines[0], spec.count, arr.linked)
          : this._duplicatePlacements(arr.sources, affines, arr.linked),
      ),
    }
  }

  /** Hand an array's copies to the host: node copies become the selection,
   *  a guide's last copy the selected guide. */
  private _landArray(landed: ArrayLanding): void {
    if ('guide' in landed) {
      this.guides.landed(landed.guide)
    } else {
      this.selection = landed.nodes
      this.onArrayCommit(landed.nodes)
    }
  }

  /** One `duplicate_selection_array` call over `nodes`, mapped to NodeRefs.
   *  A linked copy goes through the call that reports its wraps; the
   *  returned nodes are then placements. */
  private _duplicateArray(
    nodes: readonly NodeRef[],
    affineF64: Float64Array,
    count: number,
    linked: boolean,
  ): NodeRef[] {
    this.linkedWraps = []
    const kinds = new Uint8Array(nodes.map((n) => nodeKindToNumber(n.kind)))
    const ids = new BigUint64Array(nodes.map((n) => n.id))
    if (linked) {
      const result = parseLinkedResult(
        this.wasmScene.duplicate_selection_array_linked(kinds, ids, affineF64, count),
      )
      this.linkedWraps = result.linked
      return result.nodes
    }
    return this.wasmScene
      .duplicate_selection_array(kinds, ids, affineF64, count, false)
      .map(nodeRefFromJs)
  }

  /** One `duplicate_selection_placements` call over `nodes` (a grid), mapped
   *  to NodeRefs; a linked grid reports its wraps as a linked row does. */
  private _duplicatePlacements(
    nodes: readonly NodeRef[],
    affines: readonly Float64Array[],
    linked: boolean,
  ): NodeRef[] {
    this.linkedWraps = []
    const kinds = new Uint8Array(nodes.map((n) => nodeKindToNumber(n.kind)))
    const ids = new BigUint64Array(nodes.map((n) => n.id))
    const flat = new Float64Array(affines.length * 12)
    affines.forEach((a, i) => flat.set(a, i * 12))
    if (linked) {
      const result = parseLinkedResult(this.wasmScene.duplicate_selection_placements_linked(kinds, ids, flat))
      this.linkedWraps = result.linked
      return result.nodes
    }
    return this.wasmScene.duplicate_selection_placements(kinds, ids, flat, false).map(nodeRefFromJs)
  }

  /** The array buffer as a readout, with the display glyph for `x` (the
   * buffer holds at most one, leading `x5` or trailing `5x`). */
  private _arrayReadout(): string {
    return this.arrayTyped.replace(/x/g, '×').replace(',', ', ')
  }

  /**
   * Resolve the typed ×N / /N array against the hot copy commit: one scene
   * undo retracts the previous array commit (guarded by the HISTORY
   * GENERATION, so the undo provably pops our own action and can never
   * clobber an intervening one), then ONE `duplicate_selection_array` call
   * places the whole array — leaving a single undo step for all N copies.
   * The gesture stays hot afterwards so a different count can be re-entered.
   */
  private _resolveArray(override?: ArraySpec, axis2?: 0 | 1 | 2 | null): void {
    const current = this.arrayHot
    const spec = override ?? parseArraySpec(this.arrayTyped)
    this.arrayTyped = ''
    this.onMeasurementCb('')
    if (current === null || spec === null) return
    // A chosen direction takes effect only if the re-lay succeeds. An arrow
    // held back while text was typed applies now.
    const chosen = axis2 !== undefined ? axis2 : current.pendingAxis2
    const hot = chosen === undefined ? current : { ...current, axis2: chosen }

    const cap = this.wasmScene.max_array_count()
    // A grid's copies are its lattice slots less the original's own.
    const placements = spec.count2 === undefined ? spec.count : (spec.count + 1) * (spec.count2 + 1) - 1
    if (placements > cap) {
      const limit = formatNumber(cap, { grouping: false })
      this.onToast(
        plural(cap, { one: `Array copy is limited to ${limit} copy`, other: `Array copy is limited to ${limit} copies` }),
      )
      return
    }
    if (spec.count2 !== undefined && hot.guide !== null) {
      this.onToast(t`A guide copies along one direction only — type a single count.`)
      return
    }

    // History-identity guard: re-resolve only if the undo stack is exactly
    // as our commit left it (top of stack = our action). The generation
    // moves on every recorded action, undo, and redo — a mutation in
    // between means the refinement window is over, and the user is told so
    // rather than Enter silently doing nothing. View-state toggles (hide/
    // tag visibility) don't move it, so decluttering the scene mid-gesture
    // keeps the window open.
    if (this.wasmScene.history_generation().toString() !== current.historyGen) {
      this.arrayHot = null
      this.onToast(t`Array entry ended — the model changed since the copy`)
      return
    }

    // Retract the previous commit/resolve: always exactly one entry.
    this.wasmScene.scene_undo().free()
    const genBefore = this.wasmScene.history_generation()
    let created: ArrayLanding
    try {
      created = this._arrayAt(hot, hot.vector, spec)
    } catch (err) {
      this._restoreAfterRefusedArray(current, genBefore)
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
      return
    }

    this.arrayHot = {
      ...hot,
      pendingAxis2: undefined,
      historyGen: this.wasmScene.history_generation().toString(),
    }
    this.arrayLast = spec
    // A distance typed now re-spaces THIS array (see `_relayArray`); the
    // array is one undo step, so one undo retracts it.
    this.retype.arm({ nodes: [...hot.sources, ...hot.sketchSources], guide: hot.guide, vector: hot.vector, copyState: hot.copyState, linked: hot.linked })
    this._landArray(created)
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
      const lockedDir = this.axisDir(this.lockAxis)
      dist = (dest[0] - base[0]) * lockedDir[0]
           + (dest[1] - base[1]) * lockedDir[1]
           + (dest[2] - base[2]) * lockedDir[2]
    } else {
      const dx = dest[0] - base[0]
      const dy = dest[1] - base[1]
      const dz = dest[2] - base[2]
      dist = Math.sqrt(dx * dx + dy * dy + dz * dz)
    }

    this.onMeasurementCb(this._decorate(formatLength(dist)))
  }

  /** Prefix a "Copy" tag onto the readout while the copy toggle is on. */
  private _decorate(text: string): string {
    const shown = this._shownCopyState()
    if (shown === 'off') return text
    const kind = this._copyKindLabel(shown)
    return t({ comment: 'Measurements box readout while a copy is armed: the kind of copy (Copy, Independent copy, or Linked copy), then the measured value', message: `${kind} · ${text}` })
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
    const dir = this.axisDir(this.lockAxis)
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
