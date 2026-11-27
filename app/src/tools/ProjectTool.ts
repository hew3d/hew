/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * ProjectTool — drape a closed shape onto a solid, then engrave or emboss it.
 *
 * Gesture:
 *   1. Click a closed drawn shape, or select one (or a whole sketch) before
 *      choosing the tool.
 *   2. Hover the solid it should land on: the outline the shape would make
 *      on the surface shows. Click to draw it there (one undo step).
 *   3. The depth stage starts at once. Move along the projection direction
 *      to set the depth, or type it; a negative depth embosses. Alt switches
 *      between Straight (cuts straight through) and Follow (keeps the
 *      surface's curve). A ghost shows the result.
 *   4. Click or Enter commits the depth (a second undo step). Escape keeps
 *      the drawn shape and stops there.
 *   5. After the commit, typing another depth redoes it.
 *
 * The kernel owns every rule (`project_imprint_preview`, `project_imprint`,
 * `project_depth_preview`, `project_depth`); refusals show in the status bar
 * while previewing and as a toast on commit.
 */

import * as THREE from 'three'
import type { Tool, Snap, EditContext } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { formatLength, getLengthUnit, parseLengthToMeters, typedReadout } from '../settings/units'
import { editLengthBuffer, isLengthInputKey } from './moveInput'
import { defaultFaceEligible, ineligibleFaceHint } from './faceDraw'
import { buildMeshPreview } from './meshPreview'
import { LineSlot } from './previewLines'
import { cueColors, pathHoverColors } from './FollowMeTool'
import { pickRegionAt, regionRing, ringSegments, type RegionHit } from './profilePick'
import { RetypeWindow, retypeStaleMessage } from './retypeWindow'
import { projectSourceOf, type ProjectSource } from './projectSource'
import { takeTouched, type TouchedIds } from './offsetShared'
import type { PlaneUnitsScene } from './planeUnits'

export type OnProjectCommit = (touched: TouchedIds) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void
export type FaceEligible = (object: bigint, instance: bigint | undefined) => boolean

/** 0 cuts straight along the projection; 1 follows the surface. */
export type ProjectMode = 0 | 1

interface DepthSpec {
  object: bigint
  source: ProjectSource
  faces: BigUint64Array
  depth: number
  mode: ProjectMode
}

/** The point on the line `c + t·u` (unit `u`) nearest the ray, as `t`; null
 *  when the ray runs parallel to the line. */
export function nearestOnLine(
  c: readonly number[],
  u: readonly number[],
  ray: Ray,
): number | null {
  const v = ray.direction
  const w0 = [c[0] - ray.origin[0], c[1] - ray.origin[1], c[2] - ray.origin[2]]
  const dot = (p: readonly number[], q: readonly number[]): number => p[0] * q[0] + p[1] * q[1] + p[2] * q[2]
  const a = dot(u, u)
  const b = dot(u, v)
  const c2 = dot(v, v)
  const d = dot(u, w0)
  const e = dot(v, w0)
  const den = a * c2 - b * b
  if (Math.abs(den) < 1e-12 * Math.max(1, a * c2)) return null
  return (b * e - c2 * d) / den
}

/** The plain-words status for a refusal while previewing. */
function plainProblem(code: string | null, raw: string): string {
  if (code === 'ShapeDegenerate') return 'Move the shape slightly; it lines up exactly with an edge.'
  return kernelErrorMessage(code ?? 'Unknown', raw)
}

export class ProjectTool implements Tool {
  readonly name = 'Project'

  private selection: NodeRef[]
  private target: bigint | null = null
  private faces: BigUint64Array | null = null
  private dir: [number, number, number] | null = null
  private anchor: [number, number, number] | null = null
  private depth = 0
  private mode: ProjectMode = 0
  private buffer = ''
  private typedDepth = false
  private problem: string | null = null
  private ghost: THREE.Group | null = null
  private readonly hoverLine: LineSlot
  private hoverKey: string | null = null
  /** The shape clicked in the tool, when nothing was selected first. */
  private picked: ProjectSource | null = null
  /** Its outline, kept on screen while the target is picked. */
  private readonly pickedLine: LineSlot
  private readonly retype: RetypeWindow<DepthSpec>
  private faceEligible: FaceEligible | null = null
  private activeContext: bigint | null = null

  constructor(
    private readonly wasmScene: WasmScene,
    private readonly preview: THREE.Group,
    selection: NodeRef[],
    private readonly onCommit: OnProjectCommit,
    private readonly onToast: OnToast,
    private readonly onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.selection = selection
    this.hoverLine = new LineSlot(preview, 995)
    this.pickedLine = new LineSlot(preview, 994)
    this.retype = new RetypeWindow(wasmScene)
  }

  setFaceEligibility(pred: FaceEligible | null): void {
    this.faceEligible = pred
  }

  setEditContext(ctx: EditContext): void {
    this.activeContext = ctx.kind === 'object' ? ctx.id : null
  }

  setSelection(nodes: NodeRef[]): void {
    this.selection = nodes
    // A selected shape takes over from one clicked in the tool.
    if (this.picked !== null && projectSourceOf(this.wasmScene as unknown as PlaneUnitsScene, nodes) !== null) {
      this._dropPicked()
    }
  }

  // ------------------------------------------------------------------ state

  private eligible(object: bigint, instance: bigint | undefined): boolean {
    return this.faceEligible !== null
      ? this.faceEligible(object, instance)
      : defaultFaceEligible(this.wasmScene, this.activeContext, object, instance)
  }

  /** The selected shape, or else the one clicked in the tool. */
  private source(): ProjectSource | null {
    return projectSourceOf(this.wasmScene as unknown as PlaneUnitsScene, this.selection) ?? this.picked
  }

  /** The source the depth stage is working on (kept once the imprint is made). */
  private _stageSource: ProjectSource | null = null

  private get inDepthStage(): boolean {
    return this.faces !== null
  }

  statusHint(): string {
    if (this.inDepthStage) {
      const lead = this.problem !== null ? `${this.problem} ` : ''
      const mode = this.mode === 0 ? 'Depth: straight (Alt for follow)' : 'Depth: follow the surface (Alt for straight)'
      return `${lead}${mode}. Move to set the depth or type it, then click or press Enter. A negative depth embosses. Esc keeps the shape drawn on the surface.`
    }
    if (this.source() === null) return 'Click the closed shape to project.'
    if (this.problem !== null) return this.problem
    return this.picked !== null && this.picked === this.source()
      ? 'Click the object to draw the shape on it. Esc picks another shape.'
      : 'Click the object to draw the shape on it.'
  }

  capturingInput(): boolean {
    return this.inDepthStage || this.buffer !== ''
  }

  hasArmedGesture(): boolean {
    return this.inDepthStage || this.buffer !== '' || this.retype.isOpen || this.picked !== null
  }

  capturesKey(key: string): boolean {
    if (key === ' ') return false // Space always resets to Select
    if (this.inDepthStage) return true
    if (this.retype.isOpen) {
      return (key >= '0' && key <= '9') || key === '-' || key === '.' || (this.buffer !== '' && (key === 'Enter' || isLengthInputKey(key)))
    }
    return false
  }

  /** Quietly end the post-commit window (the host calls this before an
   *  explicit undo, redo or delete). */
  disarmRetype(): void {
    this.retype.close()
    // The depth stage rides on the imprint, a committed step: an undo (or
    // delete) can remove the faces it points at, so it ends with the window.
    if (this.inDepthStage) {
      this._endDepthStage()
      this.onMeasurement('')
      return
    }
    // The shape clicked in the tool may be what was undone or deleted.
    this._dropPicked()
    this._clearHover()
    if (this.buffer !== '') this._setBuffer('')
  }

  private _setBuffer(text: string): void {
    this.buffer = text
    this.onMeasurement(text === '' ? '' : typedReadout(text))
  }

  // ---------------------------------------------------------------- pointer

  private _pick(ray: Ray): { object: bigint; instance: bigint | undefined } | null {
    const hit = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (hit === undefined) return null
    try {
      return { object: hit.object(), instance: hit.instance() }
    } finally {
      hit.free()
    }
  }

  onPointerMove(_snap: Snap | null, ray: Ray): void {
    if (this.inDepthStage) {
      this._moveDepth(ray)
      return
    }
    const src = this.source()
    if (src === null) {
      this._hoverShape(ray)
      return
    }
    let hit
    try {
      hit = this._pick(ray)
    } catch {
      hit = null
    }
    if (hit === null || !this.eligible(hit.object, hit.instance)) {
      this._clearHover()
      this.problem = null
      return
    }
    const key = `${hit.object}`
    if (key === this.hoverKey) return
    this.hoverKey = key
    try {
      const lines = this.wasmScene.project_imprint_preview(hit.object, src.sketch, BigUint64Array.from(src.regions))
      this.problem = null
      this.hoverLine.set(Array.from(lines), pathHoverColors().hover, key)
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const raw = err instanceof Error ? err.message : String(err)
      this.problem = plainProblem(code, raw)
      this.hoverLine.clear()
    }
  }

  private _clearHover(): void {
    this.hoverLine.clear()
    this.hoverKey = null
  }

  /** The drawn shape under `ray`, or null. */
  private _shapeAt(ray: Ray): RegionHit | null {
    try {
      return pickRegionAt(this.wasmScene, ray, null)
    } catch {
      return null
    }
  }

  /** No shape chosen yet: outline the drawn shape under the pointer. */
  private _hoverShape(ray: Ray): void {
    const hit = this._shapeAt(ray)
    if (hit === null) {
      this._clearHover()
      return
    }
    const key = `shape:${hit.sketchHandle}:${hit.regionHandle}`
    if (key === this.hoverKey) return
    this.hoverKey = key
    this.hoverLine.set(ringSegments(regionRing(this.wasmScene, hit.sketchHandle, hit.regionHandle)), pathHoverColors().hover, key)
  }

  /** Takes the hovered drawn shape as the one to project. */
  private _pickShape(hit: RegionHit): void {
    this.picked = { sketch: hit.sketchHandle, regions: [hit.regionHandle] }
    this._clearHover()
    this.pickedLine.set(ringSegments(regionRing(this.wasmScene, hit.sketchHandle, hit.regionHandle)), pathHoverColors().path)
  }

  /** Forgets the shape clicked in the tool. */
  private _dropPicked(): void {
    this.picked = null
    this.pickedLine.clear()
  }

  onPointerDown(_snap: Snap | null, ray: Ray): void {
    this.retype.close()
    if (this.buffer !== '') this._setBuffer('')
    if (this.inDepthStage) {
      this._commitDepth()
      return
    }
    const src = this.source()
    if (src === null) {
      const shape = this._shapeAt(ray)
      if (shape !== null) this._pickShape(shape)
      else this.onToast('Click a closed drawn shape to project, then the object to draw it on.')
      return
    }
    let hit
    try {
      hit = this._pick(ray)
    } catch {
      hit = null
    }
    if (hit === null) {
      this.onToast('Click the object you want the shape drawn on.')
      return
    }
    if (!this.eligible(hit.object, hit.instance)) {
      this.onToast(ineligibleFaceHint('Project', this.activeContext !== null, hit.instance))
      return
    }
    this._commitImprint(hit.object, src)
  }

  // ----------------------------------------------------------------- imprint

  private _commitImprint(object: bigint, src: ProjectSource): void {
    try {
      const faces = this.wasmScene.project_imprint(object, src.sketch, BigUint64Array.from(src.regions))
      this._startDepth(object, src, faces)
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const raw = err instanceof Error ? err.message : String(err)
      this.onToast(plainProblem(code, raw), code ?? undefined)
    }
  }

  /** The imprint is on the surface: report it, then start the depth stage. */
  private _startDepth(object: bigint, src: ProjectSource, faces: BigUint64Array): void {
    this._clearHover()
    this.pickedLine.clear()
    this.target = object
    this.faces = faces
    this._stageSource = src
    this.depth = 0
    this.typedDepth = false
    this.problem = null
    this.dir = this._projectionDirection(object, src.sketch)
    this.anchor = this._regionCenter(object, faces)
    this.onCommit({ objectIds: [object], componentIds: [], instanceIds: [] })
    this.onMeasurement(formatLength(0))
  }

  /** The sketch plane's normal, signed to point from the sketch toward the object. */
  private _projectionDirection(object: bigint, sketch: bigint): [number, number, number] | null {
    const plane = this.wasmScene.sketch_plane(sketch)
    if (plane === undefined || plane.length < 6) return null
    let center: [number, number, number] | null = null
    try {
      const mesh = this.wasmScene.object_mesh(object)
      try {
        const p = mesh.positions()
        if (p.length >= 3) {
          const lo = [Infinity, Infinity, Infinity]
          const hi = [-Infinity, -Infinity, -Infinity]
          for (let i = 0; i + 2 < p.length; i += 3) {
            for (let k = 0; k < 3; k += 1) {
              lo[k] = Math.min(lo[k], p[i + k])
              hi[k] = Math.max(hi[k], p[i + k])
            }
          }
          center = [(lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2, (lo[2] + hi[2]) / 2]
        }
      } finally {
        mesh.free()
      }
    } catch {
      return null
    }
    if (center === null) return null
    const n: [number, number, number] = [plane[3], plane[4], plane[5]]
    const side = (center[0] - plane[0]) * n[0] + (center[1] - plane[1]) * n[1] + (center[2] - plane[2]) * n[2]
    const s = side >= 0 ? 1 : -1
    return [n[0] * s, n[1] * s, n[2] * s]
  }

  /** The middle of the drawn region's faces, where the depth line starts. */
  private _regionCenter(object: bigint, faces: BigUint64Array): [number, number, number] | null {
    let sx = 0
    let sy = 0
    let sz = 0
    let n = 0
    for (const face of faces) {
      let ring: Float32Array
      try {
        ring = this.wasmScene.face_boundary(object, face)
      } catch {
        continue
      }
      for (let i = 0; i + 2 < ring.length; i += 3) {
        sx += ring[i]
        sy += ring[i + 1]
        sz += ring[i + 2]
        n += 1
      }
    }
    return n === 0 ? null : [sx / n, sy / n, sz / n]
  }

  // ------------------------------------------------------------------ depth

  private _moveDepth(ray: Ray): void {
    if (this.typedDepth || this.dir === null || this.anchor === null) return
    const t = nearestOnLine(this.anchor, this.dir, ray)
    if (t === null) return
    this.depth = t
    this.onMeasurement(formatLength(this.depth))
    this._drawGhost()
  }

  private _depthSpec(): DepthSpec | null {
    if (this.target === null || this.faces === null || this._stageSource === null) return null
    return { object: this.target, source: this._stageSource, faces: this.faces, depth: this.depth, mode: this.mode }
  }

  private _drawGhost(): void {
    this._clearGhost()
    const spec = this._depthSpec()
    if (spec === null) return
    if (Math.abs(spec.depth) < 1e-6) {
      this.problem = null
      return
    }
    let mesh
    try {
      mesh = this.wasmScene.project_depth_preview(
        spec.object, spec.source.sketch, spec.faces, spec.depth, spec.mode,
      )
      this.problem = null
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const raw = err instanceof Error ? err.message : String(err)
      // Following the surface previews at wherever the pointer happens to
      // be, a depth nobody has chosen: a corner it cannot thicken there is
      // no reason to warn, and the depth that gets committed (typed, or a
      // click) says so itself if it fails.
      this.problem = this.mode === 1 && !this.typedDepth && code === 'UnsupportedShellCorner' ? null : plainProblem(code, raw)
      return
    }
    const ghost = buildMeshPreview(mesh, spec.depth > 0 ? cueColors().ok : cueColors().orient)
    if (ghost === null) return
    this.ghost = ghost
    this.preview.add(ghost)
  }

  private _clearGhost(): void {
    if (this.ghost === null) return
    this.ghost.traverse((o) => {
      if (o instanceof THREE.Mesh || o instanceof THREE.LineSegments) {
        o.geometry.dispose()
        ;(o.material as THREE.Material).dispose()
      }
    })
    this.preview.remove(this.ghost)
    this.ghost = null
  }

  /** One `project_depth` call; true when the kernel accepted it. */
  private _invokeDepth(spec: DepthSpec): boolean {
    try {
      const change = this.wasmScene.project_depth(
        spec.object, spec.source.sketch, spec.faces, spec.depth, spec.mode,
      )
      this.onCommit(takeTouched(change))
      return true
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const raw = err instanceof Error ? err.message : String(err)
      this.onToast(plainProblem(code, raw), code ?? undefined)
      return false
    }
  }

  private _commitDepth(): void {
    const spec = this._depthSpec()
    if (spec === null) return
    if (Math.abs(spec.depth) < 1e-6) {
      this.onToast('Move or type a depth first.')
      return
    }
    const genBefore = this.wasmScene.history_generation()
    if (!this._invokeDepth(spec)) return
    this._endDepthStage()
    this.retype.armFrom(spec, genBefore)
  }

  private _endDepthStage(): void {
    this._clearGhost()
    this._dropPicked()
    this.faces = null
    this.target = null
    this.dir = null
    this.anchor = null
    this.typedDepth = false
    this.problem = null
    this._setBuffer('')
  }

  // --------------------------------------------------------------- keyboard

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      if (this.buffer !== '') {
        this._setBuffer('')
        this.typedDepth = false
      } else if (!this.inDepthStage && this.picked !== null) {
        // Back up one stage: pick another shape.
        this._dropPicked()
        this._clearHover()
        this.problem = null
      } else {
        this.cancel()
      }
      return
    }
    if (ev.key === 'Alt') {
      if (ev.repeat) return
      this._toggleMode()
      return
    }
    const idle = !this.inDepthStage
    if (idle && !this.retype.isOpen) return
    if (!this.capturesKey(ev.key)) return
    if (ev.key === 'Enter') {
      this._enter()
      return
    }
    this._setBuffer(editLengthBuffer(this.buffer, ev.key, getLengthUnit()))
    if (this.inDepthStage) {
      const meters = parseLengthToMeters(this.buffer)
      if (meters !== null) {
        this.typedDepth = true
        this.depth = meters
        this._drawGhost()
      }
    }
  }

  private _toggleMode(): void {
    const next: ProjectMode = this.mode === 0 ? 1 : 0
    if (this.retype.isOpen) {
      this.mode = next
      this._retypeWith({ mode: next })
      return
    }
    this.mode = next
    if (this.inDepthStage) this._drawGhost()
  }

  /** Re-commit the last depth with some values changed: one undo step either
   *  way, and a refused value leaves what was there. */
  private _retypeWith(change: Partial<Pick<DepthSpec, 'depth' | 'mode'>>): void {
    const old = this.retype.spec
    if (old === null) return
    const next: DepthSpec = { ...old, ...change }
    const outcome = this.retype.apply(
      () => this._invokeDepth(next),
      (s) => this._invokeDepth(s),
      () => next,
    )
    if (outcome === 'stale') this.onToast(retypeStaleMessage('depth'))
  }

  private _enter(): void {
    const meters = this.buffer === '' ? null : parseLengthToMeters(this.buffer)
    if (this.buffer !== '' && meters === null) {
      this.onToast('Type a depth, such as 2 mm. A negative depth embosses.')
      this._setBuffer('')
      return
    }
    if (this.retype.isOpen && meters !== null) {
      this._setBuffer('')
      this._retypeWith({ depth: meters })
      return
    }
    if (meters !== null) {
      this.depth = meters
      this.typedDepth = true
    }
    this._setBuffer('')
    if (this.inDepthStage) this._commitDepth()
  }

  // ----------------------------------------------------------------- cancel

  cancel(): void {
    this.retype.close()
    this._endDepthStage()
    this._clearHover()
    this.onMeasurement('')
  }
}
