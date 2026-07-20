/**
 * ScaleTool — SketchUp-style bounding-box grip gizmo: scale one axis
 * (stretch, via a face-center grip), two axes (an edge-midpoint grip), or all
 * three uniformly (a corner grip), about a chosen anchor.
 *
 * The gizmo is the selection's world-axis-aligned bounding box. Whenever the
 * tool is idle with a non-empty selection, the box and its grips are drawn:
 *   - 6 face-center grips  → single-axis (1D) stretch.
 *   - 8 corner grips       → 3-axis uniform scale.
 *   - 12 edge-midpoint grips → 2-axis scale.
 *
 * Gesture (two-click, matching Move/Rotate):
 *   1. First click  : ONLY if the selection is empty, auto-select whatever is
 *                     under the cursor (see below) and draw its gizmo — this
 *                     click does not grab a grip, since grip positions aren't
 *                     known until the box exists. If a grip IS within pick
 *                     tolerance of the click ray (the ordinary case: the
 *                     gizmo was already showing for a live selection), it is
 *                     grabbed and the drag begins immediately. The anchor is
 *                     captured here: the grip OPPOSITE the one grabbed by
 *                     default, or the box CENTER if Ctrl's durable toggle is
 *                     on (see below).
 *   2. Move         : per driven axis, `s_axis = (cursor − pivot)·axis /
 *                     (grab − pivot)·axis`, clamped to `MIN_SCALE` — dragging
 *                     a grip past its anchor clamps rather than reflecting
 *                     (the kernel refuses reflection typed; mirroring is a
 *                     separate future tool). A corner grip's "axis" is the
 *                     pivot→grab diagonal, driving sx=sy=sz by one shared
 *                     ratio (uniform). Updates a THREE.js ghost preview.
 *   3. Second click : commit the scale (one node → the per-kind transform; a
 *                     multi-selection → one transform_selection call, one
 *                     undo step), then redraw the gizmo at the new size so a
 *                     follow-up grip can be grabbed right away.
 *   4. Esc          : cancel — the ghost clears, the gizmo redraws unchanged.
 *
 * Anchor (Ctrl, durable toggle — like Move's copy toggle, tap not hold): off
 * (default) anchors at the grabbed grip's OPPOSITE grip (grab the top face →
 * the bottom face stays put); on anchors at the box CENTER (both sides move).
 * Toggling mid-drag re-anchors immediately from the last-known cursor point.
 *
 * Typed VCB (while dragging): a bare number (no unit) is a FACTOR on the
 * driven axis/axes (a corner takes one value driving all three); a length
 * with units ("50mm", "2\"") is a TARGET DIMENSION — `factor = target /
 * currentExtentAlongAxis` (a corner uses the box's diagonal as its "axis"
 * extent, matching the diagonal ratio the drag itself uses).
 *
 * If nothing is selected, the first click auto-selects whatever is under the
 * cursor (via the Viewport-injected selection acquirer) and reveals its
 * gizmo in the same gesture; only a click over empty space shows a hint toast
 * and stays idle.
 *
 * The gizmo box is computed from the selection's rendered mesh positions —
 * TODO: reuse Object Info's world-AABB helper (`panels/objectBounds.ts`)
 * here instead of the local `_selectionBox` walk.
 *
 * The box outline is real world geometry — it stays sized to the actual
 * selection. Grip MARKERS are not: each is held to a small, CONSTANT
 * on-screen size regardless of camera distance or model scale (see
 * `GRIP_SCREEN_PX` / `updateGripScale`), so they read as tidy handles at any
 * zoom on any model, from a few-centimeter part to a house.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { nonUniformScaleAboutPivot, affineToFloat64 } from './transformMath'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { clearPreview } from './transformPreview'
import { commitSelectionTransform, buildSelectionPreview } from './transformSelection'
import { editLengthBuffer, isLengthInputKey, parseDistance } from './moveInput'
import { parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { rayPlaneIntersect } from '../viewport/geoHelpers'
import { axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'
import type { NodeRef } from '../panels/treeModel'
import { collectLeafIds, nodeRefFromJs } from '../panels/treeModel'

export type OnScaleCommit = (nodes: NodeRef[]) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

type Vec3 = [number, number, number]

/** Never let a driven axis reach or cross zero (reflection/singular) — the
 * same floor the kernel's typed refusal exists to keep the UI away from. */
const MIN_SCALE = 0.01

/** A grip on the gizmo box: its world position, the grip that anchors it by
 * default (the geometric opposite), and which axes it drives. `axisIndex`
 * is `null` for a corner (uniform, all three axes via one shared ratio along
 * the pivot→grab diagonal); otherwise it names the single world axis a face
 * grip drives, OR is read alongside `axisIndex2` for a two-axis edge grip. */
interface Grip {
  pos: Vec3
  opposite: Vec3
  /** `null` = corner (uniform). Otherwise the first (or only) driven axis. */
  axisIndex: 0 | 1 | 2 | null
  /** The second driven axis for a two-axis edge grip; `null` for a face or
   * corner grip. */
  axisIndex2: 0 | 1 | 2 | null
}

/**
 * Target on-screen size of a grip marker — full edge length, in CSS pixels —
 * held CONSTANT regardless of camera distance or model scale (see
 * `updateGripScale`). Grips used to be sized as a fraction of the box
 * diagonal, clamped to 2–15 cm of WORLD space: the 2 cm floor dwarfed small
 * models, and a compact rounded shape's 26 always-on-top grips piled into
 * what playtesting called "an explosion of multi-colored cubes that make no
 * logical sense." A small fixed pixel size reads as a handle, not a block,
 * at any zoom or model size.
 */
const GRIP_SCREEN_PX = 9
/** Floor on a grip's rendered WORLD half-size, in meters — only guards
 * against a literal zero/negative scale at a degenerate (zero-height)
 * viewport; far below anything a user would ever perceive. */
const MIN_GRIP_WORLD_HALF = 1e-5
/** Placeholder half-size (meters) a newly-drawn grip renders at for the one
 * frame before the Viewport's render loop first calls `updateGripScale`
 * (mirrors ProtractorTool/SliceTool's identical "placeholder, corrected next
 * frame" convention). Also the fallback used for pick tolerance before that
 * first tick — e.g. in unit tests, which never drive a real render loop. */
const FALLBACK_GRIP_HALF_M = 0.02
/** Pick tolerance around a grip, as a multiple of its rendered on-screen
 * half-size — forgiving enough to grab a small handle without pixel-precise
 * aiming. */
const GRIP_PICK_MULTIPLIER = 3
/** Neutral color for the 8 uniform (corner) grips — face/edge grips are
 * colored by the axis (axes) they drive instead. */
const GIZMO_CORNER_COLOR = 0xdddddd
/** Box outline color and opacity (depth-tested off, like the Rotate
 * protractor, so the gizmo stays visible through the selection's own mesh). */
const GIZMO_OUTLINE_COLOR = 0x999999
const GIZMO_OUTLINE_OPACITY = 0.6

/** Average two 0xRRGGBB colors channel-wise — used to give a 2-axis edge
 * grip a color distinct from either of its driven axes' face grips (see
 * `gripColor`), instead of reading identically to its FIRST axis alone
 * (a known deferred nit: an X-Z edge grip used to look exactly like an X
 * face grip). */
function blendAxisColors(a: number, b: number): number {
  const ar = (a >> 16) & 0xff, ag = (a >> 8) & 0xff, ab = a & 0xff
  const br = (b >> 16) & 0xff, bg = (b >> 8) & 0xff, bb = b & 0xff
  return (Math.round((ar + br) / 2) << 16) | (Math.round((ag + bg) / 2) << 8) | Math.round((ab + bb) / 2)
}

/** A grip's marker color: neutral for a corner (uniform), the driven axis's
 * color for a face (single axis), or a BLEND of both driven axes' colors for
 * an edge — distinct from a same-axis face grip instead of just repeating
 * its first axis, so the three grip kinds read apart by color as well as
 * position. */
function gripColor(grip: Grip, axisColors: readonly number[]): number {
  if (grip.axisIndex === null) return GIZMO_CORNER_COLOR
  if (grip.axisIndex2 === null) return axisColors[grip.axisIndex]
  return blendAxisColors(axisColors[grip.axisIndex], axisColors[grip.axisIndex2])
}

/** The 14 (or 26 with edges) grips of an axis-aligned box `[min, max]`. */
function gripsFromBox(min: Vec3, max: Vec3, includeEdgeGrips: boolean): Grip[] {
  const c: Vec3 = [(min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2]
  const grips: Grip[] = []

  // 6 face-center grips (1D stretch).
  for (const axis of [0, 1, 2] as const) {
    for (const side of [0, 1] as const) {
      const pos: Vec3 = [...c]
      const opposite: Vec3 = [...c]
      pos[axis] = side === 0 ? min[axis] : max[axis]
      opposite[axis] = side === 0 ? max[axis] : min[axis]
      grips.push({ pos, opposite, axisIndex: axis, axisIndex2: null })
    }
  }

  // 8 corner grips (uniform).
  for (const x of [min[0], max[0]]) {
    for (const y of [min[1], max[1]]) {
      for (const z of [min[2], max[2]]) {
        const pos: Vec3 = [x, y, z]
        const opposite: Vec3 = [
          x === min[0] ? max[0] : min[0],
          y === min[1] ? max[1] : min[1],
          z === min[2] ? max[2] : min[2],
        ]
        grips.push({ pos, opposite, axisIndex: null, axisIndex2: null })
      }
    }
  }

  // 12 edge-midpoint grips (2D scale) — optional (DESIGN §1).
  if (includeEdgeGrips) {
    const AXES: readonly (0 | 1 | 2)[] = [0, 1, 2]
    for (const fixed of AXES) {
      const [a, b] = AXES.filter((ax) => ax !== fixed)
      for (const sa of [0, 1] as const) {
        for (const sb of [0, 1] as const) {
          const pos: Vec3 = [...c]
          const opposite: Vec3 = [...c]
          pos[a] = sa === 0 ? min[a] : max[a]
          pos[b] = sb === 0 ? min[b] : max[b]
          opposite[a] = sa === 0 ? max[a] : min[a]
          opposite[b] = sb === 0 ? max[b] : min[b]
          grips.push({ pos, opposite, axisIndex: a, axisIndex2: b })
        }
      }
    }
  }

  return grips
}

/** SHIP the optional 12 edge (2-axis) grips alongside the required 6 face +
 * 8 corner grips. */
const INCLUDE_EDGE_GRIPS = true

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'dragging'
      nodes: NodeRef[]
      grab: Vec3
      opposite: Vec3
      center: Vec3
      axisIndex: 0 | 1 | 2 | null
      axisIndex2: 0 | 1 | 2 | null
      /** Box size (max − min) per axis at grab time — the reference extent
       * for typed target-dimension entry; independent of the anchor. */
      boxExtent: Vec3
      previewMesh: THREE.Object3D | null
      /** Last resolved cursor point, so toggling the anchor (Ctrl) mid-drag
       * can re-render immediately without waiting for the next move. */
      lastCursor: Vec3
    }

export class ScaleTool implements Tool {
  readonly name = 'Scale'

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    if (this.stage.kind === 'dragging') {
      return 'Move to scale, click to commit, or type an exact factor/dimension — Ctrl anchors at the center.'
    }
    if (this.selection.length === 0) {
      return 'Click the object you want to scale.'
    }
    return 'Drag a grip to scale — a face stretches one axis, an edge two, a corner all three. Ctrl anchors at the center.'
  }

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnScaleCommit
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement
  private selection: NodeRef[] = []
  private objectsGroup: THREE.Group | null = null
  private instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null = null
  /** VCB buffer — raw string being typed by the user (bare factor OR a
   * length-with-units target dimension; see `_isBareFactor`). */
  private typed: string = ''
  /** Ctrl's durable anchor-at-center toggle (tap, not hold — matches Move's
   * copy toggle). Off = anchor at the grabbed grip's opposite. */
  private anchorAtCenter: boolean = false

  /** The idle-stage gizmo: box outline + grip markers, redrawn whenever the
   * selection or its geometry changes. Null when nothing is selected. */
  private gizmoGroup: THREE.Group | null = null
  private gizmoGrips: Grip[] | null = null
  /** The rendered grip meshes from the last `_drawGizmo`, parallel to
   * `gizmoGrips` — `updateGripScale` rescales these in place every frame so
   * each one holds a constant on-screen size (see that method's doc). Null
   * when the gizmo isn't showing. */
  private gizmoGripMeshes: THREE.Mesh[] | null = null
  /** The camera's half-vertical-FOV tangent and the canvas height from the
   * most recent `updateGripScale` tick — cached so `_pickGrip` (which only
   * has a ray, not the camera) can reproduce the SAME on-screen size for
   * pick tolerance. Null before the first tick (unit tests never drive a
   * real render loop) — `_pickToleranceAt` falls back to a fixed reasonable
   * tolerance then. */
  private _pickTanHalfFov: number | null = null
  private _pickViewportHeight: number | null = null

  /** Auto-select fallback, injected by the Viewport (see MoveTool's). */
  private acquireSelection: ((ray: Ray) => NodeRef[] | null) | null = null
  setSelectionAcquirer(acquire: ((ray: Ray) => NodeRef[] | null) | null): void {
    this.acquireSelection = acquire
  }
  /** Keep the cached targets in step with the app selection (Tool.
   * setSelection; see MoveTool) — the next gesture starts from live
   * handles after an undo/redo prune, and the gizmo redraws for the new
   * selection while idle. */
  setSelection(nodes: NodeRef[]): void {
    this.selection = nodes
    if (this.stage.kind === 'idle') {
      if (nodes.length > 0) this._showGizmo(nodes)
      else this._hideGizmo()
    }
  }

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    objectsGroup: THREE.Group | null,
    selection: NodeRef[],
    onCommit: OnScaleCommit,
    onToast: OnToast,
    instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null = null,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.objectsGroup = objectsGroup
    this.selection = selection
    this.onCommit = onCommit
    this.onToast = onToast
    this.instanceGroupGetter = instanceGroupGetter
    this.onMeasurementCb = onMeasurement

  }

  /**
   * Draw the idle gizmo when this tool becomes active. Called by
   * `ToolController.setTool` AFTER the outgoing tool's `cancel()` runs — which
   * is essential: the outgoing tool's cancel clears the SHARED preview group
   * (`clearPreview`), so a gizmo drawn in the constructor (before setTool) is
   * wiped by that cancel. Drawing here, as the last step of the switch, is the
   * only point at which the preview group is stable. Feature-detected in
   * `ToolController` via `'activate' in tool`, so no other tool is affected.
   */
  activate(): void {
    if (this.selection.length > 0) this._showGizmo(this.selection)
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (this.stage.kind === 'idle') {
      // Bring the gizmo back on the next pointer move if a prior cancel()
      // cleared it (Esc aborting a drag empties the shared preview group — see
      // cancel()). No-op once it's already showing. A tool SWITCH also runs
      // cancel(), but then this tool no longer receives moves, so the gizmo
      // stays gone as intended.
      if (this.gizmoGroup === null && this.selection.length > 0) {
        this._showGizmo(this.selection)
      }
      return
    }
    const cursor = this._resolveCursor(snap, ray)
    if (cursor === null) return // ray parallel to the grip plane — hold the last cursor
    this.stage.lastCursor = cursor
    this._renderDrag()
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'idle') {
      let nodes = this.selection
      if (nodes.length === 0 && this.acquireSelection !== null) {
        // Empty selection: auto-select whatever the click landed on and
        // reveal its gizmo — this click does not grab a grip (grip
        // positions depend on the box, which didn't exist until now).
        const acquired = this.acquireSelection(ray)
        if (acquired !== null && acquired.length > 0) {
          this.selection = acquired
          nodes = acquired
        }
      }
      if (nodes.length === 0) {
        this.onToast('Click an object to scale it')
        return
      }

      this._showGizmo(nodes)
      if (this.gizmoGrips === null || this._gizmoCenter === null) return // no geometry to scale

      const grip = this._pickGrip(ray, this.gizmoGrips)
      if (grip === null) return // revealed the gizmo; wait for a click ON a grip

      const boxExtent = this._boxExtent()
      if (boxExtent === null) return

      const previewMesh = this._buildPreview(nodes)
      if (previewMesh !== null) this.preview.add(previewMesh)
      const center = this._gizmoCenter
      this._hideGizmo()

      this.stage = {
        kind: 'dragging',
        nodes,
        grab: grip.pos,
        opposite: grip.opposite,
        center,
        axisIndex: grip.axisIndex,
        axisIndex2: grip.axisIndex2,
        boxExtent,
        previewMesh,
        lastCursor: grip.pos,
      }
      this.typed = ''
      this._reportFactors([1, 1, 1])
    } else if (this.stage.kind === 'dragging') {
      // Resolve against THIS click's position, not a stale `lastCursor` from
      // the last move (there may have been none — a click straight after
      // grabbing, with no intervening pointer move, must still land where
      // the user actually clicked). Route through the same axis/plane
      // constraint the drag uses; a parallel-ray null keeps the last cursor.
      const cursor = this._resolveCursor(snap, ray)
      if (cursor !== null) this.stage.lastCursor = cursor
      const { nodes } = this.stage
      const factors = this._currentFactors()
      const pivot = this._pivot()

      this.stage = { kind: 'idle' }
      this.typed = ''
      clearPreview(this.preview)
      this.onMeasurementCb('')
      this._commit(nodes, pivot, factors)
    }
  }

  capturingInput(): boolean {
    return this.stage.kind === 'dragging'
  }

  /**
   * Constrain the drag cursor to the SCALE DIRECTION of the grabbed grip, so a
   * grip that stretches an out-of-ground-plane axis (the +Z "make it taller"
   * grip especially) resolves ALONG that axis instead of collapsing to the
   * ground-plane fallback. The Viewport feeds this into `snapService.resolve`
   * exactly like MoveTool's axis lock (feature-detected via
   * `'snapConstraint' in tool`):
   *
   * - **Face grip → axis LINE.** `lockAxis` = the driven world axis, `anchor`
   *   = the grabbed grip. The inference engine synthesises a point on that
   *   line even in empty space (`closest_point_on_line_to_ray`), so dragging
   *   up genuinely grows the height. Inference still snaps along the line.
   * - **Edge grip → 2-axis PLANE.** `constraintPlane` (normal = the fixed
   *   third axis) FILTERS inference candidates onto the plane. The kernel does
   *   NOT synthesise an empty-space plane point (it only filters), so
   *   `_resolveCursor` projects the ray onto the same plane when nothing
   *   snaps — keeping both driven axes live off the ground plane.
   * - **Corner grip → none.** Uniform-via-diagonal has no single axis/plane;
   *   it resolves against the ground/view plane as before, and the diagonal
   *   ratio projects the cursor onto the pivot→grab direction anyway.
   *
   * Null while idle so the grip-grab click uses ordinary snapping.
   */
  snapConstraint(): {
    anchor?: [number, number, number]
    lockAxis?: 0 | 1 | 2
    constraintPlane?: { point: [number, number, number]; normal: [number, number, number] }
  } | null {
    if (this.stage.kind !== 'dragging') return null
    const { grab, axisIndex, axisIndex2 } = this.stage
    if (axisIndex === null) return null // corner: uniform, no single axis/plane
    if (axisIndex2 === null) {
      return { anchor: grab, lockAxis: axisIndex } // face: lock to the driven axis line
    }
    const normal: [number, number, number] = [0, 0, 0]
    normal[this._fixedAxis(axisIndex, axisIndex2)] = 1
    return { constraintPlane: { point: grab, normal } } // edge: the 2-axis plane
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
      return
    }

    // NB: the Ctrl center-anchor toggle is NOT handled here — a bare Control
    // keydown reports `ctrlKey: true`, so the Viewport's generic key path
    // (gated on `!isMod`) never routes it to a tool's onKey. It arrives via
    // `toggleCenterAnchor()`, driven by a dedicated Ctrl listener in the
    // Viewport (the same reason Shift has its own listener).
    if (this.stage.kind !== 'dragging') return

    if (ev.key === 'Enter') {
      this._commitFromTyped()
      return
    }

    if (isLengthInputKey(ev.key)) {
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
      this._reportTyped()
    }
  }

  /**
   * Flip the durable "anchor at box center" mode (SketchUp's Ctrl). A TAP
   * toggle, not hold: set it before grabbing a grip, or flip it mid-drag to
   * re-anchor live (the drag ghost re-renders immediately). Called by the
   * Viewport's dedicated Ctrl listener on a clean Control tap — see the onKey
   * note above for why this can't ride the generic key path. Autorepeat / the
   * combo-vs-tap distinction are the Viewport listener's concern.
   */
  toggleCenterAnchor(): void {
    this.anchorAtCenter = !this.anchorAtCenter
    if (this.stage.kind === 'dragging') this._renderDrag()
  }

  cancel(): void {
    // Reset to idle and clear EVERYTHING in the shared preview group — the
    // drag ghost AND the idle gizmo (both live there). Crucially does NOT
    // redraw the gizmo: cancel() is also the hook ToolController.setTool()
    // calls on the OUTGOING tool when switching away, so redrawing here would
    // strand the box + grip cubes in the viewport permanently. While the tool
    // stays active (Esc aborting a drag), the gizmo returns on the next
    // pointer move (see onPointerMove) or click.
    this.stage = { kind: 'idle' }
    this.typed = ''
    clearPreview(this.preview)
    // clearPreview disposed + detached the gizmo; drop BOTH stale refs (a
    // stale mesh list would keep updateGripScale rescaling disposed meshes
    // every frame after an idle Esc, until the next pointer move redraws).
    this.gizmoGroup = null
    this.gizmoGripMeshes = null
    this.onMeasurementCb('')
  }

  // ── Private: gizmo geometry & picking ───────────────────────────────────

  /** Cached box center from the last `_showGizmo` — used as the drag pivot
   * when the anchor toggle is on. */
  private _gizmoCenter: Vec3 | null = null
  /** Cached box min/max from the last `_showGizmo` — the source of
   * `boxExtent` at grab time. */
  private _gizmoMin: Vec3 | null = null
  private _gizmoMax: Vec3 | null = null

  private _boxExtent(): Vec3 | null {
    if (this._gizmoMin === null || this._gizmoMax === null) return null
    return [
      this._gizmoMax[0] - this._gizmoMin[0],
      this._gizmoMax[1] - this._gizmoMin[1],
      this._gizmoMax[2] - this._gizmoMin[2],
    ]
  }

  /**
   * World-space bounding box of the whole selection, computed from the
   * rendered meshes — pose-correct for instances (their definition-local
   * geometry is mapped through the instance group's matrix) and free of FFI
   * buffer copies. Free sketches contribute their world-space line
   * endpoints. Null when nothing in the selection has geometry.
   */
  private _selectionBox(nodes: NodeRef[]): THREE.Box3 | null {
    const box = new THREE.Box3()
    const pt = new THREE.Vector3()
    for (const node of nodes) {
      if (node.kind === 'sketch-edge' || node.kind === 'sketch-curve') {
        continue // not transformable — contributes nothing to the box
      }
      if (node.kind === 'sketch-island' && node.sketch !== undefined) {
        const lines = this.wasmScene.sketch_island_lines(node.sketch, node.id)
        for (let i = 0; i + 2 < lines.length; i += 3) {
          box.expandByPoint(pt.set(lines[i], lines[i + 1], lines[i + 2]))
        }
        continue
      }
      if (node.kind === 'sketch') {
        const lines = this.wasmScene.sketch_lines(node.id)
        for (let i = 0; i + 2 < lines.length; i += 3) {
          box.expandByPoint(pt.set(lines[i], lines[i + 1], lines[i + 2]))
        }
      } else if (node.kind === 'instance') {
        const group = this.instanceGroupGetter !== null ? this.instanceGroupGetter(node.id) : null
        if (group !== null) box.expandByObject(group)
      } else if (node.kind === 'group') {
        // A group's leaves are its world objects AND its instances
        // (`node_leaf_objects` stops at instances), so walk the JS tree to
        // gather both — otherwise the scale box excludes grouped instances.
        const { objectIds, instanceIds } = collectLeafIds(node, (groupId) =>
          this.wasmScene.group_members(groupId).map(nodeRefFromJs),
        )
        for (const id of objectIds) {
          const objGroup = this.objectsGroup?.getObjectByName(`Object_${id}`)
          if (objGroup !== undefined) box.expandByObject(objGroup)
        }
        for (const id of instanceIds) {
          const g = this.instanceGroupGetter !== null ? this.instanceGroupGetter(id) : null
          if (g !== null) box.expandByObject(g)
        }
      } else {
        const objGroup = this.objectsGroup?.getObjectByName(`Object_${node.id}`)
        if (objGroup !== undefined) box.expandByObject(objGroup)
      }
    }
    if (box.isEmpty()) return null
    return box
  }

  /** (Re)compute the box + grips for `nodes` and redraw the idle gizmo. */
  private _showGizmo(nodes: NodeRef[]): void {
    this._hideGizmo()
    const box = this._selectionBox(nodes)
    if (box === null) {
      this._gizmoCenter = null
      this._gizmoMin = null
      this._gizmoMax = null
      this.gizmoGrips = null
      return
    }
    const min: Vec3 = [box.min.x, box.min.y, box.min.z]
    const max: Vec3 = [box.max.x, box.max.y, box.max.z]
    this._gizmoMin = min
    this._gizmoMax = max
    this._gizmoCenter = [(min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2]
    this.gizmoGrips = gripsFromBox(min, max, INCLUDE_EDGE_GRIPS)
    this._drawGizmo(min, max, this.gizmoGrips)
  }

  private _drawGizmo(min: Vec3, max: Vec3, grips: Grip[]): void {
    const group = new THREE.Group()
    group.name = 'ScaleGizmo'

    const box3 = new THREE.Box3(
      new THREE.Vector3(min[0], min[1], min[2]),
      new THREE.Vector3(max[0], max[1], max[2]),
    )
    const outline = new THREE.Box3Helper(box3, GIZMO_OUTLINE_COLOR)
    const outlineMat = outline.material as THREE.LineBasicMaterial
    outlineMat.depthTest = false
    outlineMat.transparent = true
    outlineMat.opacity = GIZMO_OUTLINE_OPACITY
    group.add(outline)

    // Grips are drawn as a UNIT cube (local half-extent 1); `updateGripScale`
    // sets each mesh's `.scale` per frame so its WORLD half-size keeps its
    // on-screen size constant (see that method's doc). Start at a small
    // placeholder — corrected on the very next render tick, same convention
    // as ProtractorTool/SliceTool's disk placeholder.
    const geo = new THREE.BoxGeometry(2, 2, 2)
    const axisColors = axisColorsForTheme(getResolvedTheme())
    const meshes: THREE.Mesh[] = []
    for (const grip of grips) {
      const color = gripColor(grip, axisColors)
      // Always-on-top, like the outline: a grip that's grabbable needs to
      // stay visible and clickable even when it sits behind or on the
      // selection's own surface. Depth-testing was reconsidered now that
      // grips are tiny (constant few px) rather than up to 15 cm blocks, but
      // always-on-top is also the conventional behavior for a manipulator
      // handle (SketchUp/Blender/Fusion all keep transform handles visible
      // through geometry) — and enabling depth test risks real z-fighting
      // flicker where a face grip sits exactly on a flat face it shares a
      // depth value with. Small + always-visible beats large + on-top.
      const mat = new THREE.MeshBasicMaterial({ color, depthTest: false })
      const mesh = new THREE.Mesh(geo, mat)
      mesh.position.set(grip.pos[0], grip.pos[1], grip.pos[2])
      mesh.scale.setScalar(FALLBACK_GRIP_HALF_M)
      group.add(mesh)
      meshes.push(mesh)
    }

    this.preview.add(group)
    this.gizmoGroup = group
    this.gizmoGripMeshes = meshes
  }

  private _hideGizmo(): void {
    if (this.gizmoGroup === null) return
    this.gizmoGroup.traverse((child) => {
      if (child instanceof THREE.Mesh || child instanceof THREE.LineSegments) {
        child.geometry.dispose()
        const m = child.material
        if (Array.isArray(m)) m.forEach((mm) => mm.dispose())
        else if (m instanceof THREE.Material) m.dispose()
      }
    })
    this.preview.remove(this.gizmoGroup)
    this.gizmoGroup = null
    this.gizmoGripMeshes = null
  }

  /**
   * Keep every grip marker a small, CONSTANT on-screen size regardless of
   * camera distance or model scale — called once per frame by the Viewport
   * render loop, BEFORE `renderer.render()` (feature-detected via
   * `'updateGripScale' in tool`, the same mechanism RotateTool/
   * ProtractorTool/SliceTool use for their own screen-constant widgets —
   * see `RotateTool.updateDiskScale`). Deliberately NOT a per-mesh
   * `THREE.Object3D.onBeforeRender` hook: that fires too late in three's
   * pipeline (`matrixWorld` is already finalized for the frame from the
   * scene's `updateMatrixWorld()` pass that runs at the START of
   * `renderer.render()`, before per-object callbacks), so a scale set there
   * wouldn't take visual effect until the NEXT frame. Calling this before
   * `renderer.render()` — like `updateDiskScale` already does — means the
   * scene's own `updateMatrixWorld()` picks up the fresh scale with no lag.
   *
   * Each grip's local geometry is a unit cube (half-extent 1); its world
   * half-extent is set to `GRIP_SCREEN_PX` pixels' worth of world space AT
   * ITS OWN DISTANCE from the camera — the standard perspective-projection
   * inverse: `worldHalf = desiredPixels · dist · tan(fov/2) / viewportHeight`.
   * `dist` is the Euclidean camera→grip distance, not view-space depth, so a
   * grip off the view axis by angle θ renders ≈1/cosθ oversized (~8% worst
   * case at 45° fov) — a deliberate approximation, exact on-axis, and
   * self-consistent with `_pickToleranceAt` (render and pick share it, so
   * grips feel exactly as big as they look). Every grip ends up (near) the
   * same apparent size on screen no matter how far it is from the camera or
   * how big the selection's box is. No-op when the gizmo isn't showing or
   * the camera isn't a `PerspectiveCamera` (the only kind this app ever
   * creates — see Viewport.tsx).
   */
  updateGripScale(camera: THREE.Camera, viewportHeight: number): void {
    if (this.gizmoGripMeshes === null || viewportHeight <= 0) return
    if (!(camera instanceof THREE.PerspectiveCamera)) return
    const tanHalfFov = Math.tan((camera.fov * Math.PI) / 360)
    this._pickTanHalfFov = tanHalfFov
    this._pickViewportHeight = viewportHeight
    for (const mesh of this.gizmoGripMeshes) {
      const dist = camera.position.distanceTo(mesh.position)
      const half = Math.max((GRIP_SCREEN_PX * dist * tanHalfFov) / viewportHeight, MIN_GRIP_WORLD_HALF)
      mesh.scale.setScalar(half)
    }
  }

  /** The pick tolerance (world units) at `pos`, matching what `pos` actually
   * renders at on screen right now — reproduces `updateGripScale`'s formula
   * using the camera info it cached last tick, and `ray.origin` as a stand-in
   * for the camera position (the ray originates at the near clip plane, ~1 cm
   * from the eye — negligible next to any real grip distance). Falls back to
   * a fixed placeholder tolerance before the first tick ever runs — in unit
   * tests (jsdom never drives a real render loop), and in one narrow real-app
   * path: the session's FIRST Scale click when it auto-selects and probes the
   * just-revealed gizmo synchronously, before any render tick has cached the
   * camera (safe — the fallback is close to the old fixed clamp). */
  private _pickToleranceAt(ray: Ray, pos: Vec3): number {
    if (this._pickTanHalfFov === null || this._pickViewportHeight === null) {
      return FALLBACK_GRIP_HALF_M * GRIP_PICK_MULTIPLIER
    }
    const dx = pos[0] - ray.origin[0], dy = pos[1] - ray.origin[1], dz = pos[2] - ray.origin[2]
    const dist = Math.sqrt(dx * dx + dy * dy + dz * dz)
    const half = Math.max(
      (GRIP_SCREEN_PX * dist * this._pickTanHalfFov) / this._pickViewportHeight,
      MIN_GRIP_WORLD_HALF,
    )
    return half * GRIP_PICK_MULTIPLIER
  }

  /** Nearest grip to `ray` in units of each grip's OWN pick tolerance, or
   * null when even the best is outside its tolerance. The normalization
   * matters: tolerance scales with a grip's distance from the camera (it
   * tracks the rendered on-screen size), so raw world distance is the wrong
   * nearest-metric across grips at very different depths — a near-camera grip
   * could win the raw contest yet fail its own tight tolerance while a
   * farther grip sat comfortably inside its larger one, swallowing the click.
   * Distance measured in tolerances is (up to the shared multiplier) distance
   * in screen pixels, which is the contest the user is actually playing. */
  private _pickGrip(ray: Ray, grips: Grip[]): Grip | null {
    let best: Grip | null = null
    let bestRatioSq = Infinity
    for (const grip of grips) {
      const d2 = this._distSqToRay(ray, grip.pos)
      const tol = this._pickToleranceAt(ray, grip.pos)
      const ratioSq = d2 / (tol * tol)
      if (ratioSq < bestRatioSq) {
        bestRatioSq = ratioSq
        best = grip
      }
    }
    if (best === null || bestRatioSq > 1) return null
    return best
  }

  /** Squared perpendicular distance from `point` to the ray (clamped to the
   * ray's forward half — a grip "behind" the camera never wins). */
  private _distSqToRay(ray: Ray, point: Vec3): number {
    const [ox, oy, oz] = ray.origin
    const [dx, dy, dz] = ray.direction
    const dLenSq = dx * dx + dy * dy + dz * dz
    const wx = point[0] - ox, wy = point[1] - oy, wz = point[2] - oz
    let t = dLenSq < 1e-18 ? 0 : (wx * dx + wy * dy + wz * dz) / dLenSq
    if (t < 0) t = 0
    const cx = ox + t * dx, cy = oy + t * dy, cz = oz + t * dz
    const ex = point[0] - cx, ey = point[1] - cy, ez = point[2] - cz
    return ex * ex + ey * ey + ez * ez
  }

  // ── Private: drag math ───────────────────────────────────────────────────

  /** The world axis (0/1/2) an edge grip does NOT drive — the third of the
   * three, so its constraint plane's normal. */
  private _fixedAxis(a: 0 | 1 | 2, b: 0 | 1 | 2): 0 | 1 | 2 {
    return (3 - a - b) as 0 | 1 | 2
  }

  /**
   * The drag cursor, constrained to the grabbed grip's scale direction. The
   * Viewport's `snapConstraint` already resolves a FACE grip's snap onto its
   * axis line and filters an EDGE grip's snap onto its plane, so a real
   * (non-ground) snap is used as-is. For an edge grip in EMPTY SPACE the
   * inference falls back to a ground-plane snap (`kind === 'ground'`), which
   * is off the grip's plane — there we project the ray onto the plane
   * ourselves (the kernel's constraint plane only filters candidates, it never
   * synthesises an empty-space point). Returns null only when an edge grip's
   * plane is parallel to the ray (no intersection) — the caller holds the last
   * cursor. A corner grip (no constraint) just takes the snap.
   */
  private _resolveCursor(snap: Snap | null, ray: Ray): Vec3 | null {
    if (this.stage.kind !== 'dragging') return null
    const { grab, axisIndex, axisIndex2 } = this.stage
    // Face grip / corner: the Viewport's snap is already what we want (axis-
    // locked for a face, ground for a corner).
    if (axisIndex === null || axisIndex2 === null) {
      return snap !== null ? [snap.x, snap.y, snap.z] : null
    }
    // Edge grip: a genuine inference snap is constraint-plane-filtered onto the
    // plane already — use it. A ground fallback (or nothing) means empty space:
    // project the ray onto the two-axis plane instead.
    if (snap !== null && snap.kind !== 'ground') {
      return [snap.x, snap.y, snap.z]
    }
    const normal: Vec3 = [0, 0, 0]
    normal[this._fixedAxis(axisIndex, axisIndex2)] = 1
    return rayPlaneIntersect(ray.origin, ray.direction, grab, normal)
  }

  private _pivot(): Vec3 {
    if (this.stage.kind !== 'dragging') return [0, 0, 0]
    return this.anchorAtCenter ? this.stage.center : this.stage.opposite
  }

  /**
   * The per-axis drag ratio `(cursor − pivot)·e / (grab − pivot)·e` for a
   * single world axis `axis` — exactly DESIGN's `s_axis` formula. Because a
   * face and edge grip's driven axes are world-aligned and independent, each
   * driven axis reads only its own component: a face grip drives one axis, an
   * edge grip drives its two INDEPENDENTLY (SketchUp's edge grips scale their
   * two axes independently; Shift-to-lock-proportional is out of scope, as is
   * comma-separated per-axis VCB). Clamped to `MIN_SCALE` — dragging past the
   * anchor clamps, never reflects.
   */
  private _perAxisRatio(pivot: Vec3, grab: Vec3, cursor: Vec3, axis: 0 | 1 | 2): number {
    const denom = grab[axis] - pivot[axis]
    if (Math.abs(denom) < 1e-9) return 1 // zero-extent box on this axis — no-op
    return Math.max((cursor[axis] - pivot[axis]) / denom, MIN_SCALE)
  }

  /**
   * The uniform ratio a CORNER grip resolves to: one shared scalar along the
   * pivot→grab 3D diagonal, applied to all three axes, which is what keeps a
   * corner drag proportional (SketchUp's corner default). Clamped to
   * `MIN_SCALE`.
   */
  private _diagonalRatio(pivot: Vec3, grab: Vec3, cursor: Vec3): number {
    const dx = grab[0] - pivot[0], dy = grab[1] - pivot[1], dz = grab[2] - pivot[2]
    const len = Math.sqrt(dx * dx + dy * dy + dz * dz)
    if (len < 1e-9) return 1 // degenerate (zero-extent) box — no-op
    const numer = (cursor[0] - pivot[0]) * dx + (cursor[1] - pivot[1]) * dy + (cursor[2] - pivot[2]) * dz
    return Math.max(numer / (len * len), MIN_SCALE)
  }

  /** [sx,sy,sz] for driving `axisIndex`/`axisIndex2` (or all three, for a
   * corner) by ONE shared scalar `s` — the typed-entry path (a single typed
   * value drives every axis a grip owns proportionally; a corner uses it
   * uniformly). The DRAG path builds its factors per-axis instead (see
   * `_currentFactors`), so an edge grip's two axes move independently on drag
   * but proportionally on a single typed value. */
  private _factorsForScalar(axisIndex: 0 | 1 | 2 | null, axisIndex2: 0 | 1 | 2 | null, s: number): Vec3 {
    if (axisIndex === null) return [s, s, s]
    const f: Vec3 = [1, 1, 1]
    f[axisIndex] = s
    if (axisIndex2 !== null) f[axisIndex2] = s
    return f
  }

  /** Live [sx,sy,sz] for the current drag: the typed buffer wins once it
   * parses to a valid factor/dimension; otherwise the last resolved cursor
   * position drives the ratio. Face/edge grips scale each driven axis
   * INDEPENDENTLY (per-axis ratio); a corner grip scales all three by one
   * uniform diagonal ratio. */
  private _currentFactors(): Vec3 {
    if (this.stage.kind !== 'dragging') return [1, 1, 1]
    const typedFactors = this._parseTypedFactors()
    if (typedFactors !== null) return typedFactors

    const { grab, axisIndex, axisIndex2, lastCursor } = this.stage
    const pivot = this._pivot()

    if (axisIndex === null) {
      // Corner: one uniform ratio along the pivot→grab diagonal.
      const s = this._diagonalRatio(pivot, grab, lastCursor)
      return [s, s, s]
    }
    // Face (one axis) or edge (two axes), each driven axis independent.
    const f: Vec3 = [1, 1, 1]
    f[axisIndex] = this._perAxisRatio(pivot, grab, lastCursor, axisIndex)
    if (axisIndex2 !== null) {
      f[axisIndex2] = this._perAxisRatio(pivot, grab, lastCursor, axisIndex2)
    }
    return f
  }

  /** A typed buffer with no length-grammar characters at all (only digits,
   * `.`, a leading `-`) is a bare FACTOR; anything else (unit letters, `'`,
   * `"`, `/`, a space) is a length — a TARGET DIMENSION. */
  private _isBareFactor(buf: string): boolean {
    return /^[-0-9.]*$/.test(buf)
  }

  /** Parse `this.typed` into [sx,sy,sz], or null while it's empty/incomplete
   * (the caller falls back to the live drag ratio, or — on Enter — treats
   * null as "nothing to commit"). */
  private _parseTypedFactors(): Vec3 | null {
    if (this.stage.kind !== 'dragging' || this.typed === '') return null
    const { axisIndex, axisIndex2, boxExtent } = this.stage

    if (this._isBareFactor(this.typed)) {
      const n = parseDistance(this.typed)
      if (n === null || n <= 0) return null
      return this._factorsForScalar(axisIndex, axisIndex2, Math.max(n, MIN_SCALE))
    }

    const meters = parseLengthToMeters(this.typed, getLengthUnit())
    if (meters === null || meters <= 0) return null
    const extent = this._referenceExtent(axisIndex, axisIndex2, boxExtent)
    if (extent < 1e-9) return null
    return this._factorsForScalar(axisIndex, axisIndex2, Math.max(meters / extent, MIN_SCALE))
  }

  /** The extent a typed target dimension divides against: a face grip's
   * single driven axis, a corner's full box diagonal, or — for an edge grip,
   * whose single typed value scales BOTH axes proportionally (comma-separated
   * per-axis VCB is out of scope) — the two axes' 2D diagonal, so "type 50mm"
   * on an edge grip means "make this face's diagonal 50mm". A face or corner
   * value targets the same |grab − pivot| distance its live drag already
   * measures, so typed and dragged entry agree for those. */
  private _referenceExtent(axisIndex: 0 | 1 | 2 | null, axisIndex2: 0 | 1 | 2 | null, boxExtent: Vec3): number {
    if (axisIndex === null) {
      return Math.sqrt(boxExtent[0] ** 2 + boxExtent[1] ** 2 + boxExtent[2] ** 2)
    }
    if (axisIndex2 === null) return boxExtent[axisIndex]
    return Math.sqrt(boxExtent[axisIndex] ** 2 + boxExtent[axisIndex2] ** 2)
  }

  // ── Private: preview, commit, VCB readout ──────────────────────────────

  private _renderDrag(): void {
    if (this.stage.kind !== 'dragging') return
    const factors = this._currentFactors()
    if (this.stage.previewMesh !== null) {
      this._applyPreviewScale(this.stage.previewMesh, this._pivot(), factors)
    }
    if (this.typed === '') this._reportFactors(factors)
  }

  private _buildPreview(nodes: NodeRef[]): THREE.Object3D | null {
    return buildSelectionPreview(this.wasmScene, this.objectsGroup, this.instanceGroupGetter, nodes)
  }

  private _applyPreviewScale(mesh: THREE.Object3D, pivot: Vec3, factors: Vec3): void {
    // Reset accumulated transform, then apply the new one
    mesh.position.set(0, 0, 0)
    mesh.rotation.set(0, 0, 0)
    mesh.scale.set(1, 1, 1)
    mesh.updateMatrix()

    const affine = nonUniformScaleAboutPivot(factors[0], factors[1], factors[2], pivot)
    const m4 = new THREE.Matrix4()
    m4.set(
      affine[0], affine[1], affine[2], affine[3],
      affine[4], affine[5], affine[6], affine[7],
      affine[8], affine[9], affine[10], affine[11],
      0, 0, 0, 1,
    )
    mesh.applyMatrix4(m4)
  }

  private _reportFactors(factors: Vec3): void {
    if (this.stage.kind !== 'dragging') return
    const { axisIndex, axisIndex2 } = this.stage
    if (axisIndex === null) {
      this.onMeasurementCb(`×${factors[0].toFixed(2)}`)
      return
    }
    const label = ['X', 'Y', 'Z']
    if (axisIndex2 === null) {
      this.onMeasurementCb(`${label[axisIndex]} ×${factors[axisIndex].toFixed(2)}`)
      return
    }
    // Edge grip: two independently-driven axes, so show both factors.
    this.onMeasurementCb(
      `${label[axisIndex]} ×${factors[axisIndex].toFixed(2)}  ${label[axisIndex2]} ×${factors[axisIndex2].toFixed(2)}`,
    )
  }

  private _reportTyped(): void {
    this.onMeasurementCb(
      this._isBareFactor(this.typed) ? `×${this.typed}` : typedReadout(this.typed, getLengthUnit()),
    )
  }

  /** Commit the scale from the typed VCB buffer, then reset to idle. A typed
   * buffer that doesn't yet parse to a positive factor/dimension is a no-op
   * (Enter is ignored, matching Move/Rotate's incomplete-entry behavior). */
  private _commitFromTyped(): void {
    if (this.stage.kind !== 'dragging') return
    const factors = this._parseTypedFactors()
    if (factors === null) return
    const { nodes } = this.stage
    const pivot = this._pivot()

    this.stage = { kind: 'idle' }
    this.typed = ''
    clearPreview(this.preview)
    this.onMeasurementCb('')
    this._commit(nodes, pivot, factors)
  }

  private _commit(nodes: NodeRef[], pivot: Vec3, factors: Vec3): void {
    const isIdentity =
      Math.abs(factors[0] - 1) < 1e-9 && Math.abs(factors[1] - 1) < 1e-9 && Math.abs(factors[2] - 1) < 1e-9
    if (!isIdentity) {
      try {
        const affine = nonUniformScaleAboutPivot(factors[0], factors[1], factors[2], pivot)
        const affineF64 = affineToFloat64(affine)
        commitSelectionTransform(this.wasmScene, nodes, affineF64)
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        if (nodes.length > 0) this._showGizmo(nodes)
        return
      }
    }
    this.onCommit(nodes)
    // Redraw the gizmo at the (possibly new) size so a follow-up grip can be
    // grabbed right away — handles stay stable through a transform (kernel
    // strong guarantee), so `nodes` is still valid.
    if (nodes.length > 0) this._showGizmo(nodes)
  }
}
