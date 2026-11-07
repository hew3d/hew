/**
 * EditVertexTool — drag a single vertex of the drawing on a plane to a new
 * position. Incident edges stretch to follow. The plane is the drafting
 * context (docs/design/unified-drafting.md): the drawing is the same whether
 * it lives on the ground, in a free sketch, or on a solid's face, so the
 * vertex is picked from the plane's `plane_units_json` listing and moved by
 * ONE draft on that context (`draft_move_vertex`). A drag clear of other
 * geometry moves the point in place; a drag that lands on a point, or makes
 * an edge touch, cross, or run along another, re-arranges the drawing like
 * drawing those lines there. The refusals left are structural (an edge
 * collapsing onto its other end, a destination off the plane, a pinch) and a
 * drag that would move a SOLID'S OWN edge (`DraftBoundaryEdgeTouched`, raised
 * when the draft ends — the document stays untouched); all surface as a
 * toast. A solid's own corners are not drawing vertices and never pick.
 *
 * Gesture (two-click, mirrors MoveTool's idle/base machine):
 *   1. idle  onPointerDown : ray-pick a drawing vertex — a free sketch's
 *      (`pick_sketch_vertex`), else one of the drawing on the face under the
 *      cursor. A hit seeds the gesture (`{ ctx, base }`) and builds a ghost
 *      preview; a miss is a no-op (idle has no destination to act on).
 *   2. base  onPointerMove : the ghost follows the resolved snap; reports the
 *      live base→cursor distance as the measurement readout.
 *   3. base  onPointerDown : if the destination ~= base (no move), just reset
 *      to idle (a click-without-drag is a cancel, not a zero-length move).
 *      Otherwise commit one draft on the vertex's context, which ARRANGES
 *      like a draw (a drag onto other lines splits and merges with them; the
 *      regions re-form). A thrown kernel error is toasted and the drawing
 *      stays as it was — the gesture always resets to idle afterward either
 *      way.
 *   4. Escape cancels the gesture (ghost cleared, back to idle).
 *
 * Ghost preview: only ONE vertex moves, so the whole-drawing translate clone
 * would be wrong here. Instead this rebuilds just the INCIDENT edges: the
 * listing's edges with an endpoint ~= `base` (within `PICK_EPSILON`) give
 * their OTHER endpoint, and a line is drawn from each to the current cursor
 * position. A vertex with no incident edges draws only a crosshair.
 */

import { t } from '@lingui/core/macro'
import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { formatLength } from '../settings/units'
import { displayedLength, isFreePick } from './displayedValue'
import { runDraft } from './draftCommit'
import { ctxOfFace, ctxOfSketchUnits, planeOfCtxIn, readPlaneUnits, type PlaneUnitsScene } from './planeUnits'

/** Fraction of the pick depth within which the ray counts as ON a vertex
 *  (about eight pixels at the default view). */
const VERTEX_PICK_FRACTION = 0.01

export type OnVertexMoveCommit = () => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/** Ghost line color — matches the sketch preview ghost used by MoveTool/transformPreview. */
const SKETCH_LINE_COLOR = 0x2266cc
/** Distance (meters) within which a `sketch_lines` endpoint is considered "the picked vertex". */
const PICK_EPSILON = 1e-6

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'base'
      /** The plane context the vertex was listed under (the draft's context). */
      ctx: string
      base: [number, number, number]
      /** Other endpoints of edges incident to `base`, captured once at pick time. */
      incidentOthers: [number, number, number][]
      /** The context's plane, point + unit normal — drives `snapConstraint`
       *  so the destination snap stays on-plane instead of resolving to
       *  ground (drawing on any plane). */
      plane: { point: [number, number, number]; normal: [number, number, number] }
    }

function approxEqual(a: [number, number, number], b: [number, number, number]): boolean {
  const dx = a[0] - b[0]
  const dy = a[1] - b[1]
  const dz = a[2] - b[2]
  return Math.sqrt(dx * dx + dy * dy + dz * dz) < PICK_EPSILON
}

export class EditVertexTool implements Tool {
  readonly name = 'Edit Vertex'

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    return this.stage.kind === 'idle'
      ? t`Click a sketch point to pick it up.`
      : t`Click where the point should go — its lines follow.`
  }

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnVertexMoveCommit
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** THREE.js LineSegments ghost for the incident-edge preview. */
  private ghost: THREE.LineSegments | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnVertexMoveCommit,
    onToast: OnToast,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onCommit = onCommit
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
  }

  // ── Tool interface ──────────────────────────────────────────────────────

  /** While dragging, constrain destination snapping to the picked vertex's
   *  own sketch plane — otherwise a rotated sketch's vertex would resolve to
   *  ground and the kernel would refuse the drag as off-plane
   *  (PointOffPlane; see Viewport's `'snapConstraint' in activeTool` duck
   *  type, the OffsetTool/LineTool precedent). Idle has no plane to offer. */
  snapConstraint(): { constraintPlane: { point: [number, number, number]; normal: [number, number, number] } } | null {
    if (this.stage.kind !== 'base') return null
    return { constraintPlane: this.stage.plane }
  }

  onPointerMove(snap: Snap | null, _ray: Ray): void {
    if (this.stage.kind !== 'base' || snap === null) return
    const cursor: [number, number, number] = [snap.x, snap.y, snap.z]
    this._updateGhost(cursor)
    this._reportMeasurement(this.stage.base, cursor)
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (this.stage.kind === 'idle') {
      const hit = this._pickVertex(ray)
      if (hit === null) return // miss — stay idle
      const { ctx, base } = hit
      // A picked vertex's plane is live by construction, but resolve it
      // defensively: a stale context is a miss, not a silent ground fallback.
      const pl = planeOfCtxIn(this.wasmScene, ctx)
      if (pl === null) return
      const plane = { point: pl.point, normal: pl.normal }
      const incidentOthers = this._findIncidentOthers(ctx, base)
      this.stage = { kind: 'base', ctx, base, incidentOthers, plane }
      this._updateGhost(base)
      return
    }

    if (this.stage.kind === 'base') {
      if (snap === null) return
      let dest: [number, number, number] = [snap.x, snap.y, snap.z]
      const { ctx, base } = this.stage

      // A free pick lands at the DISPLAYED drag distance along the drag
      // direction (there is no typed path here; this is what typing that
      // distance along the drag would mean). A hard snap keeps its point.
      if (isFreePick(snap, base)) {
        const dx = dest[0] - base[0], dy = dest[1] - base[1], dz = dest[2] - base[2]
        const len = Math.sqrt(dx * dx + dy * dy + dz * dz)
        const shown = displayedLength(len)
        if (len >= PICK_EPSILON && shown > 0) {
          dest = [base[0] + (dx / len) * shown, base[1] + (dy / len) * shown, base[2] + (dz / len) * shown]
        }
      }

      if (approxEqual(base, dest)) {
        // No movement — treat as a cancel, not a zero-length commit.
        this._resetToIdle()
        return
      }

      try {
        runDraft(this.wasmScene, { kind: 'ctx', ctx }, t`Move vertex`, (d) => {
          this.wasmScene.draft_move_vertex(d, base[0], base[1], base[2], dest[0], dest[1], dest[2])
        })
      } catch (err) {
        // The kernel refused the move (DegenerateSegment, or a solid's own
        // edge — DraftBoundaryEdgeTouched at draft end) and the draft was
        // cancelled, leaving the document untouched — toast and reset, do
        // NOT fire onCommit (nothing changed; a refresh would falsely dirty
        // the doc).
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        this._resetToIdle()
        return
      }
      this._resetToIdle()
      this.onCommit()
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
    }
  }

  cancel(): void {
    this._resetToIdle()
  }

  // ── Private helpers ─────────────────────────────────────────────────────

  /**
   * The drawing vertex under `ray`, with the context it lives in. A free
   * sketch's vertex first (`pick_sketch_vertex` — ground sketches and any
   * plane), else the nearest vertex of the drawing on the face under the
   * cursor, within a pixel-sized world tolerance of the ray. A face's own
   * corners are not in the listing, so they never pick.
   */
  private _pickVertex(ray: Ray): { ctx: string; base: [number, number, number] } | null {
    const pick = this.wasmScene.pick_sketch_vertex(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (pick !== undefined) {
      let sketch: bigint
      let base: [number, number, number]
      try {
        sketch = pick.sketch()
        base = [pick.x(), pick.y(), pick.z()]
      } finally {
        pick.free()
      }
      const ctx = ctxOfSketchUnits(this.wasmScene, sketch, null)
      return ctx === null ? null : { ctx, base }
    }

    const face = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (face === undefined) return null
    let object: bigint, faceId: bigint, instance: bigint | undefined, depth: number
    try {
      object = face.object()
      faceId = face.face()
      instance = face.instance()
      depth = face.depth()
    } finally {
      face.free()
    }
    // A face seen through a placed component instance is not editable here.
    if (instance !== undefined) return null
    const ctx = ctxOfFace(this.wasmScene, object, faceId, null)
    if (ctx === null) return null
    const pu = readPlaneUnits(this.wasmScene as unknown as PlaneUnitsScene, ctx)
    if (pu === null) return null
    const tol = VERTEX_PICK_FRACTION * Math.max(depth, 0)
    const dir = ray.direction
    const dlen = Math.hypot(dir[0], dir[1], dir[2])
    if (!(dlen > 0)) return null
    let best: { d: number; p: [number, number, number] } | null = null
    for (const u of pu.units) {
      for (const e of u.edges) {
        for (const p of [e.a, e.b]) {
          // Distance from the vertex to the ray.
          const vx = p[0] - ray.origin[0], vy = p[1] - ray.origin[1], vz = p[2] - ray.origin[2]
          const t = Math.max(0, (vx * dir[0] + vy * dir[1] + vz * dir[2]) / (dlen * dlen))
          const d = Math.hypot(vx - dir[0] * t, vy - dir[1] * t, vz - dir[2] * t)
          if (d <= tol && (best === null || d < best.d)) best = { d, p: [p[0], p[1], p[2]] }
        }
      }
    }
    return best === null ? null : { ctx, base: best.p }
  }

  /**
   * The listing's edges with an endpoint ~= `base`, collecting each one's
   * OTHER endpoint. Seeds the incident-edge ghost without a second kernel
   * round-trip for topology.
   */
  private _findIncidentOthers(
    ctx: string,
    base: [number, number, number],
  ): [number, number, number][] {
    const pu = readPlaneUnits(this.wasmScene as unknown as PlaneUnitsScene, ctx)
    const others: [number, number, number][] = []
    if (pu === null) return others
    for (const u of pu.units) {
      for (const e of u.edges) {
        const a: [number, number, number] = [e.a[0], e.a[1], e.a[2]]
        const b: [number, number, number] = [e.b[0], e.b[1], e.b[2]]
        if (approxEqual(a, base)) others.push(b)
        else if (approxEqual(b, base)) others.push(a)
      }
    }
    return others
  }

  private _resetToIdle(): void {
    this.stage = { kind: 'idle' }
    this._clearGhost()
    this.onMeasurementCb('')
  }

  private _reportMeasurement(base: [number, number, number], cursor: [number, number, number]): void {
    const dx = cursor[0] - base[0]
    const dy = cursor[1] - base[1]
    const dz = cursor[2] - base[2]
    const dist = Math.sqrt(dx * dx + dy * dy + dz * dz)
    this.onMeasurementCb(formatLength(dist))
  }

  /**
   * Rebuild the ghost: one line segment per incident edge, from its captured
   * OTHER endpoint to `cursor`. Falls back to a tiny crosshair at `cursor` if
   * the picked vertex had no incident edges in the line buffer.
   */
  private _updateGhost(cursor: [number, number, number]): void {
    this._clearGhost()
    if (this.stage.kind !== 'base') return

    const { incidentOthers } = this.stage
    const pts: number[] = []
    if (incidentOthers.length > 0) {
      for (const other of incidentOthers) {
        pts.push(other[0], other[1], other[2], cursor[0], cursor[1], cursor[2])
      }
    } else {
      // Fallback marker: a small crosshair at the cursor so the drag still
      // gives visual feedback even with no incident-edge data.
      const h = 0.05
      pts.push(
        cursor[0] - h, cursor[1], cursor[2], cursor[0] + h, cursor[1], cursor[2],
        cursor[0], cursor[1] - h, cursor[2], cursor[0], cursor[1] + h, cursor[2],
        cursor[0], cursor[1], cursor[2] - h, cursor[0], cursor[1], cursor[2] + h,
      )
    }

    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(pts), 3))
    const mat = new THREE.LineBasicMaterial({ color: SKETCH_LINE_COLOR, depthTest: false })
    const line = new THREE.LineSegments(geo, mat)
    this.preview.add(line)
    this.ghost = line
  }

  private _clearGhost(): void {
    if (this.ghost === null) return
    this.ghost.geometry.dispose()
    if (this.ghost.material instanceof THREE.Material) {
      this.ghost.material.dispose()
    }
    this.preview.remove(this.ghost)
    this.ghost = null
  }
}
