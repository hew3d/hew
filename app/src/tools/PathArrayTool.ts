/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * PathArrayTool — copy the selection along a path.
 *
 * Gesture:
 *   1. Select what to copy first (objects, groups or component instances).
 *   2. Click a path: a drawn line or curve (its whole connected run), or the
 *      outer edge of a solid's face. The hovered path is outlined first.
 *   3. Type the count (`12` or `12x`, the original included) and press Enter.
 *      Optionally type a scale ramp such as `100%→40%` and press Enter. Each
 *      Enter applies its token; a count commits. Until then, ghosts show the
 *      copies at the count and ramp so far (the first few, for a long array).
 *   4. Alt flips whether copies turn to follow the path.
 *   5. After the commit, typing a new count or ramp re-commits (one undo step
 *      throughout), until the next click, Esc or tool switch.
 *
 * The kernel places the copies (`array_along_path`) and owns every refusal; this
 * tool gathers the picks, previews the placements it is told, and relays
 * the copy for refusals.
 */

import * as THREE from 'three'
import type { LineSegments2 } from 'three/examples/jsm/lines/LineSegments2.js'
import type { Tool, Snap, EditContext } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { nodeKindToNumber, nodeRefFromJs, type NodeRef } from '../panels/treeModel'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { makeFatSegments, disposeFatSegments, PREVIEW_LINE_STYLE } from '../viewport/fatLine'
import { RetypeWindow, retypeStaleMessage } from './retypeWindow'
import { buildSelectionPreview } from './transformSelection'
import { clearPreview } from './transformPreview'
import { editCurveBuffer, isCurveInputKey, parseArrayInput } from './curveInput'
import { pickPath, pathHighlight, pathJson, type PathTarget, type PathFaceEligible } from './pathPick'
import { pathHoverColors } from './FollowMeTool'
import { defaultFaceEligible } from './faceDraw'
import { worldBoundsForSelection } from '../panels/objectBounds'

export type OnPathArrayCommit = (created: NodeRef[]) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/** The count the ghosts show before one is typed. */
const DEFAULT_COUNT = 5
/** Ghosts drawn at most: a long array previews its first copies only. */
const GHOST_CAP = 60

interface ArraySpec {
  path: PathTarget
  count: number
  from: number
  to: number
  align: boolean
}

export class PathArrayTool implements Tool {
  readonly name = 'Array along Path'

  private path: PathTarget | null = null
  private count = DEFAULT_COUNT
  private from = 1
  private to = 1
  private align = true
  private buffer = ''
  /** Why the current picks cannot preview or commit, in plain words. */
  private problem: string | null = null
  /** A note on the array just committed ("Copies overlap at this count."). */
  private note: string | null = null
  private missNotified = false
  private readonly retype: RetypeWindow<ArraySpec>
  private ghosts: THREE.Group | null = null
  private pathLine: LineSegments2 | null = null
  private hoverLine: LineSegments2 | null = null
  private hoverKey: string | null = null
  private faceEligible: PathFaceEligible | null = null
  private activeContext: bigint | null = null

  constructor(
    private readonly wasmScene: WasmScene,
    private readonly preview: THREE.Group,
    private readonly objectsGroup: THREE.Group | null,
    private selection: NodeRef[],
    private readonly onCommit: OnPathArrayCommit,
    private readonly onToast: OnToast,
    private readonly onMeasurement: OnMeasurement = () => { /* no-op */ },
    private readonly instanceGroupGetter: ((id: bigint) => THREE.Group | null) | null = null,
  ) {
    this.retype = new RetypeWindow(wasmScene)
  }

  /** Replace the face-eligibility rule (the Viewport passes the shared
   *  context-aware one, as for Follow Me). */
  setFaceEligibility(pred: PathFaceEligible | null): void {
    this.faceEligible = pred
  }

  setEditContext(ctx: EditContext): void {
    this.activeContext = ctx.kind === 'object' ? ctx.id : null
  }

  setSelection(nodes: NodeRef[]): void {
    this.selection = nodes
    if (!this.retype.isOpen) this._drawGhosts()
  }

  // ---------------------------------------------------------------- state

  private copyables(): NodeRef[] {
    return this.selection.filter((n) => n.kind === 'object' || n.kind === 'group' || n.kind === 'instance')
  }

  private eligible(): PathFaceEligible {
    return this.faceEligible
      ?? ((object, instance) => defaultFaceEligible(this.wasmScene, this.activeContext, object, instance))
  }

  statusHint(): string {
    if (this.copyables().length === 0) return 'Select what to copy first.'
    if (this.problem !== null) return this.problem
    if (this.note !== null) return this.note
    // The state first, then what Alt does to it: "Alt: align on" read as
    // an instruction to press Alt to align (playtest run 2).
    const facing = this.align
      ? 'Copies turn to follow the path (Alt: face one way).'
      : 'Copies all face one way (Alt: follow the path).'
    return `Click a path. Type a count, optionally a scale ramp like 100%→40%, then Enter. ${facing}`
  }

  capturingInput(): boolean {
    return this.buffer !== ''
  }

  hasArmedGesture(): boolean {
    return this.buffer !== '' || this.path !== null || this.retype.isOpen
  }

  capturesKey(key: string): boolean {
    if (key === ' ') return false // Space always resets to Select
    if (this.buffer === '') {
      return (this.path !== null || this.retype.isOpen) && key >= '0' && key <= '9'
    }
    return key === 'Enter' || key === 'Backspace' || key === 'X' || key === '*' || isCurveInputKey(key)
  }

  /** Quietly end the post-commit window (the host calls this before an
   *  explicit undo, redo or delete). */
  disarmRetype(): void {
    this.retype.close()
  }

  // --------------------------------------------------------------- pointer

  onPointerMove(_snap: Snap | null, ray: Ray): void {
    let pick
    try {
      pick = pickPath(this.wasmScene, ray, this.eligible())
    } catch {
      pick = { kind: 'none' as const }
    }
    if (pick.kind === 'target') {
      this.missNotified = false
      const { key, points } = pathHighlight(this.wasmScene, pick.path)
      this._showHover(key, points)
      return
    }
    if (pick.kind === 'ineligible-face') this.missNotified = false
    this._clearHover()
  }

  onPointerDown(_snap: Snap | null, ray: Ray): void {
    // Any pointer action ends the retype window and drops a half-typed value.
    this.retype.close()
    this._setBuffer('')
    if (this.copyables().length === 0) {
      this.onToast('Select what to copy first.')
      return
    }
    let pick
    try {
      pick = pickPath(this.wasmScene, ray, this.eligible())
    } catch {
      pick = { kind: 'none' as const }
    }
    if (pick.kind === 'ineligible-face') {
      this.onToast(pick.message)
      return
    }
    if (pick.kind === 'none') {
      if (!this.missNotified) {
        this.missNotified = true
        this.onToast('Click a path: a drawn line or curve, or the edge of a face.')
      }
      return
    }
    this.missNotified = false
    this.path = pick.path
    this._clearHover()
    this._drawPath()
    this._drawGhosts()
  }

  // -------------------------------------------------------------- keyboard

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      if (this.buffer !== '') this._setBuffer('')
      else this.cancel()
      return
    }
    if (ev.key === 'Alt') {
      if (ev.repeat) return
      this._setAlign(!this.align)
      return
    }
    if (ev.key === 'Enter') {
      this._enter()
      return
    }
    if (!this.capturesKey(ev.key)) return
    this._setBuffer(editCurveBuffer(this.buffer, ev.key))
  }

  private _setBuffer(text: string): void {
    this.buffer = text
    this.onMeasurement(text)
  }

  private _setAlign(on: boolean): void {
    if (this.retype.isOpen) {
      this._retype({ align: on })
      return
    }
    this.align = on
    this._drawGhosts()
  }

  /** Apply the typed token: a ramp is stored, a count commits. */
  private _enter(): void {
    if (this.buffer === '') return
    const token = parseArrayInput(this.buffer, 'm')
    this._setBuffer('')
    if ('error' in token) {
      this.onToast(token.error)
      return
    }
    if (token.kind === 'ramp') {
      if (this.retype.isOpen) this._retype({ from: token.from, to: token.to })
      else {
        this.from = token.from
        this.to = token.to
        this._drawGhosts()
      }
      return
    }
    const max = this.wasmScene.max_array_count()
    if (token.n < 2) {
      this.onToast('An array needs a count of 2 or more: the original plus at least one copy.')
      return
    }
    if (token.n > max) {
      this.onToast(`An array holds at most ${max} copies.`)
      return
    }
    if (this.retype.isOpen) {
      this._retype({ count: token.n })
      return
    }
    this.count = token.n
    if (this.path === null) {
      this.onToast('Click a path first.')
      return
    }
    this._commit()
  }

  // ----------------------------------------------------------------- commit

  private _commit(): void {
    if (this.path === null) return
    const spec: ArraySpec = { path: this.path, count: this.count, from: this.from, to: this.to, align: this.align }
    const genBefore = this.wasmScene.history_generation()
    if (!this._invoke(spec)) return
    this._clearGhosts()
    this.retype.armFrom(spec, genBefore)
  }

  /** Re-commit with some values changed: one undo step either way, and a
   *  refused size leaves the array that was there. */
  private _retype(change: Partial<Omit<ArraySpec, 'path'>>): void {
    const old = this.retype.spec
    if (old === null) return
    const next: ArraySpec = { ...old, ...change }
    const outcome = this.retype.apply(
      () => this._invoke(next),
      (s) => this._invoke(s),
      () => next,
    )
    if (outcome === 'stale') this.onToast(retypeStaleMessage('array'))
  }

  /** One `array_along_path` call; true when the kernel accepted it. */
  private _invoke(spec: ArraySpec): boolean {
    const nodes = this.copyables()
    try {
      const created = this.wasmScene.array_along_path(
        new Uint8Array(nodes.map((n) => nodeKindToNumber(n.kind))),
        new BigUint64Array(nodes.map((n) => n.id)),
        pathJson(spec.path),
        spec.count,
        spec.from,
        spec.to,
        spec.align,
      )
      this.count = spec.count
      this.from = spec.from
      this.to = spec.to
      this.align = spec.align
      this.problem = null
      this.note = this._overlapNote(spec, nodes)
      this.onCommit(created.map(nodeRefFromJs))
      return true
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const raw = err instanceof Error ? err.message : String(err)
      this.onToast(kernelErrorMessage(code ?? 'Unknown', raw), code ?? undefined)
      return false
    }
  }

  /** "Copies overlap at this count." when the copies sit closer together
   *  than the selection is long along the path's start direction. A note,
   *  never a refusal; null when it cannot be worked out. */
  private _overlapNote(spec: ArraySpec, nodes: NodeRef[]): string | null {
    try {
      const bounds = worldBoundsForSelection(this.wasmScene, nodes, { lines: false })
      if (bounds === null) return null
      const { points } = pathHighlight(this.wasmScene, spec.path)
      if (points.length < 6) return null
      const d = [points[3] - points[0], points[4] - points[1], points[5] - points[2]]
      const dl = Math.hypot(d[0], d[1], d[2])
      if (!(dl > 0)) return null
      // The box's length along the start tangent (an axis-aligned box's shadow).
      let extent = 0
      for (let k = 0; k < 3; k += 1) extent += Math.abs(d[k] / dl) * (bounds.max[k] - bounds.min[k])
      const transforms = this.wasmScene.array_along_path_preview_transforms(
        pathJson(spec.path), spec.count, spec.from, spec.to, spec.align,
      )
      const c = [0, 1, 2].map((k) => (bounds.min[k] + bounds.max[k]) / 2)
      // Where the box's center goes in the original and in each copy.
      const centers: number[][] = [c]
      for (let i = 0; i + 12 <= transforms.length; i += 12) {
        const t = transforms
        centers.push([0, 1, 2].map((r) => t[i + r * 4] * c[0] + t[i + r * 4 + 1] * c[1] + t[i + r * 4 + 2] * c[2] + t[i + r * 4 + 3]))
      }
      // Copy i (1 to count - 1) is scaled σ_i = from + (to - from)(i - 1) /
      // max(1, count - 2), the original stays 1 (the kernel's ramp): two
      // neighbors overlap when their centers sit closer than the average of
      // their scaled lengths.
      const sigma = (i: number): number =>
        i === 0 ? 1 : spec.from + (spec.to - spec.from) * ((i - 1) / Math.max(1, spec.count - 2))
      for (let i = 1; i < centers.length; i += 1) {
        const spacing = Math.hypot(
          centers[i][0] - centers[i - 1][0],
          centers[i][1] - centers[i - 1][1],
          centers[i][2] - centers[i - 1][2],
        )
        const reach = (extent * (sigma(i - 1) + sigma(i))) / 2
        if (spacing < reach - 1e-9) return 'Copies overlap at this count.'
      }
      return null
    } catch {
      return null
    }
  }

  // ------------------------------------------------------------------ ghosts

  /** Ghost copies at the current count and ramp (not drawn once committed). */
  private _drawGhosts(): void {
    this._clearGhosts()
    if (!this.retype.isOpen) this.note = null
    if (this.path === null || this.retype.isOpen) return
    const nodes = this.copyables()
    if (nodes.length === 0) return
    let transforms: Float64Array
    try {
      transforms = this.wasmScene.array_along_path_preview_transforms(
        pathJson(this.path), this.count, this.from, this.to, this.align,
      )
      this.problem = null
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const raw = err instanceof Error ? err.message : String(err)
      this.problem = kernelErrorMessage(code ?? 'Unknown', raw)
      return
    }
    const group = new THREE.Group()
    group.name = 'PathArrayGhosts'
    const shown = Math.min(GHOST_CAP, Math.floor(transforms.length / 12))
    for (let i = 0; i < shown; i += 1) {
      const t = transforms.subarray(i * 12, i * 12 + 12)
      const ghost = buildSelectionPreview(this.wasmScene, this.objectsGroup, this.instanceGroupGetter, nodes)
      if (ghost === null) break
      const holder = new THREE.Group()
      holder.matrixAutoUpdate = false
      holder.matrix.set(
        t[0], t[1], t[2], t[3],
        t[4], t[5], t[6], t[7],
        t[8], t[9], t[10], t[11],
        0, 0, 0, 1,
      )
      holder.add(ghost)
      group.add(holder)
    }
    this.ghosts = group
    this.preview.add(group)
  }

  private _clearGhosts(): void {
    if (this.ghosts === null) return
    clearPreview(this.ghosts)
    this.preview.remove(this.ghosts)
    this.ghosts = null
  }

  // ------------------------------------------------------------- highlights

  private _line(points: number[], color: number, renderOrder: number): LineSegments2 {
    return makeFatSegments(new Float32Array(points), {
      color,
      widthPx: PREVIEW_LINE_STYLE.widthPx,
      depthTest: false,
      renderOrder,
    })
  }

  private _drawPath(): void {
    this._disposeLine(this.pathLine)
    this.pathLine = null
    if (this.path === null) return
    const { points } = pathHighlight(this.wasmScene, this.path)
    if (points.length === 0) return
    this.pathLine = this._line(points, pathHoverColors().path, 996)
    this.preview.add(this.pathLine)
  }

  private _showHover(key: string, points: number[]): void {
    if (key === this.hoverKey) return
    this._clearHover()
    if (points.length === 0) return
    this.hoverLine = this._line(points, pathHoverColors().hover, 995)
    this.preview.add(this.hoverLine)
    this.hoverKey = key
  }

  private _clearHover(): void {
    this._disposeLine(this.hoverLine)
    this.hoverLine = null
    this.hoverKey = null
  }

  private _disposeLine(line: LineSegments2 | null): void {
    if (line === null) return
    this.preview.remove(line)
    disposeFatSegments(line)
  }

  // ------------------------------------------------------------------ cancel

  cancel(): void {
    this.retype.close()
    this.path = null
    this.problem = null
    this.note = null
    this.buffer = ''
    this.onMeasurement('')
    this._clearGhosts()
    this._clearHover()
    this._disposeLine(this.pathLine)
    this.pathLine = null
  }
}
