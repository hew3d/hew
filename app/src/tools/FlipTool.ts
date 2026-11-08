/**
 * FlipTool — mirror the selection across a plane, in place or as a copy.
 *
 * The gizmo is three translucent squares through the center of the
 * selection's bounding box, one per world axis, each colored by the axis
 * that is its normal (red for the plane across the red axis, and so on) and
 * sized to the box. Hovering a square highlights it and shows a ghost of the
 * mirrored selection; clicking commits. Hovering a face of the model (one
 * that is not part of the selection) mirrors across that face's plane
 * instead, with the same ghost and click.
 *
 * The copy modifier (tap Option on macOS, Ctrl elsewhere) cycles plain Flip,
 * Copy (the preferred kind), and the other kind of copy, as Move and Rotate
 * do (`copyMode.ts`). A copy is a mirrored copy beside the original: a
 * linked copy is another placement of a shared shape with a mirrored pose.
 *
 * With nothing selected the first click selects what is under the cursor
 * (as Move does) and shows the gizmo. Escape drops the hover ghost.
 *
 * Every flip is one undo step, labeled "Flip". Objects and drawing bake the
 * mirror; component instances take a mirrored pose (`flip_selection`).
 */

import { t } from '@lingui/core/macro'
import * as THREE from 'three'
import type { Tool, Snap, EditContext } from './types'
import { editContextEq } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { resolveDraft } from './planeUnits'
import { clearPreview } from './transformPreview'
import {
  buildSelectionPreview,
  commitSelectionTransform,
  copyOwnersAndDrawing,
  dropRidingDrafts,
  inHistoryBracket,
} from './transformSelection'
import { reflectionAffine, type Affine } from './transformMath'
import { axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'
import { getCopiesLinked } from '../settings/editing'
import { nextCopyState, linkedFor, type CopyState, type OnCopyModeChange } from './copyMode'
import { worldFacePlane } from './faceDraw'
import { isMac, COPY_MODIFIER_LABEL } from '../platform'
import type { NodeRef } from '../panels/treeModel'
import { collectLeafIds, isDraftKind, isSketchScopedKind, nodeKindToNumber, nodeRefFromJs } from '../panels/treeModel'
import { worldBoundsForSelection, unionBounds, type Bounds } from '../panels/objectBounds'

type V3 = [number, number, number]

/** The mirror plane a flip uses: a world-axis plane through the selection's
 *  center (`axis` names its normal), or the plane of a picked face. */
export type FlipPlane = { kind: 'axis'; axis: 0 | 1 | 2 } | { kind: 'face'; point: V3; normal: V3 }

/** How a flip lands: in place, or as a linked or independent mirrored copy. */
export type FlipCopy = 'none' | 'linked' | 'independent'

/** Called after a flip lands: the nodes to select (the flipped nodes or the
 *  copies), and whether the whole scene needs rebuilding (a linked copy
 *  wraps its loose originals, which removes world nodes). */
export type OnFlipCommit = (nodes: NodeRef[], fullRefresh: boolean) => void
export type OnToast = (message: string, code?: string) => void

/** Square opacity at rest and under the cursor. */
const SQUARE_OPACITY = 0.18
const SQUARE_HOVER_OPACITY = 0.45
/** Each square overhangs the box by this fraction of its side, so the three
 *  read as planes through the part rather than as its faces. */
const SQUARE_MARGIN = 0.15
/** Smallest square side, as a fraction of the box's largest extent (a flat
 *  drawing has no height, but its upright squares still need a target). */
const SQUARE_MIN_FRACTION = 0.25

const AXIS_UNIT: readonly V3[] = [
  [1, 0, 0],
  [0, 1, 0],
  [0, 0, 1],
]

/** The world bounding box of a selection: the solids (`objectBounds`), the
 *  sketches' lines, and drawn shapes' edges. Members of an open component
 *  are mapped through its placement. `null` when nothing has geometry. */
export function flipSelectionBounds(
  wasmScene: WasmScene,
  selection: readonly NodeRef[],
  activeInstance: bigint | null = null,
): Bounds | null {
  const pose = activeInstance !== null ? wasmScene.instance_pose(activeInstance) : undefined
  const mapPoint = (p: V3): V3 => {
    if (pose === undefined) return p
    return [
      pose[0] * p[0] + pose[1] * p[1] + pose[2] * p[2] + pose[3],
      pose[4] * p[0] + pose[5] * p[1] + pose[6] * p[2] + pose[7],
      pose[8] * p[0] + pose[9] * p[1] + pose[10] * p[2] + pose[11],
    ]
  }
  let box: Bounds | null = null
  const addPoint = (p: V3): void => {
    box = unionBounds(box, { min: p, max: p })
  }
  const owners: NodeRef[] = []
  for (const node of selection) {
    if (isDraftKind(node.kind)) {
      // Drawn shapes report world coordinates already.
      const r = resolveDraft(wasmScene, node)
      if (r === null || r.unit === null) continue
      for (const e of r.unit.edges) {
        addPoint([e.a[0], e.a[1], e.a[2]])
        addPoint([e.b[0], e.b[1], e.b[2]])
      }
    } else if (node.kind === 'sketch') {
      const lines = wasmScene.sketch_lines(node.id)
      for (let i = 0; i + 2 < lines.length; i += 3) addPoint(mapPoint([lines[i], lines[i + 1], lines[i + 2]]))
    } else if (node.kind === 'object' && pose !== undefined) {
      try {
        const b = wasmScene.object_bounds(node.id, Float64Array.from(pose))
        if (b.length >= 6) box = unionBounds(box, { min: [b[0], b[1], b[2]], max: [b[3], b[4], b[5]] })
      } catch {
        // A stale member contributes nothing.
      }
    } else if (node.kind === 'object' || node.kind === 'group' || node.kind === 'instance') {
      owners.push(node)
    }
  }
  if (owners.length > 0) box = unionBounds(box, worldBoundsForSelection(wasmScene, owners))
  return box
}

/** The center of a bounds box. */
function centerOf(b: Bounds): V3 {
  return [(b.min[0] + b.max[0]) / 2, (b.min[1] + b.max[1]) / 2, (b.min[2] + b.max[2]) / 2]
}

/** The point and normal of `plane`, with `center` for an axis plane. */
export function planePointNormal(plane: FlipPlane, center: V3): { point: V3; normal: V3 } {
  if (plane.kind === 'axis') return { point: center, normal: [...AXIS_UNIT[plane.axis]] as V3 }
  return { point: plane.point, normal: plane.normal }
}

/**
 * Mirror `selection` across the plane through `point` with normal `normal`,
 * as ONE undo step labeled "Flip": in place (`copy` 'none') or as a linked
 * or independent mirrored copy. Returns the nodes to select afterwards (the
 * flipped nodes, re-found where drawing moved, or the copies).
 *
 * The solids go through `flip_selection`; drawing in the selection moves or
 * copies through the same paths Move and Rotate use, with the matrix
 * `flip_matrix` reports, so a board and the lines drawn beside it mirror
 * together. Inside an open component the members flip through the component
 * (`commitSelectionTransform`). Kernel refusals throw, with the document
 * unchanged.
 */
export function runFlip(
  wasmScene: WasmScene,
  selection: readonly NodeRef[],
  point: V3,
  normal: V3,
  copy: FlipCopy,
  activeInstance: bigint | null = null,
): NodeRef[] {
  const label = t({ context: 'undo verb', message: 'Flip' })
  // A copy lands in the world; members of an open component do not.
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- kernel error code identifier, mapped to copy by kernelErrors
  if (copy !== 'none' && activeInstance !== null) throw new Error('ExplodeSessionScope: copy inside a component')
  return inHistoryBracket(wasmScene, label, () => {
    const nodes = copy === 'none' ? [...selection] : dropRidingDrafts(wasmScene, selection)
    const owners = nodes.filter((n) => n.kind === 'object' || n.kind === 'group' || n.kind === 'instance')
    const drawn = nodes.filter((n) => isSketchScopedKind(n.kind))
    const kinds = new Uint8Array(owners.map((n) => nodeKindToNumber(n.kind)))
    const ids = new BigUint64Array(owners.map((n) => n.id))
    const sketchIds = new BigUint64Array(nodes.filter((n) => n.kind === 'sketch').map((n) => n.id))
    // about_kind 2: the plane through `point`.
    const matrix =
      activeInstance === null && (owners.length > 0 || sketchIds.length > 0)
        ? wasmScene.flip_matrix(kinds, ids, sketchIds, normal[0], normal[1], normal[2], 2, point[0], point[1], point[2])
        : localMatrix(point, normal)
    const flip = (mode: number): NodeRef[] =>
      wasmScene
        .flip_selection(kinds, ids, new BigUint64Array(0), normal[0], normal[1], normal[2], 2, point[0], point[1], point[2], mode)
        .map(nodeRefFromJs)
    if (copy === 'none') {
      if (activeInstance !== null || drawn.length > 0) {
        return commitSelectionTransform(wasmScene, nodes, matrix, activeInstance, label)
      }
      flip(0)
      return nodes
    }
    const mode = copy === 'linked' ? 1 : 2
    return copyOwnersAndDrawing(wasmScene, owners, drawn, [matrix], () => flip(mode))
  })
}

/**
 * Object ▸ Flip Along: mirror `nodes` in place across the plane through
 * their bounding box center whose normal is world axis `axis`. Returns the
 * flipped nodes, or `null` when nothing in `nodes` has geometry to flip
 * (the caller says so). Kernel refusals throw, document unchanged.
 */
export function flipAlongAxis(
  wasmScene: WasmScene,
  nodes: readonly NodeRef[],
  axis: 0 | 1 | 2,
  activeInstance: bigint | null = null,
): NodeRef[] | null {
  const box = flipSelectionBounds(wasmScene, nodes, activeInstance)
  if (box === null) return null
  const { point, normal } = planePointNormal({ kind: 'axis', axis }, centerOf(box))
  return runFlip(wasmScene, nodes, point, normal, 'none', activeInstance)
}

function localMatrix(point: V3, normal: V3): Float64Array {
  const m: Affine | null = reflectionAffine(point, normal)
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- kernel error code identifier, mapped to copy by kernelErrors
  if (m === null) throw new Error('Singular: transform is singular')
  return new Float64Array(m)
}

/** What the cursor is over: a gizmo square, or a model face's plane. */
type Hover = { plane: FlipPlane; ghost: THREE.Object3D | null } | null

export class FlipTool implements Tool {
  readonly name = 'Flip'

  statusHint(): string {
    if (this.selection.length === 0) return t`Click what you want to flip.`
    const kind =
      this.copyState === 'off'
        ? null
        : linkedFor(this.copyState, getCopiesLinked())
          ? t`Linked copy`
          : t`Independent copy`
    if (kind !== null) {
      return t`${kind} is on. Click a square or a face to place a mirrored copy. Tap ${COPY_MODIFIER_LABEL} to change.`
    }
    return t`Click a square to flip across that plane, or click a face to flip across it. Tap ${COPY_MODIFIER_LABEL} to make a mirrored copy.`
  }

  private preview: THREE.Group
  private wasmScene: WasmScene
  private objectsGroup: THREE.Group | null
  private instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null
  private onCommit: OnFlipCommit
  private onToast: OnToast
  private onCopyModeChange: OnCopyModeChange
  private selection: NodeRef[] = []
  private copyState: CopyState = 'off'
  private hover: Hover = null

  /** The squares drawn for the current selection, by axis. */
  private gizmoGroup: THREE.Group | null = null
  private squares: { axis: 0 | 1 | 2; mesh: THREE.Mesh; half: [number, number] }[] = []
  private center: V3 | null = null

  private acquireSelection: ((ray: Ray) => NodeRef[] | null) | null = null
  setSelectionAcquirer(acquire: ((ray: Ray) => NodeRef[] | null) | null): void {
    this.acquireSelection = acquire
  }

  private _editContext: EditContext = { kind: 'top' }
  setEditContext(ctx: EditContext): void {
    if (editContextEq(ctx, this._editContext)) return
    this._editContext = ctx
    this.cancel()
  }
  private get _activeInstance(): bigint | null {
    return this._editContext.kind === 'instance' ? this._editContext.id : null
  }

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    objectsGroup: THREE.Group | null,
    selection: NodeRef[],
    onCommit: OnFlipCommit,
    onToast: OnToast,
    instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null = null,
    onCopyModeChange: OnCopyModeChange = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.objectsGroup = objectsGroup
    this.selection = selection
    this.onCommit = onCommit
    this.onToast = onToast
    this.instanceGroupGetter = instanceGroupGetter
    this.onCopyModeChange = onCopyModeChange
  }

  /** Draw the gizmo once the tool switch has cleared the shared preview
   *  group (see ScaleTool.activate). */
  activate(): void {
    if (this.selection.length > 0) this._showGizmo()
  }

  setSelection(nodes: NodeRef[]): void {
    this.selection = nodes
    this._clearHover()
    if (nodes.length > 0) this._showGizmo()
    else this._hideGizmo()
  }

  /** Advance the copy cycle (Flip, Copy, the other kind of copy, Flip). */
  toggleCopyMode(): void {
    this.copyState = nextCopyState(this.copyState)
    this.onCopyModeChange(this.copyState, linkedFor(this.copyState, getCopiesLinked()))
  }

  /** The current copy state, for the host to badge the cursor. */
  currentCopyState(): CopyState {
    return this.copyState
  }

  /** The plane under the cursor right now, if any (for tests and the host). */
  hoveredPlane(): FlipPlane | null {
    return this.hover?.plane ?? null
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    if (this.selection.length === 0) {
      this._clearHover()
      return
    }
    if (this.gizmoGroup === null) this._showGizmo()
    const plane = this._planeUnder(snap, ray)
    if (samePlane(plane, this.hover?.plane ?? null)) return
    this._clearHover()
    if (plane === null) return
    this.hover = { plane, ghost: this._buildGhost(plane) }
    this._paintSquares()
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (this.selection.length === 0) {
      const acquired = this.acquireSelection !== null ? this.acquireSelection(ray) : null
      if (acquired !== null && acquired.length > 0) {
        // The acquirer selects what was picked in the host; the flip waits
        // for the next click.
        this.selection = acquired
        this._showGizmo()
      } else {
        this.onToast(t`Click an object to flip it`)
      }
      return
    }
    if (this.gizmoGroup === null) this._showGizmo()
    const plane = this._planeUnder(snap, ray)
    if (plane === null) return
    this.commit(plane)
  }

  /** Flip the selection across `plane` with the current copy state. */
  commit(plane: FlipPlane): boolean {
    if (this.center === null) this._showGizmo()
    const center = this.center
    if (center === null) return false
    const { point, normal } = planePointNormal(plane, center)
    const copy: FlipCopy =
      this.copyState === 'off' ? 'none' : linkedFor(this.copyState, getCopiesLinked()) ? 'linked' : 'independent'
    if (copy !== 'none' && this._activeInstance !== null) {
      // A copy lands in the world, but the members of an open component
      // live in its own space: refuse it plainly rather than copy wrongly.
      this.onToast(t`A mirrored copy can't be made while you're editing inside a component. Close the component first, or flip in place.`)
      return false
    }
    this._clearHover()
    clearPreview(this.preview)
    this.gizmoGroup = null
    this.squares = []
    let committed: NodeRef[]
    try {
      committed = runFlip(this.wasmScene, this.selection, point, normal, copy, this._activeInstance)
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
      this._showGizmo()
      return false
    }
    this.selection = committed
    this.onCommit(committed, copy === 'linked')
    this._showGizmo()
    return true
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this._clearHover()
      this._paintSquares()
      return
    }
    // macOS taps Option for the copy cycle; Windows and Linux tap Ctrl,
    // which the Viewport routes to `toggleCopyMode` (see MoveTool.onKey).
    if (isMac && ev.key === 'Alt') {
      if (!ev.repeat) {
        ev.preventDefault()
        this.toggleCopyMode()
      }
    }
  }

  /** Escape belongs to the tool while a ghost is showing. */
  hasArmedGesture(): boolean {
    return this.hover !== null
  }

  cancel(): void {
    this.hover = null
    clearPreview(this.preview)
    this.gizmoGroup = null
    this.squares = []
  }

  // ── Private ──────────────────────────────────────────────────────────

  /** The square under the ray (nearest first), else a model face's plane. */
  private _planeUnder(snap: Snap | null, ray: Ray): FlipPlane | null {
    const axis = this._pickSquare(ray)
    if (axis !== null) return { kind: 'axis', axis }
    return this._facePlane(snap)
  }

  private _pickSquare(ray: Ray): 0 | 1 | 2 | null {
    if (this.center === null) return null
    const c = this.center
    let best: { axis: 0 | 1 | 2; t: number } | null = null
    for (const sq of this.squares) {
      const a = sq.axis
      const d = ray.direction[a]
      if (Math.abs(d) < 1e-12) continue
      const tHit = (c[a] - ray.origin[a]) / d
      if (tHit <= 0) continue
      const [u, v] = ([0, 1, 2] as const).filter((k) => k !== a)
      const pu = ray.origin[u] + tHit * ray.direction[u] - c[u]
      const pv = ray.origin[v] + tHit * ray.direction[v] - c[v]
      if (Math.abs(pu) > sq.half[0] || Math.abs(pv) > sq.half[1]) continue
      if (best === null || tHit < best.t) best = { axis: a, t: tHit }
    }
    return best?.axis ?? null
  }

  /** The plane of the model face under the cursor, unless that face belongs
   *  to the selection itself. */
  private _facePlane(snap: Snap | null): FlipPlane | null {
    if (snap === null || snap.elementKind !== 'face' || snap.object === undefined || snap.element === undefined) {
      return null
    }
    if (this._inSelection(snap.object, snap.instance)) return null
    const plane = worldFacePlane(this.wasmScene, snap.object, snap.element, snap.instance ?? null)
    if (plane === null) return null
    return { kind: 'face', point: plane.point, normal: plane.normal }
  }

  private _inSelection(object: bigint, instance: bigint | undefined): boolean {
    // Inside an open component a selected member's faces report the
    // placement being edited as their instance; they are still its own.
    const own = instance === undefined || instance === this._activeInstance
    for (const node of this.selection) {
      if (node.kind === 'object' && node.id === object && own) return true
      if (node.kind === 'instance' && instance !== undefined && node.id === instance) return true
      if (node.kind === 'group') {
        const { objectIds, instanceIds } = collectLeafIds(node, (g) =>
          this.wasmScene.group_members(g).map(nodeRefFromJs),
        )
        if (instance === undefined && objectIds.includes(object)) return true
        if (instance !== undefined && instanceIds.includes(instance)) return true
      }
    }
    return false
  }

  private _showGizmo(): void {
    this._hideGizmo()
    const box = flipSelectionBounds(this.wasmScene, this.selection, this._activeInstance)
    if (box === null) {
      this.center = null
      return
    }
    const b: Bounds = box
    this.center = centerOf(b)
    const ext: V3 = [b.max[0] - b.min[0], b.max[1] - b.min[1], b.max[2] - b.min[2]]
    const largest = Math.max(ext[0], ext[1], ext[2], 1e-3)
    const side = (e: number): number => Math.max(e, largest * SQUARE_MIN_FRACTION) * (1 + SQUARE_MARGIN)
    const colors = axisColorsForTheme(getResolvedTheme())
    const group = new THREE.Group()
    group.name = 'FlipGizmo'
    this.squares = []
    for (const axis of [0, 1, 2] as const) {
      const [u, v] = ([0, 1, 2] as const).filter((k) => k !== axis)
      const w = side(ext[u])
      const h = side(ext[v])
      const geo = new THREE.PlaneGeometry(w, h)
      const mat = new THREE.MeshBasicMaterial({
        color: colors[axis],
        transparent: true,
        opacity: SQUARE_OPACITY,
        side: THREE.DoubleSide,
        depthTest: false,
        depthWrite: false,
      })
      const mesh = new THREE.Mesh(geo, mat)
      mesh.name = `FlipSquare_${axis}`
      // PlaneGeometry lies in its local XY plane: turn it so its width runs
      // along `u`, its height along `v`, and its normal along `axis`.
      const basis = new THREE.Matrix4().makeBasis(
        new THREE.Vector3(...AXIS_UNIT[u]),
        new THREE.Vector3(...AXIS_UNIT[v]),
        new THREE.Vector3(...AXIS_UNIT[axis]),
      )
      mesh.quaternion.setFromRotationMatrix(basis)
      mesh.position.set(this.center[0], this.center[1], this.center[2])
      mesh.renderOrder = 10
      group.add(mesh)
      this.squares.push({ axis, mesh, half: [w / 2, h / 2] })
    }
    this.preview.add(group)
    this.gizmoGroup = group
    this._paintSquares()
  }

  private _hideGizmo(): void {
    if (this.gizmoGroup === null) return
    this.gizmoGroup.traverse((child) => {
      if (child instanceof THREE.Mesh) {
        child.geometry.dispose()
        const m = child.material
        if (Array.isArray(m)) m.forEach((mm) => mm.dispose())
        else if (m instanceof THREE.Material) m.dispose()
      }
    })
    this.preview.remove(this.gizmoGroup)
    this.gizmoGroup = null
    this.squares = []
  }

  /** Highlight the hovered square; the rest stay faint. */
  private _paintSquares(): void {
    const hot = this.hover?.plane.kind === 'axis' ? this.hover.plane.axis : null
    for (const sq of this.squares) {
      const mat = sq.mesh.material as THREE.MeshBasicMaterial
      mat.opacity = sq.axis === hot ? SQUARE_HOVER_OPACITY : SQUARE_OPACITY
    }
  }

  private _clearHover(): void {
    const ghost = this.hover?.ghost ?? null
    this.hover = null
    if (ghost !== null) {
      this.preview.remove(ghost)
      // The ghost owns cloned geometry and materials (transformPreview), so
      // dispose both, as clearPreview does.
      ghost.traverse((child) => {
        if (child instanceof THREE.Mesh || child instanceof THREE.LineSegments) {
          child.geometry.dispose()
          const m = child.material
          if (Array.isArray(m)) m.forEach((mm) => mm.dispose())
          else if (m instanceof THREE.Material) m.dispose()
        }
      })
    }
    this._paintSquares()
  }

  /** A ghost of the selection mirrored across `plane`. */
  private _buildGhost(plane: FlipPlane): THREE.Object3D | null {
    if (this.center === null) return null
    const { point, normal } = planePointNormal(plane, this.center)
    const m = reflectionAffine(point, normal)
    if (m === null) return null
    const ghost = buildSelectionPreview(
      this.wasmScene,
      this.objectsGroup,
      this.instanceGroupGetter,
      this.selection,
      this._activeInstance,
    )
    if (ghost === null) return null
    ghost.name = 'FlipGhost'
    const m4 = new THREE.Matrix4().set(
      m[0], m[1], m[2], m[3],
      m[4], m[5], m[6], m[7],
      m[8], m[9], m[10], m[11],
      0, 0, 0, 1,
    )
    ghost.applyMatrix4(m4)
    this.preview.add(ghost)
    return ghost
  }
}

function samePlane(a: FlipPlane | null, b: FlipPlane | null): boolean {
  if (a === null || b === null) return a === b
  if (a.kind === 'axis' && b.kind === 'axis') return a.axis === b.axis
  if (a.kind === 'face' && b.kind === 'face') {
    // The same plane through any of its points: same normal, same offset.
    const dot = (p: V3, n: V3): number => p[0] * n[0] + p[1] * n[1] + p[2] * n[2]
    return (
      a.normal.every((x, i) => Math.abs(x - b.normal[i]) < 1e-9) &&
      Math.abs(dot(a.point, a.normal) - dot(b.point, a.normal)) < 1e-9
    )
  }
  return false
}
