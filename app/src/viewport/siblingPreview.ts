/**
 * Live drag preview on sibling placements (ARCHITECTURE.md 2.11).
 *
 * Inside a component session every other placement of the definition
 * shows a committed edit through its own pose. The gesture BEFORE the
 * commit — Push/Pull's swept prism, a Move/Rotate/Scale ghost, Offset's
 * loops — used to show only on the copy being edited, so the other copies
 * sat still until the click. SketchUp mirrors the preview on every copy
 * during the drag; this does the same, without teaching any tool about
 * siblings.
 *
 * The mirror watches the one preview group every tool draws into. Each
 * object a builder marked as session geometry (`markSessionGhost` in
 * tools/transformPreview.ts — a stand-in for what the gesture commits, in
 * the session's world coordinates) is cloned once per sibling frame under
 * a group carrying that frame's transform. A clone shares the ghost's
 * geometry and materials, so a mirror costs a few Object3Ds per sibling
 * and nothing is re-tessellated. Outside a session the frame list is
 * empty and nothing is drawn.
 *
 * What a sync costs, from cheapest to dearest, decided by a snapshot of
 * the ghosts (every node's identity, geometry, material, transform and
 * visibility — one walk over the ghost tree, never over the mirrors):
 *   - nothing changed: skip;
 *   - the ghost tree has the same shape (node types and arity) but new
 *     objects, geometry or transforms — Push/Pull's fresh prism per move,
 *     a Move/Rotate/Scale ghost driven by its root transform, a child
 *     swapped in place: RETARGET the existing clones (point them at the
 *     new geometry and materials, copy the transforms), no allocation;
 *   - the shape changed (an array copy joined the ghost, another node
 *     was selected): rebuild the clones.
 * The frames come from the kernel (`session_sibling_frames`), read when a
 * gesture's first ghost appears and again only when the session
 * fingerprint changes (the entered instance or the placement list — undo
 * or redo crossing a session boundary, a placement created or deleted) or
 * the viewport calls `invalidate()` (a document change, a visibility or
 * isolation change), so a mid-gesture change never leaves a mirror at a
 * stale pose. Everything is dropped when the preview empties — on commit,
 * cancel, Escape, or a tool switch, exactly when the ghost itself goes.
 *
 * Two budgets bound the cost on a big model. `SIBLING_PREVIEW_CAP` bounds
 * the frames: past it, the first frames in the kernel's order get a mirror
 * and the rest show the edit at the commit. `SIBLING_PREVIEW_NODE_BUDGET`
 * bounds the total Object3Ds mirrored, since a multi-select ghost is one
 * subtree per selected object: fewer frames get a full mirror when the
 * ghost is big, and a ghost too big for even one mirror is shown on every
 * frame as a wireframe box of its bounds instead — the drag still reads
 * on every copy, at one object per copy.
 *
 * The mirror root is a scene-level group of its own, never under the
 * preview group (a tool's `clearPreview` disposes what it finds there, and
 * the clones' geometry is the ghost's), and the print pass hides it with
 * the other overlays. Picking and inference never see it: both go through
 * the kernel, not the three.js scene.
 */

import * as THREE from 'three'
import { isSessionGhost } from '../tools/transformPreview'

/** The most sibling frames mirrored at once (see the module doc). */
export const SIBLING_PREVIEW_CAP = 64

/** The most Object3Ds the mirrors may hold in all, across every frame
 *  (see the module doc): with a 20-node ghost that is the full 64 frames,
 *  with a 500-node multi-select ghost it is 8 frames, and past 4096 nodes
 *  in one ghost every frame gets a bounds box instead. */
export const SIBLING_PREVIEW_NODE_BUDGET = 4096

/** The bounds-box proxy's tint — the push/pull accent, like the prism. */
const PROXY_COLOR = 0xee8800

/** What the last `sync()` did, for tests and the render-stats readout. */
export type SiblingPreviewWork = 'idle' | 'none' | 'skip' | 'retarget' | 'rebuild' | 'proxy'

export interface SiblingPreviewOptions {
  /** The preview group every tool draws into. */
  source: THREE.Group
  /** Where the mirrors are drawn: a scene-level group, not under `source`. */
  root: THREE.Group
  /** The sibling frames as flat row-major 3×4 affines (12 floats each),
   *  mapping the session's world coordinates onto each sibling's rendering
   *  — `Scene.session_sibling_frames()`. */
  frames: () => ArrayLike<number>
  /** A cheap stamp of what the frames depend on — the entered instance and
   *  the placement list; the frames are read again when it changes. */
  fingerprint?: () => string
  /** Overrides `SIBLING_PREVIEW_CAP`. */
  cap?: number
  /** Overrides `SIBLING_PREVIEW_NODE_BUDGET`. */
  nodeBudget?: number
}

/** The row-major 3×4 affine at `at` in `flat`, as a three.js matrix. */
export function affineToMatrix4(flat: ArrayLike<number>, at: number): THREE.Matrix4 {
  const m = new THREE.Matrix4()
  m.set(
    flat[at], flat[at + 1], flat[at + 2], flat[at + 3],
    flat[at + 4], flat[at + 5], flat[at + 6], flat[at + 7],
    flat[at + 8], flat[at + 9], flat[at + 10], flat[at + 11],
    0, 0, 0, 1,
  )
  return m
}

/** The marked ghost roots in `group`, in draw order — a marked subtree is
 *  taken whole (its root is listed, not its descendants). */
export function collectSessionGhosts(group: THREE.Object3D): THREE.Object3D[] {
  const out: THREE.Object3D[] = []
  const walk = (node: THREE.Object3D): void => {
    for (const child of node.children) {
      if (isSessionGhost(child)) out.push(child)
      else walk(child)
    }
  }
  walk(group)
  return out
}

/** An object that draws geometry with a material: a Mesh, a Line of any
 *  kind (fat lines included — `LineSegments2` extends Mesh), Points. */
type Drawable = THREE.Object3D & { geometry: THREE.BufferGeometry; material: THREE.Material | THREE.Material[] }

function isDrawable(obj: THREE.Object3D): obj is Drawable {
  return 'geometry' in obj && 'material' in obj
}

function materialStamp(m: THREE.Material | THREE.Material[]): string {
  return Array.isArray(m) ? m.map((x) => x.uuid).join('+') : m.uuid
}

/** One walk over the ghost trees: everything a mirror depends on, as a
 *  flat list that two syncs compare element by element. `count` is the
 *  number of nodes (what the node budget meters). */
export function snapshotGhosts(ghosts: THREE.Object3D[]): { stamp: (string | number)[]; count: number } {
  const stamp: (string | number)[] = []
  let count = 0
  const visit = (node: THREE.Object3D): void => {
    count++
    stamp.push(node.uuid, node.visible ? 1 : 0, node.renderOrder, node.layers.mask, node.matrixAutoUpdate ? 1 : 0)
    if (node.matrixAutoUpdate) {
      const p = node.position, q = node.quaternion, s = node.scale
      stamp.push(p.x, p.y, p.z, q.x, q.y, q.z, q.w, s.x, s.y, s.z)
    } else {
      for (const e of node.matrix.elements) stamp.push(e)
    }
    if (isDrawable(node)) stamp.push(node.geometry.uuid, materialStamp(node.material))
    for (const child of node.children) visit(child)
  }
  for (const g of ghosts) visit(g)
  return { stamp, count }
}

function sameStamp(a: (string | number)[] | null, b: (string | number)[]): boolean {
  if (a === null || a.length !== b.length) return false
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false
  return true
}

/** Point a clone at its source's current geometry, materials, transform
 *  and visibility, through the whole subtree. Returns false when the two
 *  trees no longer have the same shape (a node of another type, a child
 *  more or fewer), which means the clone must be rebuilt instead. */
function retargetTree(src: THREE.Object3D, dst: THREE.Object3D): boolean {
  if (src.constructor !== dst.constructor || src.children.length !== dst.children.length) return false
  dst.position.copy(src.position)
  dst.quaternion.copy(src.quaternion)
  dst.scale.copy(src.scale)
  dst.matrix.copy(src.matrix)
  dst.matrixAutoUpdate = src.matrixAutoUpdate
  dst.matrixWorldNeedsUpdate = true
  dst.visible = src.visible
  dst.renderOrder = src.renderOrder
  dst.layers.mask = src.layers.mask
  if (isDrawable(src) && isDrawable(dst)) {
    dst.geometry = src.geometry
    dst.material = Array.isArray(src.material) ? src.material.slice() : src.material
  }
  for (let i = 0; i < src.children.length; i++) {
    if (!retargetTree(src.children[i], dst.children[i])) return false
  }
  return true
}

export class SiblingPreviewMirror {
  private readonly source: THREE.Group
  private readonly root: THREE.Group
  private readonly readFrames: () => ArrayLike<number>
  private readonly readFingerprint: (() => string) | null
  private readonly cap: number
  private readonly nodeBudget: number
  /** The frames of the gesture in flight; null between gestures and after
   *  `invalidate()`. */
  private frames: THREE.Matrix4[] | null = null
  /** The fingerprint the frames were read under. */
  private fingerprint: string | null = null
  /** The ghost roots mirrored right now (identity, in order). */
  private ghosts: THREE.Object3D[] = []
  /** The snapshot the mirrors were last brought up to date with. */
  private stamp: (string | number)[] | null = null
  /** One mirror group per mirrored frame. */
  private mirrors: THREE.Group[] = []
  private mode: 'clones' | 'proxy' | null = null
  /** The bounds-box proxy's own geometry and material (owned here, unlike
   *  the clones' shared ones). */
  private proxyGeometry: THREE.BufferGeometry | null = null
  private proxyMaterial: THREE.LineBasicMaterial | null = null
  private _lastWork: SiblingPreviewWork = 'idle'

  constructor(opts: SiblingPreviewOptions) {
    this.source = opts.source
    this.root = opts.root
    this.readFrames = opts.frames
    this.readFingerprint = opts.fingerprint ?? null
    this.cap = opts.cap ?? SIBLING_PREVIEW_CAP
    this.nodeBudget = opts.nodeBudget ?? SIBLING_PREVIEW_NODE_BUDGET
  }

  /** How many sibling frames the gesture in flight is mirrored onto
   *  (0 between gestures and outside a session). */
  get mirrorCount(): number {
    return this.mirrors.length
  }

  /** What the last `sync()` did. */
  get lastWork(): SiblingPreviewWork {
    return this._lastWork
  }

  /** Whether the mirrors are bounds boxes rather than clones. */
  get isProxy(): boolean {
    return this.mode === 'proxy'
  }

  /** Forget the frames: the next `sync()` reads them again (and rebuilds
   *  the mirrors on them). For a change the fingerprint does not see — a
   *  visibility, tag, or isolation change, any document change. */
  invalidate(): void {
    this.frames = null
  }

  /** Bring the mirrors up to date with the preview group. Call once per
   *  frame, before the render. */
  sync(): void {
    const ghosts = collectSessionGhosts(this.source)
    if (ghosts.length === 0) {
      if (this.frames !== null || this.mirrors.length > 0) this.end()
      this._lastWork = 'idle'
      return
    }
    let framesChanged = false
    const fingerprint = this.readFingerprint?.() ?? ''
    if (this.frames === null || fingerprint !== this.fingerprint) {
      this.frames = this.loadFrames()
      this.fingerprint = fingerprint
      framesChanged = true
    }
    if (this.frames.length === 0) {
      if (this.mirrors.length > 0) this.clearMirrors()
      this.ghosts = ghosts
      this.stamp = null
      this._lastWork = 'none'
      return
    }
    const { stamp, count } = snapshotGhosts(ghosts)
    if (!framesChanged && this.sameGhosts(ghosts) && sameStamp(this.stamp, stamp)) {
      this._lastWork = 'skip'
      return
    }
    const perMirror = Math.floor(this.nodeBudget / count)
    if (perMirror === 0) {
      this.rebuildProxy(ghosts)
      this._lastWork = 'proxy'
    } else {
      const used = Math.min(this.frames.length, perMirror)
      if (!framesChanged && this.mode === 'clones' && this.mirrors.length === used && this.retarget(ghosts)) {
        this._lastWork = 'retarget'
      } else {
        this.rebuildClones(ghosts, used)
        this._lastWork = 'rebuild'
      }
    }
    this.ghosts = ghosts
    this.stamp = stamp
  }

  /** Drop every mirror and forget the gesture's frames. The clones share
   *  the ghosts' geometry and materials, so nothing of theirs is disposed;
   *  the proxy's own are. */
  end(): void {
    this.clearMirrors()
    this.ghosts = []
    this.stamp = null
    this.frames = null
    this.fingerprint = null
    this.proxyGeometry?.dispose()
    this.proxyMaterial?.dispose()
    this.proxyGeometry = null
    this.proxyMaterial = null
  }

  private clearMirrors(): void {
    this.root.clear()
    this.mirrors = []
    this.mode = null
  }

  private loadFrames(): THREE.Matrix4[] {
    const flat = this.readFrames()
    const n = Math.min(Math.floor(flat.length / 12), this.cap)
    const out: THREE.Matrix4[] = []
    for (let k = 0; k < n; k++) out.push(affineToMatrix4(flat, k * 12))
    return out
  }

  private sameGhosts(ghosts: THREE.Object3D[]): boolean {
    if (ghosts.length !== this.ghosts.length) return false
    for (let i = 0; i < ghosts.length; i++) if (ghosts[i] !== this.ghosts[i]) return false
    return true
  }

  private newMirror(frame: THREE.Matrix4): THREE.Group {
    const mirror = new THREE.Group()
    mirror.name = 'SiblingPreviewFrame'
    mirror.matrixAutoUpdate = false
    mirror.matrix.copy(frame)
    mirror.matrixWorldNeedsUpdate = true
    this.root.add(mirror)
    this.mirrors.push(mirror)
    return mirror
  }

  /** Point every existing clone at the current ghosts. False when a ghost
   *  changed shape, so the caller rebuilds. */
  private retarget(ghosts: THREE.Object3D[]): boolean {
    for (const mirror of this.mirrors) {
      if (mirror.children.length !== ghosts.length) return false
      for (let j = 0; j < ghosts.length; j++) {
        if (!retargetTree(ghosts[j], mirror.children[j])) return false
      }
    }
    return true
  }

  private rebuildClones(ghosts: THREE.Object3D[], used: number): void {
    this.clearMirrors()
    this.mode = 'clones'
    for (const frame of (this.frames ?? []).slice(0, used)) {
      const mirror = this.newMirror(frame)
      for (const ghost of ghosts) {
        // `clone(true)` shares geometry and materials and copies the
        // transform, render order, and visibility of every node.
        mirror.add(ghost.clone(true))
      }
    }
  }

  /** One wireframe box of the ghosts' bounds per frame, in the session's
   *  world coordinates (the frame maps it like the ghost it stands for). */
  private rebuildProxy(ghosts: THREE.Object3D[]): void {
    this.clearMirrors()
    this.mode = 'proxy'
    const bounds = new THREE.Box3()
    for (const ghost of ghosts) {
      ghost.updateMatrixWorld(true)
      bounds.expandByObject(ghost, true)
    }
    if (bounds.isEmpty()) return
    const center = bounds.getCenter(new THREE.Vector3())
    const size = bounds.getSize(new THREE.Vector3())
    this.proxyGeometry ??= new THREE.EdgesGeometry(new THREE.BoxGeometry(1, 1, 1))
    this.proxyMaterial ??= new THREE.LineBasicMaterial({
      color: PROXY_COLOR,
      transparent: true,
      opacity: 0.7,
      depthTest: false,
      depthWrite: false,
    })
    for (const frame of this.frames ?? []) {
      const mirror = this.newMirror(frame)
      const box = new THREE.LineSegments(this.proxyGeometry, this.proxyMaterial)
      box.name = 'SiblingPreviewBounds'
      box.position.copy(center)
      box.scale.copy(size)
      box.renderOrder = 996
      mirror.add(box)
    }
  }
}
