/**
 * FollowMeTool — sweep a profile region along a path (docs/design/follow-me.md).
 *
 * Gesture (SketchUp's preselect-the-path idiom):
 *   1. Activate with the path already selected (sketch edges, a drawn
 *      curve, or a whole island) → the tool starts at "click the profile".
 *      A single selected edge expands to its whole connected island, the
 *      same pickup a path click gives.
 *   2. Or pick the path with the tool itself: clicking a sketch edge takes
 *      that edge's whole connected island as the path; clicking a solid
 *      face means "run around this face's boundary" (molding).
 *   3. Click the profile region → immediate commit. The profile outline is
 *      consumed exactly like an extrusion's; the path stays. Clicking a
 *      solid FACE here re-picks the path instead — but only while the path
 *      is a leftover preselection (faces are never profiles in this
 *      release, so that stale-selection recovery is unambiguous). Once a
 *      path is picked deliberately in-tool, a stray face graze is ignored,
 *      never a silent substitution of the swept face; Esc re-picks.
 *   Esc steps back one stage. Kernel refusals surface as toasts.
 *
 * The kernel owns every eligibility decision (perpendicularity, chain
 * validity, tight bends, self-intersection); this tool only gathers the
 * two picks and relays the typed error copy.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { clearPreview } from './transformPreview'

export type PathTarget =
  | { kind: 'edges'; sketchHandle: bigint; edgeHandles: bigint[] }
  | { kind: 'face'; objectHandle: bigint; faceHandle: bigint }

/**
 * Where the current path came from. A path inherited from the
 * activation-time selection (`preselection`) may be a stale/hijacked
 * leftover from placing the profile, so a solid-face click at the profile
 * stage is allowed to re-target it (the molding recovery). A path the user
 * picked deliberately in the tool (`in-tool`) is never silently replaced by
 * a stray face graze.
 */
type PathSource = 'preselection' | 'in-tool'

type Stage =
  | { kind: 'pick-path' }
  | { kind: 'pick-profile'; path: PathTarget; pathSource: PathSource }

export type OnFollowMeCommit = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void

const HIGHLIGHT_COLOR = 0xee8800

export class FollowMeTool implements Tool {
  readonly name = 'Follow Me'

  private stage: Stage = { kind: 'pick-path' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnFollowMeCommit
  private onToast: OnToast

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnFollowMeCommit,
    onToast: OnToast,
    initialSelection: readonly NodeRef[] = [],
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onCommit = onCommit
    this.onToast = onToast

    const preselected = this._pathFromSelection(initialSelection)
    if (preselected !== null) {
      this.stage = { kind: 'pick-profile', path: preselected, pathSource: 'preselection' }
      this._highlightPath(preselected)
    }
  }

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    if (this.stage.kind === 'pick-path') {
      return 'Click the path to follow — a drawn line or curve, or a solid face to run around.'
    }
    // The face-click re-pick is offered only while the path is a leftover
    // preselection (the recovery); once a path is deliberately picked, the
    // hint stops promising a face click will retarget it.
    return this.stage.pathSource === 'preselection'
      ? 'Click the profile to sweep along the highlighted path — a solid-face click follows that face instead; Esc re-picks the path.'
      : 'Click the profile to sweep along the highlighted path. Esc re-picks the path.'
  }

  onPointerMove(_snap: Snap | null, _ray: Ray): void {
    // No hover ephemera: both stages are single clicks. The path highlight
    // drawn on pick persists until commit/cancel.
  }

  onPointerDown(_snap: Snap | null, ray: Ray): void {
    if (this.stage.kind === 'pick-path') {
      const path = this._pickPath(ray)
      if (path === null) return
      this.stage = { kind: 'pick-profile', path, pathSource: 'in-tool' }
      this._highlightPath(path)
      return
    }

    // pick-profile: a region click commits. A solid-face click instead
    // RE-PICKS the path, but ONLY when the current path is a leftover
    // preselection — the recovery that matters when a stale selection from
    // placing the profile silently became the path: the user "clicks the
    // box's top face" expecting to pick it, and before this fallback that
    // click was a dead no-op. A path the user picked deliberately in-tool
    // is NEVER retargeted by a stray face graze: doing so would silently
    // swap the swept face out from under the very next profile click. Any
    // click that neither commits nor legitimately recovers stays a no-op,
    // so the picked path survives for the next, better-aimed click.
    const regionPick = this.wasmScene.pick_sketch_region(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (regionPick === undefined) {
      if (this.stage.pathSource !== 'preselection') return
      const facePick = this.wasmScene.pick_face(
        ray.origin[0], ray.origin[1], ray.origin[2],
        ray.direction[0], ray.direction[1], ray.direction[2],
      )
      if (facePick !== undefined) {
        let path: PathTarget
        try {
          path = { kind: 'face', objectHandle: facePick.object(), faceHandle: facePick.face() }
        } finally {
          facePick.free()
        }
        // The recovered face is now a deliberate pick — mark it in-tool so a
        // further stray graze can't re-target it in turn.
        this.stage = { kind: 'pick-profile', path, pathSource: 'in-tool' }
        this._highlightPath(path)
      }
      return
    }
    let sketchHandle: bigint
    let regionHandle: bigint
    try {
      sketchHandle = regionPick.sketch()
      regionHandle = regionPick.region()
    } finally {
      regionPick.free()
    }
    this._commit(this.stage.path, sketchHandle, regionHandle)
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      if (this.stage.kind === 'pick-profile') {
        // Step back one stage: drop the picked path, keep the tool.
        this.stage = { kind: 'pick-path' }
        clearPreview(this.preview)
      } else {
        this.cancel()
      }
    }
  }

  cancel(): void {
    this.stage = { kind: 'pick-path' }
    clearPreview(this.preview)
  }

  /**
   * Resolve a preselection into a path. Sketch-scoped refs (edges, curves,
   * islands) contribute their edges; they must all live in ONE sketch.
   * Anything else (objects, groups…) yields no preselected path — the tool
   * starts at pick-path instead. Stale handles resolve to nothing.
   *
   * A selection of exactly ONE edge expands to its whole connected island —
   * the same one-click pickup the in-tool path click gives (the guide's
   * "clicking one line picks up the whole connected shape"); a Select click
   * on a line yields a single sketch-edge ref, and without this expansion
   * the preselect flow swept just that segment. An explicit multi-edge
   * selection is honored as picked (a deliberate partial path).
   */
  private _pathFromSelection(selection: readonly NodeRef[]): PathTarget | null {
    let sketchHandle: bigint | null = null
    const edges = new Set<bigint>()
    let sketchRefs = 0
    let soleEdgeRef: NodeRef | null = null
    for (const ref of selection) {
      if (ref.sketch === undefined) continue
      if (ref.kind !== 'sketch-edge' && ref.kind !== 'sketch-curve' && ref.kind !== 'sketch-island') {
        continue
      }
      sketchRefs += 1
      soleEdgeRef = sketchRefs === 1 && ref.kind === 'sketch-edge' ? ref : null
      if (sketchHandle === null) sketchHandle = ref.sketch
      else if (sketchHandle !== ref.sketch) return null // spans two sketches
      try {
        if (ref.kind === 'sketch-edge') {
          edges.add(ref.id)
        } else if (ref.kind === 'sketch-curve') {
          for (const e of this.wasmScene.sketch_curve_edges(ref.sketch, ref.id)) edges.add(e)
        } else {
          for (const e of this.wasmScene.sketch_island_edges(ref.sketch, ref.id)) edges.add(e)
        }
      } catch {
        return null // stale handle — no usable preselection
      }
    }
    if (sketchHandle === null || edges.size === 0) return null
    if (soleEdgeRef !== null) {
      try {
        const island = this.wasmScene.sketch_edge_island(sketchHandle, soleEdgeRef.id)
        if (island !== undefined) {
          for (const e of this.wasmScene.sketch_island_edges(sketchHandle, island)) edges.add(e)
        }
      } catch {
        // stale mid-query — fall back to the bare edge
      }
    }
    return { kind: 'edges', sketchHandle, edgeHandles: [...edges] }
  }

  /**
   * Pick a path under the cursor: a sketch edge expands to its whole
   * connected island (the shape the user drew — the kernel refuses a
   * branching island with its own guidance); otherwise a solid face means
   * "run around this face's outer boundary".
   */
  private _pickPath(ray: Ray): PathTarget | null {
    const edgePick = this.wasmScene.pick_sketch_edge(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (edgePick !== undefined) {
      let sketchHandle: bigint
      let edgeHandle: bigint
      try {
        sketchHandle = edgePick.sketch()
        edgeHandle = edgePick.edge()
      } finally {
        edgePick.free()
      }
      try {
        const island = this.wasmScene.sketch_edge_island(sketchHandle, edgeHandle)
        const edgeHandles = island !== undefined
          ? [...this.wasmScene.sketch_island_edges(sketchHandle, island)]
          : [edgeHandle]
        return { kind: 'edges', sketchHandle, edgeHandles }
      } catch {
        return { kind: 'edges', sketchHandle, edgeHandles: [edgeHandle] }
      }
    }

    const facePick = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (facePick !== undefined) {
      try {
        return {
          kind: 'face',
          objectHandle: facePick.object(),
          faceHandle: facePick.face(),
        }
      } finally {
        facePick.free()
      }
    }
    return null
  }

  private _commit(path: PathTarget, sketchHandle: bigint, regionHandle: bigint): void {
    try {
      const objectId = path.kind === 'edges'
        ? this.wasmScene.follow_me_along_edges(
            sketchHandle,
            regionHandle,
            path.sketchHandle,
            new BigUint64Array(path.edgeHandles),
          )
        : this.wasmScene.follow_me_around_face(
            sketchHandle,
            regionHandle,
            path.objectHandle,
            path.faceHandle,
          )
      this.stage = { kind: 'pick-path' }
      clearPreview(this.preview)
      this.onCommit(objectId)
    } catch (err) {
      // Typed refusal: keep the picked path so the user can adjust the
      // profile and click again.
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      const message = kernelErrorMessage(code ?? 'Unknown', rawMsg)
      this.onToast(message, code ?? undefined)
    }
  }

  /** Draw the picked path as highlighted line segments (viewport ephemera). */
  private _highlightPath(path: PathTarget): void {
    clearPreview(this.preview)
    const pts: number[] = []
    try {
      if (path.kind === 'edges') {
        for (const edge of path.edgeHandles) {
          const ends = this.wasmScene.sketch_edge_endpoints(path.sketchHandle, edge)
          if (ends !== undefined && ends.length >= 6) {
            pts.push(ends[0], ends[1], ends[2], ends[3], ends[4], ends[5])
          }
        }
      } else {
        const loop = this.wasmScene.face_boundary(path.objectHandle, path.faceHandle)
        for (let i = 0; i < loop.length; i += 3) {
          const j = (i + 3) % loop.length
          pts.push(loop[i], loop[i + 1], loop[i + 2], loop[j], loop[j + 1], loop[j + 2])
        }
      }
    } catch {
      return // stale handle — no highlight, the commit will surface an error
    }
    if (pts.length === 0) return
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(pts), 3))
    const mat = new THREE.LineBasicMaterial({ color: HIGHLIGHT_COLOR, depthTest: false })
    const lines = new THREE.LineSegments(geo, mat)
    lines.renderOrder = 996
    this.preview.add(lines)
  }
}
