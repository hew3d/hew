/**
 * EraserTool — erase drawn lines. With the Smooth preview, also crease the
 * edges of solids.
 *
 * Hover an edge to see what a click would do:
 *   - a drawn line (a line, a curve, a shape's edge, on the ground or on a
 *     face): a click erases it, the whole curve if it is one. Drag across
 *     several lines to erase them all as one undo step.
 *   - an edge of a solid: a plain click only explains that solid edges can't
 *     be erased.
 *
 * The crease half is a Preview behavior (`eraser-crease`, part of the Smooth
 * feature). While it is on: Ctrl-click (Cmd on a Mac) creases an edge, which
 * keeps it sharp when the object is smoothed; Ctrl-click a creased edge to
 * let it soften again; Ctrl-drag creases every edge it crosses; Ctrl-click
 * on a soft seam says seams can't be creased. While it is off the modifier
 * is ignored and the tool behaves as if it were never held, and none of the
 * crease words (`eraserCreaseCopy.ts`) are reachable.
 *
 * The kernel owns every rule; drawn lines erase through the same path as the
 * dock's Erase (`runDelete`), creases through `set_crease`.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { t } from '@lingui/core/macro'
import { behaviorEnabled } from '../preview'
import { creaseCopy, type CreaseCopy } from './eraserCreaseCopy'
import { LineSlot } from './previewLines'
import { cueColors, pathHoverColors } from './FollowMeTool'
import { geometryOf, type PlaneUnitsScene } from './planeUnits'
import { takeTouched, type TouchedIds } from './offsetShared'

export type OnToast = (message: string, code?: string) => void

/** Whether the crease modifier does anything: the Smooth preview's `eraser-crease` behavior. */
export const creaseAvailable = (): boolean => creaseCopy !== null && behaviorEnabled('eraser-crease')

/** The crease words; only called where the gate is on, which a production build never is. */
function words(): CreaseCopy {
  if (creaseCopy === null) throw new Error('unreachable')
  return creaseCopy
}

/** What the viewport lends the tool. */
export interface EraserHost {
  /** The selectable thing under the snap, as the Select tool would pick it. */
  resolveRef(snap: Snap | null, ray: Ray): NodeRef | null
  /** Erase drawn things exactly as the dock's Erase does (refreshes itself). */
  runDelete(nodes: NodeRef[]): void
  /** Crease changes were committed; refresh these objects. */
  onCreaseCommit(touched: TouchedIds): void
}

/** The kernel's `edge_smoothness` codes. */
const HARD = 0
const TANGENT = 1
const SEAM = 2

/** Screen pixels the pointer may move between press and release and still be a click. */
export const CLICK_SLOP_PX = 4

export type EraserTarget =
  | { kind: 'drawn'; key: string; ref: NodeRef }
  | { kind: 'solid'; key: string; object: bigint; edges: bigint[] }
  | { kind: 'seam'; key: string; object: bigint; edges: bigint[] }
  | { kind: 'fill'; key: string }
  | { kind: 'component'; key: string }

export class EraserTool implements Tool {
  readonly name = 'Eraser'

  private crease = false
  private target: EraserTarget | null = null
  private pressed = false
  private pressPx: [number, number] | null = null
  private pressMode: 'erase' | 'crease' = 'erase'
  private dragging = false
  /** A Ctrl-drag that crossed a soft seam has said once that seams can't
   *  be creased; the rest of the drag creases what it can. */
  private seamToasted = false
  private currentPx: [number, number] | null = null
  private readonly collected = new Map<string, EraserTarget>()
  private readonly hoverLine: LineSlot
  private readonly collectLine: LineSlot
  private hoverKey: string | null = null

  constructor(
    private readonly wasmScene: WasmScene,
    private readonly preview: THREE.Group,
    private readonly host: EraserHost,
    private readonly onToast: OnToast,
    /** Whether the crease modifier is live; tests pass their own. */
    private readonly creaseEnabled: () => boolean = creaseAvailable,
  ) {
    this.hoverLine = new LineSlot(preview, 995)
    this.collectLine = new LineSlot(preview, 996)
  }

  /** The crease modifier (Ctrl, or Cmd on a Mac), read live from each pointer event. */
  setCreaseModifier(on: boolean): void {
    this.crease = on && this.creaseEnabled()
  }

  // ----------------------------------------------------------------- status

  statusHint(): string {
    const t_ = this.target
    const creasing = this.creaseEnabled()
    if (t_ === null) {
      const base = t`Click a drawn line to erase it. Drag across lines to erase several.`
      return creasing ? words().idle(base) : base
    }
    switch (t_.kind) {
      case 'drawn':
        return this.crease ? words().drawnWithModifier() : t`Click to erase this line.`
      case 'solid': {
        if (!creasing) return t`Edges of a solid can't be erased.`
        if (!this.crease) return words().solidEdge()
        const creased = t_.edges.length > 0 && this.wasmScene.edge_is_creased(t_.object, t_.edges[0])
        return creased ? words().creaseOff : words().creaseOn
      }
      case 'seam':
        return this.crease ? words().seamWithModifier : t`That edge is a soft seam.`
      case 'fill':
        return t`Click a line to erase it, not the inside of a shape.`
      case 'component':
        return creasing ? words().componentEdge : t`Open the component first to erase its edges.`
    }
  }

  capturingInput(): boolean {
    return this.pressed
  }

  hasArmedGesture(): boolean {
    return this.pressed
  }

  // ---------------------------------------------------------------- targets

  /** What the pointer is over. */
  private targetAt(snap: Snap | null, ray: Ray): EraserTarget | null {
    if (snap !== null && snap.object !== undefined && snap.elementKind === 'edge' && snap.element !== undefined) {
      if (snap.instance !== undefined) return { kind: 'component', key: `i:${snap.instance}` }
      const object = snap.object
      const edge = snap.element
      const smooth = this.wasmScene.edge_smoothness(object, edge)
      if (smooth === SEAM) return { kind: 'seam', key: `m:${object}:${edge}`, object, edges: [edge] }
      if (smooth === TANGENT) {
        return { kind: 'solid', key: `s:${object}:${edge}`, object, edges: [edge] }
      }
      if (smooth === HARD) {
        // The whole logical edge, walked along a tangent chain: the base
        // ring of a faceted round is one edge to crease, not one facet.
        const chain = this.wasmScene.logical_edge(object, edge, true)
        let group: bigint[]
        try {
          group = Array.from(chain.edges())
        } finally {
          chain.free()
        }
        if (group.length > 0) {
          const key = `s:${object}:${group.map(String).sort().join('.')}`
          return { kind: 'solid', key, object, edges: group }
        }
        // A hard edge that bends nowhere is a drawn line on a face: fall through.
      }
    }
    const ref = this.host.resolveRef(snap, ray)
    if (ref === null) return null
    if (ref.kind === 'draft-edge' || ref.kind === 'draft-curve') {
      return { kind: 'drawn', key: `d:${ref.kind}:${ref.ctx ?? ''}:${ref.key ?? ref.id}`, ref }
    }
    if (ref.kind === 'draft-region' || ref.kind === 'draft-island') return { kind: 'fill', key: `f:${ref.key ?? ref.id}` }
    return null
  }

  /** Whether the current press collects `t`: erase takes drawn lines, crease takes solid edges. */
  private collects(t: EraserTarget | null): t is Extract<EraserTarget, { kind: 'drawn' | 'solid' }> {
    if (t === null) return false
    return this.pressMode === 'erase' ? t.kind === 'drawn' : t.kind === 'solid'
  }

  /** The line work of a target, as flat segment endpoints. */
  private segmentsOf(t: EraserTarget): number[] {
    if (t.kind === 'drawn') return Array.from(geometryOf(this.wasmScene as unknown as PlaneUnitsScene, t.ref))
    if (t.kind === 'solid' || t.kind === 'seam') {
      const out: number[] = []
      for (const e of t.edges) {
        const p = this.wasmScene.edge_endpoints(t.object, e)
        if (p !== undefined && p.length >= 6) out.push(p[0], p[1], p[2], p[3], p[4], p[5])
      }
      return out
    }
    return []
  }

  // ---------------------------------------------------------------- pointer

  onPointerMove(snap: Snap | null, ray: Ray): void {
    let t: EraserTarget | null
    try {
      t = this.targetAt(snap, ray)
    } catch {
      t = null
    }
    this.target = t
    if (this.pressed && this.dragging && this.collects(t)) {
      if (!this.collected.has(t.key)) {
        this.collected.set(t.key, t)
        this._drawCollected()
      }
    }
    this._noteSeamInDrag(t)
    this._showHover(t)
  }

  /** A soft seam under a Ctrl-drag: skipped, and said once per drag. */
  private _noteSeamInDrag(t: EraserTarget | null): void {
    if (!this.pressed || !this.dragging || this.pressMode !== 'crease') return
    if (t === null || t.kind !== 'seam' || this.seamToasted) return
    this.seamToasted = true
    this.onToast(words().seamToast, words().seamCode)
  }

  onPointerScreenMove(x: number, y: number): void {
    this.currentPx = [x, y]
    if (this.pressed && !this.dragging && this.pressPx !== null) {
      if (Math.hypot(x - this.pressPx[0], y - this.pressPx[1]) > CLICK_SLOP_PX) {
        this.dragging = true
        // The press itself counts as the first thing swept over.
        if (this.collects(this.target)) {
          this.collected.set(this.target.key, this.target)
          this._drawCollected()
        }
        this._noteSeamInDrag(this.target)
      }
    }
  }

  onPointerRawDown(x: number, y: number): void {
    this.pressPx = [x, y]
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    try {
      this.target = this.targetAt(snap, ray)
    } catch {
      this.target = null
    }
    this.pressed = true
    this.dragging = false
    this.seamToasted = false
    this.pressMode = this.crease ? 'crease' : 'erase'
    this.collected.clear()
    this.collectLine.clear()
    if (this.pressPx === null) this.pressPx = this.currentPx
  }

  onPointerUp(_snap: Snap | null, _ray: Ray): void {
    if (!this.pressed) return
    const mode = this.pressMode
    const dragged = this.dragging
    const target = this.target
    const collected = [...this.collected.values()]
    this._endPress()
    if (dragged) {
      if (collected.length > 0) this._apply(mode, collected)
      return
    }
    if (target === null) return
    if (mode === 'erase') {
      if (target.kind === 'drawn') this._apply('erase', [target])
      // A solid edge, seam or fill only explains itself in the status bar.
      return
    }
    // Crease mode: solid edges toggle; a seam asks the kernel and shows its refusal.
    if (target.kind === 'solid' || target.kind === 'seam') this._apply('crease', [target])
  }

  private _endPress(): void {
    this.pressed = false
    this.dragging = false
    this.pressPx = null
    this.collected.clear()
    this.collectLine.clear()
  }

  // ------------------------------------------------------------------ apply

  private _inBracket(label: string, count: number, run: () => void): void {
    if (count <= 1) {
      run()
      return
    }
    this.wasmScene.history_bracket_open()
    try {
      run()
      this.wasmScene.history_bracket_commit(label)
    } catch (err) {
      this.wasmScene.history_bracket_abort()
      throw err
    }
  }

  private _apply(mode: 'erase' | 'crease', targets: EraserTarget[]): void {
    this._clearHover()
    if (mode === 'erase') {
      const refs = targets.flatMap((t) => (t.kind === 'drawn' ? [t.ref] : []))
      if (refs.length === 0) return
      try {
        this._inBracket(t`Erase`, refs.length, () => this.host.runDelete(refs))
      } catch (err) {
        this._toast(err)
      }
      return
    }
    const edges = targets.flatMap((t) => (t.kind === 'solid' || t.kind === 'seam' ? [t] : []))
    if (edges.length === 0) return
    const touched: TouchedIds = { objectIds: [], componentIds: [], instanceIds: [] }
    try {
      this._inBracket(words().historyLabel, edges.length, () => {
        for (const t of edges) {
          // Each logical edge toggles from its own first edge's flag.
          const creased = t.edges.length > 0 && this.wasmScene.edge_is_creased(t.object, t.edges[0])
          const change = this.wasmScene.set_crease(t.object, BigUint64Array.from(t.edges), !creased)
          const got = takeTouched(change)
          touched.objectIds.push(...got.objectIds)
          touched.componentIds.push(...got.componentIds)
          touched.instanceIds.push(...got.instanceIds)
        }
      })
    } catch (err) {
      this._toast(err)
      return
    }
    this.host.onCreaseCommit(touched)
    // A crease changes nothing visible until the object is smoothed: say
    // what happened (playtest run 3, F-44).
    // Each logical edge toggled from its own flag, so a drag can crease
    // some and uncrease others: count both after the commit.
    let creasedN = 0
    let uncreasedN = 0
    for (const t of edges) {
      const on = t.edges.length > 0 && this.wasmScene.edge_is_creased(t.object, t.edges[0])
      if (on) creasedN++
      else uncreasedN++
    }
    this.onToast(words().toast(creasedN, uncreasedN))
  }

  private _toast(err: unknown): void {
    const code = parseKernelErrorCode(err)
    const raw = err instanceof Error ? err.message : String(err)
    this.onToast(kernelErrorMessage(code ?? 'Unknown', raw), code ?? undefined)
  }

  // ------------------------------------------------------------- highlights

  private _showHover(t: EraserTarget | null): void {
    if (t === null || t.kind === 'fill' || t.kind === 'component') {
      this._clearHover()
      return
    }
    if (t.key === this.hoverKey) return
    this.hoverKey = t.key
    const color = t.kind === 'seam' ? cueColors().blocked : pathHoverColors().hover
    this.hoverLine.set(this.segmentsOf(t), color, t.key)
  }

  private _clearHover(): void {
    this.hoverLine.clear()
    this.hoverKey = null
  }

  private _drawCollected(): void {
    const segs: number[] = []
    for (const t of this.collected.values()) segs.push(...this.segmentsOf(t))
    this.collectLine.set(segs, this.pressMode === 'erase' ? cueColors().blocked : pathHoverColors().path)
  }

  // ----------------------------------------------------------------- keys

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      if (this.pressed) this._endPress()
      else this.cancel()
    }
  }

  cancel(): void {
    this._endPress()
    this._clearHover()
    this.target = null
  }
}
