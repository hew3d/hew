/**
 * RotateTool — SketchUp-style rotation about an arbitrary axis, driven by a
 * live "protractor" widget (a screen-constant disk that shows, and lets you
 * lock, the rotation axis before you commit to it).
 *
 * The protractor (a 360° ring lying in the rotation plane, centered on the
 * cursor) is visible the moment the tool is active. Its NORMAL is the rotation
 * axis; the ring is colored by that axis (X=red / Y=green / Z=blue, blue by
 * default because the ground plane's normal is +Z), or neutral purple when the
 * axis is off every world axis. This makes the axis you're about to rotate
 * around visible up front — the single biggest source of "the rotation went
 * somewhere I didn't expect", especially for cylinders, whose curved side
 * offers no face to infer an axis from.
 *
 * Axis inference & locking (works from the idle/hover phase onward, so you can
 * settle the axis BEFORE placing the center — the SketchUp order):
 *   - Hover a face  → axis = that face's normal.
 *   - Hover an edge → axis = the edge's direction (spin about the edge).
 *   - Otherwise     → axis = world +Z (ground-plane spin).
 *   - Shift toggles a lock on the current axis (the ring renders at full
 *     opacity with a short normal tick so the lock is obvious). Toggle, not
 *     hold — matches Slice/Protractor.
 *   - Arrow keys force-lock a world axis: → X, ← Y, ↑ Z; ↓ clears the lock
 *     and returns to inference. Locking with an arrow is how you tip a
 *     cylinder onto its side (see the status hint).
 *
 * Gesture (three-click), once the axis is settled:
 *   1. Click  : set the pivot (center of rotation). The axis is captured here.
 *   2. Click  : set the start-reference point (the 0° arm).
 *   3. Move   : sweep — a ghost of the selection rotates live; the angle snaps
 *               to 15° increments unless a value is typed. The protractor draws
 *               a dim baseline arm (0°) and a colored swept arm at the angle.
 *   4. Click  : commit the rotation by the live delta about the pivot (a single
 *               node → its per-kind transform; a multi-selection → one
 *               transform_selection call, one undo step).
 *   5. Esc    : cancel the gesture (and clear any axis lock).
 *
 * Numeric VCB (while sweeping): type digits / . / - to build an angle buffer;
 * Enter commits that exact number of DEGREES (unitless) about the effective
 * axis.
 *
 * Copy: tapping Option/Alt toggles copy mode — a DURABLE toggle (not
 * hold-to-copy), so an exact angle can still be typed with the modifier long
 * released (mirrors MoveTool's Alt idiom). While on, the readout is prefixed
 * "Copy ·", the cursor grows a `+` badge, and the commit becomes a
 * duplicate: one `duplicate_selection_array` call (count 1, ONE undo step)
 * carrying the COMMITTED rotation affine (`rotateAboutPivotAxis`) in place
 * of a plain transform — the clones become the new selection so a
 * follow-up rotation chains off them. Sketch selections copy too (see
 * `transformSelection.duplicateSketchSelectionByAffine`), split per target
 * sketch by whether the rotation maps that sketch's plane onto itself: an
 * in-plane rotation (axis normal to the sketch, pivot anywhere on it — the
 * spokes/petals/bolt-ring case) REPLAYS the rotated geometry into the SAME
 * sketch, one undo step per source sketch, so sticky merging and later
 * cross-copy drawing keep working; anything else (out-of-plane, or a
 * 180°-about-an-in-plane-axis flip) copies via `copy_sketch_islands` (Move's
 * out-of-plane kernel primitive) onto one new sketch built with the rotation
 * baked in, also one undo step per source sketch. Tapping Alt again returns
 * to plain Rotate.
 *
 * Array copy (SketchUp's N× / N÷): immediately after a copy commits, typing
 * `3x` (or `x3`, `*3` — both token orders) + Enter re-resolves into 3 total
 * copies continuing at the SAME angular step; `3/` (or `/3`) + Enter into 3
 * copies evenly dividing the swept angle. The gesture stays "hot" (idle, no
 * new action begun) so a different count can be re-entered. Object/group/
 * instance sources go through the kernel's `duplicate_selection_array`,
 * which composes the rotation affine CUMULATIVELY (N copies of one
 * committed rotation land at θ, 2θ, …, Nθ about the same pivot/axis) and is
 * ONE history entry regardless of N. Sketch sources have no such kernel
 * primitive, so the array builds the N cumulative affines itself and
 * replays them into `duplicateSketchSelectionByAffineArray`, which brackets
 * every in-plane rep into ONE sketch gesture per touched sketch (also ONE
 * history entry); an out-of-plane sketch source costs one
 * `copy_sketch_islands` call per rep instead, since that primitive has no
 * count parameter. Retracting a hot array for re-resolution therefore
 * retraces exactly however many entries the LAST resolve recorded (a
 * history-generation delta, tracked as `recordedEntries`) rather than a
 * hardcoded one, so a mixed or sketch-only array retracts and reissues as
 * cleanly as a pure object array. This is MoveTool's array machinery
 * (`editArrayBuffer`/`parseArraySpec`, the history-generation guard on the
 * retracting undo) — see MoveTool's doc comment for the shared reasoning.
 *
 * The rotation plane is the plane through the pivot whose normal is the
 * effective axis. Once an axis LOCK is engaged (Shift or an arrow key) and a
 * pivot exists, `snapConstraint()` feeds that plane to the snap resolver
 * (`types.ts`/`snapService.ts`), so every subsequent click and the live sweep
 * only ever see in-plane candidates (or the plane-intersection fallback) —
 * the reference arm and the swept target are always exactly what the cursor
 * confirmed, never a free-3D point that merely projects near it. Unlocked
 * rotation has no plane to offer (the axis is still just inferred, and may
 * change every hover) and stays fully unconstrained, using ray/plane
 * intersection for the live sweep as before.
 *
 * If nothing is selected, the first click auto-selects whatever is under the
 * cursor (via the Viewport-injected selection acquirer) and starts the
 * rotation on it in the same gesture; only a click over empty space shows a
 * hint toast and stays idle.
 */

import * as THREE from 'three'
import type { Tool, Snap, EditContext } from './types'
import { editContextEq } from './types'
import type { Ray } from '../viewport/math'
import {
  screenConstantWorldHalf,
  screenConstantWorldHalfFromWorldPerPixel,
  tanHalfFovRad,
  legacyScreenConstantToPixels,
  LEGACY_REFERENCE_FOV_DEG,
  LEGACY_REFERENCE_VIEWPORT_HEIGHT_PX,
} from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { getDrawingAxes } from './drawingAxes'
import {
  rotateAboutPivotAxis,
  rotationAxisAffine,
  signedAngleAboutAxis,
  snapAngleDeg,
  affineToFloat64,
  projectOntoPlane,
  planeBasis,
  normalize3,
} from './transformMath'
import { rayPlaneIntersect } from '../viewport/geoHelpers'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { clearPreview } from './transformPreview'
import {
  commitSelectionTransform,
  buildSelectionPreview,
  duplicateSketchSelectionByAffine,
  duplicateSketchSelectionByAffineArray,
} from './transformSelection'
import {
  arrowToAxis,
  editArrayBuffer,
  editNumericBuffer,
  parseArraySpec,
  parseDistance,
} from './moveInput'
import { axisColorForDirection, axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'
import type { NodeRef } from '../panels/treeModel'
import { nodeKindToNumber, nodeRefFromJs } from '../panels/treeModel'

export type OnRotateCommit = (nodes: NodeRef[]) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void
export type OnCopyModeChange = (on: boolean) => void

type Vec3 = [number, number, number]

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'pivot'
      nodes: NodeRef[]
      pivot: Vec3
      /** Rotation axis captured at the pivot click (unit). Overridable by a
       * later Shift/arrow lock via `lockedNormal`. */
      axis: Vec3
    }
  | {
      kind: 'ref'
      nodes: NodeRef[]
      pivot: Vec3
      axis: Vec3
      /** The 3D reference point (2nd click); the baseline (0°) arm is this
       * point projected into the rotation plane. */
      refPoint: Vec3
      previewMesh: THREE.Object3D | null
      /** Last computed delta (radians, snapped) — held steady when the cursor
       * ray is parallel to the rotation plane. */
      lastDelta: number
    }

const SNAP_DEG = 15
const AXIS_LABEL: Record<0 | 1 | 2, string> = { 0: 'X', 1: 'Y', 2: 'Z' }
/** Default axis: world +Z, the ground plane's normal (a blue protractor). */
const WORLD_UP: Vec3 = [0, 0, 1]
/** Neutral (off-axis) ring color — matches Slice/Protractor's neutral plane. */
const NEUTRAL_PREVIEW_COLOR = 0x9933cc
/** Dim color for the baseline (0°) arm, so the colored swept arm reads against it. */
const BASELINE_ARM_COLOR = 0x888888
/** Axis-color tolerance: within ~2° of a world axis, expressed as cos(θ). */
const AXIS_SNAP_TOL_DOT = Math.cos((2 * Math.PI) / 180)
/** Local radius the ring/arm geometry is built at; the group is scaled to keep
 * it a constant on-screen size (see DISK_SCREEN_PX / updateDiskScale). */
const DISK_UNIT_RADIUS = 1.0
/** Sample count for the protractor ring. */
const DISK_SEGMENTS = 64
/** Length of the locked-axis normal tick, as a fraction of the unit radius. */
const DISK_TICK_LENGTH = DISK_UNIT_RADIUS * 0.5
/**
 * Screen-constant disk radius in pixels, fed to `screenConstantWorldHalf`
 * (`viewport/math.ts`) every frame in `updateDiskScale` — replaces a former
 * `worldRadius = DISK_SCREEN_K * cameraDistance` constant (K = 0.06) that
 * baked `tan(fov/2)/viewportHeight` into a single number and so drifted
 * whenever the fov changed or the viewport was resized. `legacyScreenConstantToPixels`
 * converts that old K, evaluated at the app's reference fov/viewport, into an
 * equivalent pixel size — unchanged from before at that baseline, and now
 * ACTUALLY constant everywhere else too. ~104px sizes the protractor to
 * roughly the Slicer's section-plane gizmo (same source K).
 */
const DISK_SCREEN_PX = legacyScreenConstantToPixels(0.06, LEGACY_REFERENCE_FOV_DEG, LEGACY_REFERENCE_VIEWPORT_HEIGHT_PX)

interface Spoke {
  dir: Vec3
  color: number
}

export class RotateTool implements Tool {
  readonly name = 'Rotate'

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    if (this.stage.kind === 'idle') {
      if (this.arrayHot !== null) {
        return 'Type 3x to make 3 copies, or 3/ to divide the angle — Enter applies.'
      }
      if (this.selection.length === 0) {
        return 'Click the object you want to rotate.'
      }
      return this.copyMode
        ? 'Copy is on — click to set the center of rotation. Tap Alt to rotate instead.'
        : 'Click to set the center of rotation. The protractor tilts to the face or edge under the cursor — Shift locks that axis, or press → / ← / ↑ to lock X / Y / Z (needed to tip a cylinder onto its side).'
    }
    if (this.stage.kind === 'pivot') {
      return this.copyMode
        ? 'Click a start point for the angle. Shift or → / ← / ↑ lock the rotation axis. Tap Alt to rotate instead.'
        : 'Click a start point for the angle. Shift or → / ← / ↑ lock the rotation axis.'
    }
    return this.copyMode
      ? 'Move to set the angle (snaps to 15°), or type exact degrees, then click to place the copy. Shift or → / ← / ↑ lock the axis. Tap Alt to rotate instead.'
      : 'Move to set the angle (snaps to 15°), or type exact degrees, then click. Shift or → / ← / ↑ lock the axis.'
  }

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnRotateCommit
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement
  private selection: NodeRef[] = []
  private objectsGroup: THREE.Group | null = null
  private instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null = null

  /** Auto-select fallback, injected by the Viewport (see MoveTool's). */
  private acquireSelection: ((ray: Ray) => NodeRef[] | null) | null = null
  setSelectionAcquirer(acquire: ((ray: Ray) => NodeRef[] | null) | null): void {
    this.acquireSelection = acquire
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
  /** Keep the cached targets in step with the app selection (Tool.
   * setSelection; see MoveTool) — the next gesture starts from live
   * handles after an undo/redo prune. */
  setSelection(nodes: NodeRef[]): void {
    this.selection = nodes
  }

  /** Axis locked by Shift/arrow (unit). Overrides inference; null = infer. */
  private lockedNormal: Vec3 | null = null
  /** Axis inferred from the hovered face/edge (idle only). Null before the
   * first move. */
  private candidateNormal: Vec3 | null = null
  /** Last snapped cursor point, so the idle protractor can be re-centered after
   * a lock-key change that didn't come with a pointer move. */
  private lastSnapPoint: Vec3 | null = null
  /** The protractor widget (ring + optional lock tick + optional arms). */
  private previewDisk: THREE.Group | null = null
  /** VCB buffer — raw string being typed by the user. */
  private typed: string = ''

  /** Durable copy toggle (tap Option/Alt) — while true, commits duplicate. */
  private copyMode: boolean = false
  /** Notifies the Viewport when the durable copy toggle flips (cursor badge). */
  private onCopyModeChange: OnCopyModeChange
  /**
   * The just-committed copy gesture, kept "hot" for an ×N / /N array
   * refinement — MoveTool's identical idiom (see its doc comment for the
   * history-generation reasoning), adapted to a rotation: `copySources` are
   * the ORIGINAL duplicable object/group/instance nodes, `sketchSources` the
   * ORIGINAL duplicable sketch nodes (each kind arrays through a different
   * primitive — see the class doc comment), `pivot`/`axis`/`theta` the
   * committed rotation, `recordedEntries` the exact number of document
   * history entries the LAST commit/resolve produced (a history-generation
   * delta — `_resolveArray` retracts precisely that many before reissuing at
   * a new count), and `historyGen` the document's history generation right
   * after the commit/resolve (the "still on top of the undo stack" guard).
   */
  private arrayHot: {
    copySources: NodeRef[]
    sketchSources: NodeRef[]
    pivot: Vec3
    axis: Vec3
    theta: number
    recordedEntries: number
    historyGen: string
  } | null = null
  /** Array-copy VCB buffer ("x3" / "/3"), live only while `arrayHot` is set. */
  private arrayTyped: string = ''
  /**
   * Commit callback for an ×N / /N array re-resolve. Unlike `onCommit`'s
   * targeted refresh, this must trigger a FULL scene refresh — see
   * MoveTool's `onArrayCommit` doc. Defaults to `onCommit` for tests/callers
   * that don't care.
   */
  private onArrayCommit: OnRotateCommit

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    objectsGroup: THREE.Group | null,
    selection: NodeRef[],
    onCommit: OnRotateCommit,
    onToast: OnToast,
    instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null = null,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
    onCopyModeChange: OnCopyModeChange = () => { /* no-op */ },
    onArrayCommit: OnRotateCommit | null = null,
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.objectsGroup = objectsGroup
    this.selection = selection
    this.onCommit = onCommit
    this.onToast = onToast
    this.instanceGroupGetter = instanceGroupGetter
    this.onMeasurementCb = onMeasurement
    this.onCopyModeChange = onCopyModeChange
    this.onArrayCommit = onArrayCommit ?? onCommit
  }

  // ── Optional Tool interface extensions ─────────────────────────────────────

  capturingInput(): boolean {
    return this.stage.kind === 'pivot' || this.stage.kind === 'ref' || this.arrayHot !== null
  }

  /**
   * Per-key refinement of the capture (see Tool.capturesKey / MoveTool's
   * identical method). Only the REF stage has a live numeric VCB: an angle
   * is meaningless before the reference (0°) click exists, so REF is the one
   * stage that legitimately eats every key, mirroring MoveTool's 'base'
   * stage (whose typed distance is meaningful from the very first click).
   *
   * The PIVOT stage has nothing to type yet — its status hint makes no VCB
   * promise — so tool-switch letters/digits/Space stay live there, exactly
   * how idle-phase axis locking already works before a pivot is placed. But
   * a pivot IS already a mid-gesture commitment: `capturingInput()` reports
   * it as capturing precisely so a stray Delete/Backspace can't destroy the
   * pivoted node out from under the gesture (the Viewport's
   * `isCapturingInput`, which App.tsx's Delete handler consults, prioritizes
   * this per-key answer over `capturingInput()` — see its doc comment — so
   * that broader guard is a dead letter unless PIVOT names the two keys it
   * actually needs to keep here). So PIVOT captures ONLY 'Delete' and
   * 'Backspace' — swallowed with no VCB to feed (a harmless no-op in
   * `onKey`) rather than reaching App's edit-delete and destroying the
   * selection mid-gesture, which would leave the gesture holding a
   * stale node id that throws a confusing error at commit. Every other key
   * (tool-switch letters/digits, Space) still falls through; Shift/Alt/arrow
   * axis-lock keys still reach `onKey` regardless, via the Viewport's
   * uncaptured-key fallback (see its `onKeyDown`).
   *
   * The ARMED ×N / /N window (idle stage, `arrayHot` set) takes only what
   * its buffer needs.
   */
  capturesKey(key: string): boolean {
    if (this.stage.kind === 'ref') return true
    if (this.stage.kind === 'pivot') return key === 'Delete' || key === 'Backspace'
    if (this.arrayHot === null) return false
    return (
      (key >= '0' && key <= '9') ||
      key === 'x' || key === 'X' || key === '*' || key === '/' ||
      key === 'Backspace' || key === 'Delete' || key === 'Enter'
    )
  }

  /**
   * Cleanly close the armed ×N / /N window without resolving it — see
   * MoveTool.disarmArray for the full reasoning (called by the Viewport
   * before an explicit delete/undo/redo command).
   */
  disarmArray(): void {
    if (this.arrayHot === null && this.arrayTyped === '') return
    this.arrayHot = null
    this.arrayTyped = ''
    this.onMeasurementCb('')
  }

  /**
   * Constrain snapping to the rotation plane once an axis LOCK (Shift or an
   * arrow key) is engaged and a pivot exists (stage `pivot` or `ref`) — the
   * plane through the pivot perpendicular to the locked axis. This is what
   * makes the reference arm and the live sweep target land exactly where the
   * cursor's cue shows, instead of a free-3D snap that only *projects* near
   * it. Deliberately keyed on `lockedNormal`, not `_effectiveAxis()`: an
   * INFERRED axis (idle, or pivot/ref with no lock) can still change on
   * every hover, so constraining to it would be constraining to a plane that
   * is about to move — idle picking the pivot and unlocked rotation both
   * stay fully unconstrained, matching prior behavior.
   */
  snapConstraint(): { constraintPlane: { point: [number, number, number]; normal: [number, number, number] } } | null {
    if (this.lockedNormal === null) return null
    if (this.stage.kind !== 'pivot' && this.stage.kind !== 'ref') return null
    return { constraintPlane: { point: this.stage.pivot, normal: this.lockedNormal } }
  }

  /**
   * Keep the protractor a constant on-screen size regardless of camera
   * distance, fov/zoom, or viewport resize — called from the Viewport render
   * loop every frame (feature-detected via `'updateDiskScale' in tool`),
   * passing the camera's own position (for the per-disk distance) and a
   * `worldPerPixel` callback derived from the active `CameraRig` (see
   * `ScaleTool.updateGripScale`'s doc comment for the shared derivation).
   * Projection-agnostic: `worldPerPixel` already resolves the perspective vs.
   * parallel distinction (docs/design/camera.md §1), so this never needs an
   * `instanceof PerspectiveCamera` guard — a former guard here silently hid
   * the protractor under parallel projection, exactly the bug that design
   * closes. No-op when no disk is shown.
   */
  updateDiskScale(camera: THREE.Camera, worldPerPixel: (dist: number) => number): void {
    if (this.previewDisk === null) return
    const dist = camera.position.distanceTo(this.previewDisk.position)
    const scale = screenConstantWorldHalfFromWorldPerPixel(DISK_SCREEN_PX, worldPerPixel(dist))
    this.previewDisk.scale.setScalar(scale)
  }

  // ── Tool interface ──────────────────────────────────────────────────────────

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (this.stage.kind === 'ref') {
      const { pivot, previewMesh } = this.stage
      const axis = this._effectiveAxis()
      // Locked: `snapConstraint()` fed the resolver our rotation plane, so
      // `snap` is either an in-plane kernel candidate or the resolver's own
      // ray∩plane fallback (`snapService.ts`) — consuming it lets the sweep
      // land exactly ON a snapped target (SketchUp parity: "rotate to that
      // vertex"), the 15° soft snap below yielding to a real one. `snap`
      // is null only when the fallback ray∩plane also failed (ray parallel
      // to the plane), matching the unlocked branch's own null case.
      // Unlocked: no constraint plane was offered, so `snap` may be any
      // free 3D point (or unrelated inference); keep the existing pure
      // ray∩plane intersection rather than changing unlocked behavior.
      const cursorPoint = this.lockedNormal !== null
        ? (snap !== null ? ([snap.x, snap.y, snap.z] as Vec3) : null)
        : rayPlaneIntersect(ray.origin, ray.direction, pivot, axis)
      if (cursorPoint === null) {
        // Ray parallel to the rotation plane — hold the previous delta.
        if (previewMesh !== null) this._applyPreviewRotation(previewMesh, pivot, axis, this.stage.lastDelta)
        this._refreshDisk()
        this._reportAngleOrTyped(this.stage.lastDelta)
        return
      }

      const refVec: Vec3 = [
        this.stage.refPoint[0] - pivot[0], this.stage.refPoint[1] - pivot[1], this.stage.refPoint[2] - pivot[2],
      ]
      const cursorVec: Vec3 = [
        cursorPoint[0] - pivot[0], cursorPoint[1] - pivot[1], cursorPoint[2] - pivot[2],
      ]
      const raw = signedAngleAboutAxis(axis[0], axis[1], axis[2], refVec[0], refVec[1], refVec[2], cursorVec[0], cursorVec[1], cursorVec[2])
      const delta = snapAngleDeg(raw, SNAP_DEG)
      this.stage.lastDelta = delta

      if (previewMesh !== null) this._applyPreviewRotation(previewMesh, pivot, axis, delta)
      this._refreshDisk()
      this._reportAngleOrTyped(delta)
      return
    }

    // idle / pivot: keep the protractor centered under the cursor (idle) or at
    // the pivot (pivot), oriented to the inferred/locked axis.
    if (snap === null) return
    if (this.stage.kind === 'idle') {
      this.lastSnapPoint = [snap.x, snap.y, snap.z]
      this.candidateNormal = this._resolveAxis(snap, ray)
    }
    this._refreshDisk()
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'idle') {
      let nodes = this.selection
      if (nodes.length === 0 && this.acquireSelection !== null) {
        // Empty selection: auto-select whatever the click landed on and
        // start the rotation on it in the same gesture.
        const acquired = this.acquireSelection(ray)
        if (acquired !== null && acquired.length > 0) {
          this.selection = acquired
          nodes = acquired
        }
      }
      if (nodes.length === 0) {
        this.onToast('Click an object to rotate it')
        return
      }
      // Starting a new gesture ends the ×N / /N refinement window.
      this.arrayHot = null
      this.arrayTyped = ''

      const pivot: Vec3 = [snap.x, snap.y, snap.z]
      // Refresh inference from the click snap so the captured axis reflects the
      // exact face/edge under the cursor even if no move preceded this click.
      this.candidateNormal = this._resolveAxis(snap, ray)
      const axis = this._effectiveAxis()
      this.typed = ''
      this.stage = { kind: 'pivot', nodes, pivot, axis }
      this.lastSnapPoint = pivot
      this._refreshDisk()
      this.onMeasurementCb('')
    } else if (this.stage.kind === 'pivot') {
      const { nodes, pivot, axis } = this.stage
      const rawRef: Vec3 = [snap.x, snap.y, snap.z]
      // Ignore a reference that coincides with the pivot, or lies on the axis
      // through it: its projection into the rotation plane is ~zero, which would
      // freeze the sweep at 0° with no feedback. Wait for a usable reference.
      const effAxis = this._effectiveAxis()
      const offset = projectOntoPlane(
        rawRef[0] - pivot[0], rawRef[1] - pivot[1], rawRef[2] - pivot[2],
        effAxis[0], effAxis[1], effAxis[2],
      )
      const baseline = normalize3(offset)
      if (baseline === null) return
      // When locked, store the reference point projected INTO the rotation
      // plane — `snapConstraint()` already asked the resolver to keep the
      // snap in-plane, but this is the defensive/exact form regardless of
      // what the resolved snap actually carried, so the drawn baseline arm
      // passes through precisely the point the cursor confirmed. Unlocked
      // rotation keeps the raw snap (unconstrained, as before).
      const refPoint: Vec3 = this.lockedNormal !== null
        ? [pivot[0] + offset[0], pivot[1] + offset[1], pivot[2] + offset[2]]
        : rawRef
      const previewMesh = this._buildPreview(nodes)
      if (previewMesh !== null) {
        this.preview.add(previewMesh)
      }
      this.stage = { kind: 'ref', nodes, pivot, axis, refPoint, previewMesh, lastDelta: 0 }
      this.typed = ''
      this._refreshDisk()
      this.onMeasurementCb(this._decorate('0.0°'))
    } else if (this.stage.kind === 'ref') {
      const { nodes, pivot, lastDelta } = this.stage
      const delta = lastDelta
      const axis = this._effectiveAxis()

      this._resetToIdle()

      if (Math.abs(delta) < 1e-9) {
        // No-op rotation
        return
      }

      this._commit(nodes, pivot, axis, delta)
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
      return
    }

    // ── Durable copy toggle: TAP Alt/Option to flip, any stage ──
    // A toggle rather than hold-to-copy so an exact angle can be typed
    // afterwards — MoveTool's identical idiom (MoveTool.onKey).
    if (ev.key === 'Alt') {
      if (!ev.repeat) {
        ev.preventDefault() // keep the browser's Alt menu-focus behavior out
        this.copyMode = !this.copyMode
        this.onCopyModeChange(this.copyMode)
        if (this.stage.kind === 'ref') {
          this._reportAngleOrTyped(this.stage.lastDelta)
        }
      }
      return
    }

    // ── Array-copy VCB (×N / /N), while a copy commit is "hot" ──
    // Only intercepts when the window is actually armed — an idle Rotate
    // with no hot window still needs Shift/arrow inference-locking below
    // (unlike MoveTool, which has no idle-phase axis behavior to preserve).
    if (this.stage.kind === 'idle' && this.arrayHot !== null) {
      if (ev.key === 'Enter') {
        this._resolveArray()
        return
      }
      const next = editArrayBuffer(this.arrayTyped, ev.key)
      if (next !== this.arrayTyped) {
        this.arrayTyped = next
        this.onMeasurementCb(this._arrayReadout())
      }
      return
    }

    // Shift toggles the axis lock (SketchUp inference-lock convention — toggle,
    // not hold, matching Slice/Protractor). Guard autorepeat so a held Shift
    // doesn't flicker the lock on and off.
    if (ev.key === 'Shift') {
      if (!ev.repeat) {
        this.lockedNormal = this.lockedNormal === null
          ? (normalize3(this._effectiveAxis()) ?? WORLD_UP)
          : null
        this._afterAxisChange()
      }
      return
    }

    // Arrow keys force-lock a world axis (→ X, ← Y, ↑ Z; ↓ clears the lock and
    // returns to inference). Works from the idle phase on, so a cylinder's spin
    // axis can be set before the center is even placed.
    if (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft' || ev.key === 'ArrowUp' || ev.key === 'ArrowDown') {
      ev.preventDefault()
      const idx = arrowToAxis(ev.key) // → 0/X, ← 1/Y, ↑ 2/Z, ↓ null
      this.lockedNormal = idx === null ? null : this.axisDir(idx)
      this._afterAxisChange()
      return
    }

    // Numeric VCB is only meaningful while sweeping (typing an angle).
    if (this.stage.kind !== 'ref') return

    if (ev.key === 'Enter') {
      const n = parseDistance(this.typed)
      if (n !== null) {
        // Degrees are unitless — commit directly, no metersFromUnit conversion.
        const theta = (n * Math.PI) / 180
        const { nodes, pivot } = this.stage
        const axis = this._effectiveAxis()
        this._resetToIdle()
        if (Math.abs(theta) > 1e-9) {
          this._commit(nodes, pivot, axis, theta)
        }
      }
      return
    }

    if (
      (ev.key >= '0' && ev.key <= '9') ||
      ev.key === '.' ||
      ev.key === '-' ||
      ev.key === 'Backspace'
    ) {
      this.typed = editNumericBuffer(this.typed, ev.key)
      this._reportTyped()
    }
  }

  cancel(): void {
    this._resetToIdle()
  }

  // ── Private helpers ─────────────────────────────────────────────────────────

  /**
   * Unit direction of drawing axis `axis` (0=red/X, 1=green/Y, 2=blue/Z) in
   * the CURRENT frame (tool-parity design §4 — movable drawing axes). Reads
   * the kernel's live frame every call (cheap — a handful of float reads
   * through `getDrawingAxes`) rather than caching it. Replaces the old
   * world-constant `WORLD_AXIS` lookup table.
   */
  private axisDir(axis: 0 | 1 | 2): Vec3 {
    const f = getDrawingAxes(this.wasmScene)
    return axis === 0 ? f.x : axis === 1 ? f.y : f.z
  }

  /** The effective rotation axis (unit): a Shift/arrow lock wins; otherwise the
   * inferred axis (idle) or the pivot-captured axis (pivot/ref). */
  private _effectiveAxis(): Vec3 {
    if (this.lockedNormal !== null) return this.lockedNormal
    if (this.stage.kind === 'pivot' || this.stage.kind === 'ref') return this.stage.axis
    return this.candidateNormal ?? WORLD_UP
  }

  /**
   * Resolve the rotation axis for a hover/click snap:
   *   1. Snap on a live world-Object face → its unit normal.
   *   2. Snap on an edge/axis with a direction → that direction (spin about it).
   *   3. Otherwise → the face under the cursor ray (so a click on a face's
   *      corner or edge still tilts the protractor to that face), else +Z.
   */
  private _resolveAxis(snap: Snap, ray: Ray): Vec3 {
    if (snap.elementKind === 'face' && snap.object !== undefined && snap.element !== undefined) {
      try {
        const n = this.wasmScene.face_normal(snap.object, snap.element)
        const normal = normalize3([n[0], n[1], n[2]])
        if (normal !== null) return normal
      } catch {
        // Not a live world-Object face (e.g. instanced geometry) — fall through.
      }
    }
    if (snap.direction !== undefined) {
      const d = normalize3(snap.direction)
      if (d !== null) return d
    }
    return this._pickFaceAxis(ray)
  }

  /**
   * Pick the face under the ray (if any) and return its unit normal. Falls back
   * to world +Z when no face is hit.
   */
  private _pickFaceAxis(ray: Ray): Vec3 {
    let pick: ReturnType<WasmScene['pick_face']> | undefined
    try {
      pick = this.wasmScene.pick_face(
        ray.origin[0], ray.origin[1], ray.origin[2],
        ray.direction[0], ray.direction[1], ray.direction[2],
      )
      if (pick === undefined) return WORLD_UP
      const n = this.wasmScene.face_normal(pick.object(), pick.face())
      return normalize3([n[0], n[1], n[2]]) ?? WORLD_UP
    } catch {
      return WORLD_UP
    } finally {
      pick?.free()
    }
  }

  /** Rotate an in-plane vector about `axis` by `theta` (radians). */
  private _rotateVec(v: Vec3, axis: Vec3, theta: number): Vec3 {
    const a = rotationAxisAffine(axis[0], axis[1], axis[2], theta)
    return [
      a[0] * v[0] + a[1] * v[1] + a[2] * v[2],
      a[4] * v[0] + a[5] * v[1] + a[6] * v[2],
      a[8] * v[0] + a[9] * v[1] + a[10] * v[2],
    ]
  }

  /** Report the angle, prefixed with an axis tag when the axis is world-aligned. */
  private _reportAngle(deltaRad: number): void {
    const deg = (deltaRad * 180) / Math.PI
    this.onMeasurementCb(this._decorate(`${this._axisTag()}${deg.toFixed(1)}°`))
  }

  /** Report the in-progress typed-angle buffer (with axis tag). */
  private _reportTyped(): void {
    this.onMeasurementCb(this._decorate(`${this._axisTag()}${this.typed}°`))
  }

  /** Prefix a "Copy" tag onto the readout while Option/Alt is on — MoveTool's
   * identical `_decorate`. */
  private _decorate(text: string): string {
    return this.copyMode ? `Copy · ${text}` : text
  }

  /**
   * While the user is mid-type, the typed buffer wins the readout (so it stays
   * visible as they type — the Viewport re-calls onPointerMove right after each
   * keystroke, which would otherwise overwrite it with the live cursor angle).
   */
  private _reportAngleOrTyped(deltaRad: number): void {
    if (this.typed !== '') {
      this._reportTyped()
      return
    }
    this._reportAngle(deltaRad)
  }

  /** "X "/"Y "/"Z " when the effective axis is aligned with the CURRENT
   *  drawing-axes frame (tool-parity §4), else "". */
  private _axisTag(): string {
    const match = axisColorForDirection(
      this._effectiveAxis(), AXIS_SNAP_TOL_DOT, undefined, getDrawingAxes(this.wasmScene),
    )
    return match !== null ? `${AXIS_LABEL[match.axis]} ` : ''
  }

  private _buildPreview(nodes: NodeRef[]): THREE.Object3D | null {
    return buildSelectionPreview(
      this.wasmScene,
      this.objectsGroup,
      this.instanceGroupGetter,
      nodes,
      this._activeInstance,
    )
  }

  /** Redraw the protractor for the current stage/axis. Reads the ghost from the
   * stage (in ref) but does not touch it. */
  private _refreshDisk(): void {
    const normal = normalize3(this._effectiveAxis()) ?? WORLD_UP
    const locked = this.lockedNormal !== null
    const match = axisColorForDirection(
      normal, AXIS_SNAP_TOL_DOT, axisColorsForTheme(getResolvedTheme()), getDrawingAxes(this.wasmScene),
    )
    const color = match !== null ? match.color : NEUTRAL_PREVIEW_COLOR

    let center: Vec3 | null
    const spokes: Spoke[] = []
    if (this.stage.kind === 'ref') {
      center = this.stage.pivot
      const baselineDir = normalize3([
        this.stage.refPoint[0] - this.stage.pivot[0],
        this.stage.refPoint[1] - this.stage.pivot[1],
        this.stage.refPoint[2] - this.stage.pivot[2],
      ])
      const baselineInPlane = baselineDir !== null
        ? normalize3(projectOntoPlane(baselineDir[0], baselineDir[1], baselineDir[2], normal[0], normal[1], normal[2]))
        : null
      if (baselineInPlane !== null) {
        spokes.push({ dir: baselineInPlane, color: BASELINE_ARM_COLOR })
        spokes.push({ dir: this._rotateVec(baselineInPlane, normal, this.stage.lastDelta), color })
      }
    } else if (this.stage.kind === 'pivot') {
      center = this.stage.pivot
    } else {
      center = this.lastSnapPoint
    }
    if (center === null) return
    this._updatePreviewDisk(center, normal, color, locked, spokes)
  }

  /**
   * Redraw the protractor after a Shift/arrow axis change: re-orient the ring
   * and, if a ghost is live (ref stage), re-apply the rotation about the new
   * axis so the ghost tracks the axis change without waiting for a move.
   */
  private _afterAxisChange(): void {
    this._refreshDisk()
    if (this.stage.kind === 'ref' && this.stage.previewMesh !== null) {
      const axis = this._effectiveAxis()
      this._applyPreviewRotation(this.stage.previewMesh, this.stage.pivot, axis, this.stage.lastDelta)
      this._reportAngleOrTyped(this.stage.lastDelta)
    }
  }

  /**
   * Commit the rotation. In copy mode this duplicates instead of
   * transforming — MoveTool's `_commit` copy branch verbatim, adapted to a
   * rotation affine: see the class doc comment for the full reasoning
   * (kernel primitives, undo-step counts, sketch envelope).
   */
  private _commit(nodes: NodeRef[], pivot: Vec3, axis: Vec3, theta: number): void {
    try {
      const affine = rotateAboutPivotAxis(pivot[0], pivot[1], pivot[2], axis[0], axis[1], axis[2], theta)
      const affineF64 = affineToFloat64(affine)
      const copyables = nodes.filter(
        (n) => n.kind === 'object' || n.kind === 'group' || n.kind === 'instance',
      )
      const sketchSources = nodes.filter(
        (n) =>
          n.kind === 'sketch' ||
          n.kind === 'sketch-island' ||
          n.kind === 'sketch-edge' ||
          n.kind === 'sketch-curve',
      )
      if (this.copyMode && (copyables.length > 0 || sketchSources.length > 0)) {
        // Copy mode: duplicate at the rotated pose instead of rotating. Each
        // copy is the same kind as its source; the copies become the
        // selection so a follow-up rotation chains off them. Sketch geometry
        // copies FIRST (see `duplicateSketchSelectionByAffine` — in-plane
        // sketches replay into themselves, everything else copies via
        // `copy_sketch_islands`), then all duplicable nodes clone in ONE
        // `duplicate_selection_array` call (count 1).
        const committed: NodeRef[] = []
        try {
          const genBefore = this.wasmScene.history_generation()
          const sketchCopies = sketchSources.length > 0
            ? duplicateSketchSelectionByAffine(this.wasmScene, nodes, affineF64)
            : []
          committed.push(...sketchCopies)
          if (copyables.length > 0) {
            const created = this._duplicateArray(copyables, affineF64, 1)
            committed.push(...created)
          }
          // The copy gesture is "hot" for an ×N / /N array refinement
          // regardless of which kind(s) duplicated — `recordedEntries` (a
          // history-generation delta) tells `_resolveArray` exactly how many
          // steps to retract before reissuing at a new count, whether that
          // turns out to be one (objects, or a single in-plane sketch),
          // several (multiple touched sketches, or out-of-plane copies), or
          // both kinds at once (see the class doc comment).
          const recordedEntries = Number(this.wasmScene.history_generation() - genBefore)
          this.arrayHot = {
            copySources: copyables,
            sketchSources,
            pivot,
            axis,
            theta,
            recordedEntries,
            historyGen: this.wasmScene.history_generation().toString(),
          }
          this.arrayTyped = ''
        } finally {
          if (committed.length > 0) {
            this.selection = committed
            this.onCommit(committed)
          }
        }
      } else {
        commitSelectionTransform(this.wasmScene, nodes, affineF64, this._activeInstance)
        this.onCommit(nodes)
      }
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
    }
  }

  /** One `duplicate_selection_array` call over `nodes`, mapped to NodeRefs —
   * MoveTool's identical helper. */
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

  /** The array buffer as a readout, with the display glyph for `x`. */
  private _arrayReadout(): string {
    return this.arrayTyped.replace('x', '×')
  }

  /**
   * Resolve the typed ×N / /N array against the hot copy commit — MoveTool's
   * `_resolveArray` idiom, adapted to a rotation AND to sketch sources (see
   * the class doc comment): retract exactly `hot.recordedEntries` steps
   * (guarded by the HISTORY GENERATION), then reissue at the new count —
   * the object half through ONE `duplicate_selection_array` call (the
   * kernel composes the per-step rotation affine CUMULATIVELY, so N copies
   * land at θ, 2θ, …, Nθ about the same pivot/axis), the sketch half by
   * building those same N cumulative affines here and handing them to
   * `duplicateSketchSelectionByAffineArray`. `/N` divides the swept angle
   * instead of repeating it — dividing recomputes every affine from the
   * (unchanged) originally-committed `hot.theta`, so re-typing `/N` after an
   * earlier `×M` always divides the ORIGINAL sweep, never the last resolve's.
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

    if (this.wasmScene.history_generation().toString() !== hot.historyGen) {
      this.arrayHot = null
      this.onToast('Array entry ended — the model changed since the copy')
      return
    }

    const theta = spec.mode === 'divide' ? hot.theta / spec.count : hot.theta

    // Retract the previous commit/resolve — however many entries it
    // recorded (one in-plane sketch replay per touched sketch, one
    // out-of-plane copy per prior rep, one object-array call, or any
    // combination — see `_commit`'s identical bookkeeping).
    for (let i = 0; i < hot.recordedEntries; i += 1) this.wasmScene.scene_undo().free()

    const genBefore = this.wasmScene.history_generation()
    let created: NodeRef[]
    try {
      // One cumulative affine per copy: θ, 2θ, …, Nθ about the same
      // pivot/axis. The object half only needs the STEP affine (the kernel
      // call composes it); the sketch half has no such primitive, so it
      // gets every cumulative affine up front.
      const affines: Float64Array[] = []
      for (let k = 1; k <= spec.count; k += 1) {
        affines.push(
          affineToFloat64(
            rotateAboutPivotAxis(
              hot.pivot[0], hot.pivot[1], hot.pivot[2],
              hot.axis[0], hot.axis[1], hot.axis[2],
              theta * k,
            ),
          ),
        )
      }
      created = hot.sketchSources.length > 0
        ? duplicateSketchSelectionByAffineArray(this.wasmScene, hot.sketchSources, affines)
        : []
      if (hot.copySources.length > 0) {
        created.push(...this._duplicateArray(hot.copySources, affines[0], spec.count))
      }
    } catch (err) {
      // Put the retracted copies back so a refused refinement never eats the
      // committed copy — see MoveTool's identical recovery.
      try {
        for (let i = 0; i < hot.recordedEntries; i += 1) this.wasmScene.scene_redo().free()
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

    const recordedEntries = Number(this.wasmScene.history_generation() - genBefore)
    this.arrayHot = {
      ...hot,
      recordedEntries,
      historyGen: this.wasmScene.history_generation().toString(),
    }
    this.selection = created
    this.onArrayCommit(created)
  }

  private _resetToIdle(): void {
    this.stage = { kind: 'idle' }
    this.lockedNormal = null
    this.candidateNormal = null
    this.lastSnapPoint = null
    this.typed = ''
    this.arrayHot = null
    this.arrayTyped = ''
    this._clearDisk()
    clearPreview(this.preview)
    this.onMeasurementCb('')
  }

  /**
   * Update the ghost mesh by applying the rotation delta to its THREE.js matrix.
   * We reset and recompute rather than incrementally rotating so the preview
   * stays accurate on every pointer move.
   */
  private _applyPreviewRotation(mesh: THREE.Object3D, pivot: Vec3, axis: Vec3, theta: number): void {
    mesh.position.set(0, 0, 0)
    mesh.rotation.set(0, 0, 0)
    mesh.updateMatrix()

    const affine = rotateAboutPivotAxis(pivot[0], pivot[1], pivot[2], axis[0], axis[1], axis[2], theta)
    const m4 = new THREE.Matrix4()
    m4.set(
      affine[0], affine[1], affine[2], affine[3],
      affine[4], affine[5], affine[6], affine[7],
      affine[8], affine[9], affine[10], affine[11],
      0, 0, 0, 1,
    )
    mesh.applyMatrix4(m4)
  }

  /**
   * Rebuild the protractor: a ring (LineLoop) centered at `center`, lying in the
   * plane ⊥ `normal`, colored `color`. When `locked`, render at full opacity
   * with a short normal-axis tick so the lock is obvious; otherwise render
   * lighter. `spokes` are in-plane unit directions drawn as radial arms (the
   * baseline and swept-angle arms during a sweep).
   */
  private _updatePreviewDisk(center: Vec3, normal: Vec3, color: number, locked: boolean, spokes: Spoke[]): void {
    this._clearDisk()

    const unitNormal = normalize3(normal) ?? WORLD_UP
    const { u, v } = planeBasis(unitNormal)

    const ringPts = new Float32Array(DISK_SEGMENTS * 3)
    for (let i = 0; i < DISK_SEGMENTS; i++) {
      const theta = (i / DISK_SEGMENTS) * Math.PI * 2
      const c = Math.cos(theta), s = Math.sin(theta)
      ringPts[i * 3 + 0] = DISK_UNIT_RADIUS * (c * u[0] + s * v[0])
      ringPts[i * 3 + 1] = DISK_UNIT_RADIUS * (c * u[1] + s * v[1])
      ringPts[i * 3 + 2] = DISK_UNIT_RADIUS * (c * u[2] + s * v[2])
    }
    const ringGeo = new THREE.BufferGeometry()
    ringGeo.setAttribute('position', new THREE.BufferAttribute(ringPts, 3))
    const ringMat = new THREE.LineBasicMaterial({
      color,
      depthTest: false,
      transparent: !locked,
      opacity: locked ? 1 : 0.5,
    })
    const ring = new THREE.LineLoop(ringGeo, ringMat)

    const group = new THREE.Group()
    group.position.set(center[0], center[1], center[2])
    // Placeholder scale — updateDiskScale() corrects it next render frame
    // (avoids a one-frame flash at the unit radius before the screen-constant
    // size is applied). ~4 m fallback distance, at the reference fov/viewport.
    group.scale.setScalar(
      screenConstantWorldHalf(DISK_SCREEN_PX, 4, tanHalfFovRad(LEGACY_REFERENCE_FOV_DEG), LEGACY_REFERENCE_VIEWPORT_HEIGHT_PX),
    )
    group.add(ring)

    if (locked) {
      const tickPts = new Float32Array([
        0, 0, 0,
        unitNormal[0] * DISK_TICK_LENGTH,
        unitNormal[1] * DISK_TICK_LENGTH,
        unitNormal[2] * DISK_TICK_LENGTH,
      ])
      const tickGeo = new THREE.BufferGeometry()
      tickGeo.setAttribute('position', new THREE.BufferAttribute(tickPts, 3))
      const tickMat = new THREE.LineBasicMaterial({ color, depthTest: false })
      group.add(new THREE.LineSegments(tickGeo, tickMat))
    }

    for (const spoke of spokes) {
      const armPts = new Float32Array([
        0, 0, 0,
        spoke.dir[0] * DISK_UNIT_RADIUS,
        spoke.dir[1] * DISK_UNIT_RADIUS,
        spoke.dir[2] * DISK_UNIT_RADIUS,
      ])
      const armGeo = new THREE.BufferGeometry()
      armGeo.setAttribute('position', new THREE.BufferAttribute(armPts, 3))
      const armMat = new THREE.LineBasicMaterial({ color: spoke.color, depthTest: false })
      group.add(new THREE.LineSegments(armGeo, armMat))
    }

    this.preview.add(group)
    this.previewDisk = group
  }

  private _clearDisk(): void {
    if (this.previewDisk === null) return
    for (const child of this.previewDisk.children) {
      if (child instanceof THREE.LineLoop || child instanceof THREE.LineSegments) {
        child.geometry.dispose()
        if (child.material instanceof THREE.Material) {
          child.material.dispose()
        }
      }
    }
    this.preview.remove(this.previewDisk)
    this.previewDisk = null
  }
}
