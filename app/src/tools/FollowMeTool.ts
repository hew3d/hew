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
 *      face means "run around this face's boundary" (molding). While the
 *      cursor is at the path stage, the face-loop (or edge-island) under it
 *      is highlighted — the sweep target is shown *before* the click, so a
 *      solid's face, which cannot be preselected, is discoverable and picked
 *      directly rather than through whatever the ray happens to hit.
 *   3. Click the profile region → immediate commit. The profile outline is
 *      consumed exactly like an extrusion's; the path stays. Clicking a
 *      solid FACE here re-picks the path instead — but only while the path
 *      is a leftover preselection (faces are never profiles in this
 *      release, so that stale-selection recovery is unambiguous). Once a
 *      path is picked deliberately in-tool, a stray face graze is ignored,
 *      never a silent substitution of the swept face; Esc re-picks.
 *   Esc steps back one stage. Kernel refusals surface as toasts; because the
 *   path source (a solid face) changes what "no good" means, the two face-
 *   specific refusals are re-worded against the FACE the user picked
 *   ("that face is parallel to the profile" / "…thinner than the profile is
 *   deep") rather than the generic drawn-path copy.
 *
 * The kernel owns every geometric eligibility decision (perpendicularity,
 * chain validity, tight bends, self-intersection); this tool only gathers
 * the two picks, previews the target, and relays the typed error copy —
 * never a silent no-op when a pick lands on nothing.
 *
 * FACE FRAME GUARD. `face_boundary` and `follow_me_around_face` take only
 * (object, face): they are coordinate-correct ONLY for a plain, identity-
 * placed, top-level world object. A face on a component INSTANCE is stored in
 * definition-local space (its world pose lives on the placement), and a face
 * reached inside a group/instance/object editing context is likewise out of
 * frame — there is no `follow_me_in_component` surface (cf.
 * `push_pull_in_component`). So the face branch is gated to the same
 * plain-top-level set every other face tool uses (faceDraw's
 * `defaultFaceEligible`, injected as `faceDrawEligible`), and any in-context
 * or instanced face is refused with copy rather than swept in the wrong
 * frame. Molding a face of an actively-edited component is a KNOWN
 * LIMITATION pending a kernel/wasm surface. Sketch-EDGE paths carry their own
 * world coordinates and are not gated.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { defaultFaceEligible, type FaceEligible } from './faceDraw'
import { clearPreview } from './transformPreview'

export type PathTarget =
  | { kind: 'edges'; sketchHandle: bigint; edgeHandles: bigint[] }
  // `instance` is carried only to keep the hover-dedup key distinct per
  // placement; a followable face is always a plain world object (instance
  // `undefined`) — `face_boundary`/`follow_me_around_face` take just
  // (object, face) and are coordinate-correct for nothing else.
  | { kind: 'face'; objectHandle: bigint; faceHandle: bigint; instance: bigint | undefined }

/**
 * The outcome of resolving a path-stage pick: a usable target, a real face
 * the tool cannot correctly sweep (refused with copy, never a wrong-frame
 * sweep), or nothing under the cursor.
 */
type PathPick =
  | { kind: 'target'; path: PathTarget }
  | { kind: 'ineligible-face'; message: string }
  | { kind: 'none' }

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

/** The committed (picked) path highlight — the loop/chain being swept. */
const PATH_COLOR = 0xee8800
/** The pre-click hover preview of the target under the cursor. */
const HOVER_COLOR = 0xffbb44

/** Guidance when a path-stage click lands on no followable geometry. */
const PATH_MISS =
  'Click the flat face to run the profile around it — or a drawn line or curve to follow.'

export class FollowMeTool implements Tool {
  readonly name = 'Follow Me'

  private stage: Stage = { kind: 'pick-path' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnFollowMeCommit
  private onToast: OnToast

  /** The picked-path highlight (persists from pick until commit/cancel). */
  private pathHighlight: THREE.LineSegments | null = null
  /** The pre-click hover preview of the target under the cursor. */
  private hoverHighlight: THREE.LineSegments | null = null
  /** Identity of what the hover currently shows, so a still cursor doesn't
   *  rebuild the same geometry every move. */
  private hoverKey: string | null = null
  /** Whether the current run of empty/refused clicks has already been called
   *  out, so repeated clicks on nothing (or on the same ineligible face)
   *  don't stack identical toasts (cleared the moment a real face — eligible
   *  or not — appears under the cursor, or a pick lands). */
  private missNotified = false

  /** The entered-object context id (null at top level); mirrors PushPullTool.
   *  Only the shared `defaultFaceEligible` fallback reads it — in production
   *  the Viewport injects `faceDrawEligible`, which knows the full path. */
  private _activeContext: bigint | null = null
  /** Richer eligibility injected by the Viewport (knows the full
   *  group/instance context path); null = the shared default policy. */
  private _faceEligible: FaceEligible | null = null
  /** True while ANY editing context is entered (object, group, or instance) —
   *  Follow Me runs at the top level only, so this refuses in-context face
   *  molding wholesale (see the FACE FRAME GUARD note). */
  private _contextScoped = false

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

  /** Set the active editing context (entered object), or null for top level.
   *  Wired by the Viewport exactly like PushPullTool. */
  setActiveContext(objectId: bigint | null): void {
    this._activeContext = objectId
  }

  /** Inject the Viewport's context-path-aware face policy (or null for the
   *  shared default). Wired like PushPullTool so Follow Me is on the same
   *  face-eligibility system as every other face tool. */
  setFaceEligibility(pred: FaceEligible | null): void {
    this._faceEligible = pred
  }

  /** True while any editing context is entered — the Viewport sets it (the
   *  object/instance id channels don't cover a GROUP context). Follow Me is a
   *  top-level act, so this makes in-context face molding refuse. */
  setContextScoped(scoped: boolean): void {
    this._contextScoped = scoped
  }

  /** Whether the face on `object` (hit through `instance`) may become a path.
   *  Only a plain, top-level, non-instanced world object is coordinate-correct
   *  for `follow_me_around_face` (see the FACE FRAME GUARD note). */
  private _faceFollowable(object: bigint, instance: bigint | undefined): boolean {
    // No in-component variant: an instanced face is definition-local and would
    // sweep in the wrong frame.
    if (instance !== undefined) return false
    // Follow Me runs at the top level; refuse any in-context face for now
    // (known limitation) rather than sweep a face the profile can't meet.
    if (this._contextScoped || this._activeContext !== null) return false
    // The shared plain-top-level policy (grouped faces refused, etc.).
    return this._faceEligible !== null
      ? this._faceEligible(object, instance)
      : defaultFaceEligible(this.wasmScene, this._activeContext, object, instance)
  }

  /** Why an ineligible face refused, directing the user to a plain object's
   *  face (Follow Me can't mold instanced/grouped/in-context geometry). */
  private _ineligibleFaceHint(instance: bigint | undefined): string {
    if (this._contextScoped || this._activeContext !== null) {
      return 'Follow Me runs at the top level — press Esc to step out of what you are editing first.'
    }
    if (instance !== undefined) {
      return 'That face belongs to a component — Follow Me needs a plain object. Explode the instance, then follow the face.'
    }
    return 'That face is inside a group — Follow Me needs a plain object. Ungroup it, then follow the face.'
  }

  onPointerMove(_snap: Snap | null, ray: Ray): void {
    // Hover preview only matters while choosing the path: show the sweep
    // target — the face-loop or edge-chain under the cursor — before the
    // click commits to it. At the profile stage the picked path stays
    // highlighted and there is nothing transient to preview.
    if (this.stage.kind !== 'pick-path') return
    let pick: PathPick
    try {
      pick = this._pickPath(ray)
    } catch {
      pick = { kind: 'none' }
    }
    if (pick.kind === 'target') {
      // A usable target is under the cursor — any earlier "nothing there" note
      // is stale, so the next genuine miss speaks up again.
      this.missNotified = false
      const { key, points } = this._targetHighlight(pick.path)
      this._showHover(key, points)
      return
    }
    // An ineligible face still counts as "a real face is here", so the next
    // click should speak (arming the refusal toast); but it is never previewed
    // as a sweep target.
    if (pick.kind === 'ineligible-face') this.missNotified = false
    this._clearHover()
  }

  onPointerDown(_snap: Snap | null, ray: Ray): void {
    if (this.stage.kind === 'pick-path') {
      const pick = this._pickPath(ray)
      if (pick.kind === 'target') {
        this.missNotified = false
        this.stage = { kind: 'pick-profile', path: pick.path, pathSource: 'in-tool' }
        this._highlightPath(pick.path)
        return
      }
      // Never a silent no-op: a solid face cannot be preselected, so a
      // path-stage click that hits nothing — or lands on a face the tool
      // cannot correctly sweep (instanced / in-context) — says what to aim at.
      this._notifyMiss(pick.kind === 'ineligible-face' ? pick.message : PATH_MISS)
      return
    }

    // pick-profile: a region click commits. A solid-face click instead
    // RE-PICKS the path, but ONLY when the current path is a leftover
    // preselection — the recovery that matters when a stale selection from
    // placing the profile silently became the path: the user "clicks the
    // box's top face" expecting to pick it, and before this fallback that
    // click was a dead no-op. A path the user picked deliberately in-tool
    // is NEVER retargeted by a stray face graze: doing so would silently
    // swap the swept face out from under the very next profile click. A
    // near-miss of a small profile's interior lands here constantly, so an
    // in-tool region miss stays quiet (the picked path is highlighted; the
    // user simply clicks again) — surfacing a toast on every near-miss would
    // be noise, not guidance.
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
        let object: bigint
        let face: bigint
        let instance: bigint | undefined
        try {
          object = facePick.object()
          face = facePick.face()
          instance = facePick.instance()
        } finally {
          facePick.free()
        }
        // Same frame guard as the path stage: a stale-preselection recovery
        // must not adopt an instanced/in-context face the sweep can't place.
        // Route through _notifyMiss so repeated clicks on the same ineligible
        // face don't stack toasts, matching the path stage's anti-spam dedup.
        if (!this._faceFollowable(object, instance)) {
          this._notifyMiss(this._ineligibleFaceHint(instance))
          return
        }
        // The recovered face is now a deliberate pick — mark it in-tool so a
        // further stray graze can't re-target it in turn.
        const path: PathTarget = { kind: 'face', objectHandle: object, faceHandle: face, instance }
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
        this.missNotified = false
        this._clearPath()
        this._clearHover()
      } else {
        this.cancel()
      }
    }
  }

  cancel(): void {
    this.stage = { kind: 'pick-path' }
    this.missNotified = false
    this.pathHighlight = null
    this.hoverHighlight = null
    this.hoverKey = null
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
   * Resolve a path-stage pick. A sketch edge expands to its whole connected
   * island (the shape the user drew — the kernel refuses a branching island
   * with its own guidance) and is always a usable target — sketch edges carry
   * world coordinates and are NOT frame-gated (only the face branch is).
   *
   * A solid face means "run around this face's outer boundary", but ONLY when
   * it is a face the sweep can place correctly (plain, top-level, non-
   * instanced — the FACE FRAME GUARD). An ineligible face is reported as such
   * so the click is refused with copy instead of silently sweeping the wrong
   * frame, and so the hover preview never renders on it.
   */
  private _pickPath(ray: Ray): PathPick {
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
        return { kind: 'target', path: { kind: 'edges', sketchHandle, edgeHandles } }
      } catch {
        return { kind: 'target', path: { kind: 'edges', sketchHandle, edgeHandles: [edgeHandle] } }
      }
    }

    const facePick = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (facePick !== undefined) {
      let object: bigint
      let face: bigint
      let instance: bigint | undefined
      try {
        object = facePick.object()
        face = facePick.face()
        instance = facePick.instance()
      } finally {
        facePick.free()
      }
      if (!this._faceFollowable(object, instance)) {
        return { kind: 'ineligible-face', message: this._ineligibleFaceHint(instance) }
      }
      return { kind: 'target', path: { kind: 'face', objectHandle: object, faceHandle: face, instance } }
    }
    return { kind: 'none' }
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
      this.missNotified = false
      this._clearPath()
      this._clearHover()
      this.onCommit(objectId)
    } catch (err) {
      // Typed refusal: keep the picked path so the user can adjust the
      // profile and click again.
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      this.onToast(this._refusalMessage(path, code, rawMsg), code ?? undefined)
    }
  }

  /**
   * Plain-language copy for a refused sweep. Two refusals mean something
   * different when the path is a *solid face* the user clicked directly: the
   * generic drawn-path copy talks about placing the profile on a
   * perpendicular surface, but here the profile is already placed and the
   * FACE is the wrong one — so name the face. Everything else defers to the
   * shared kernel-error table (kernelErrors.ts), the one surfacing path.
   */
  private _refusalMessage(path: PathTarget, code: string | null, rawMsg: string): string {
    if (path.kind === 'face') {
      if (code === 'ProfileNotPerpendicular') {
        return 'That face is parallel to the profile — pick the flat face the profile stands across, not one it runs along.'
      }
      if (code === 'PathTooTight') {
        return 'That face is thinner than the profile is deep — pick a wider face, or use a shallower profile.'
      }
    }
    return kernelErrorMessage(code ?? 'Unknown', rawMsg)
  }

  /** Toast a path-stage miss once per run of empty clicks (anti-spam). */
  private _notifyMiss(message: string): void {
    if (this.missNotified) return
    this.missNotified = true
    this.onToast(message)
  }

  /** Draw the picked path as the persistent highlight (viewport ephemera). */
  private _highlightPath(path: PathTarget): void {
    this._clearPath()
    // Committing to a path retires the pre-click hover preview.
    this._clearHover()
    const { points } = this._targetHighlight(path)
    if (points.length === 0) return
    this.pathHighlight = this._buildLines(points, PATH_COLOR, 996)
    this.preview.add(this.pathHighlight)
  }

  /** Show the pre-click hover preview of the target under the cursor. */
  private _showHover(key: string, points: number[]): void {
    if (key === this.hoverKey) return // same target — nothing to rebuild
    this._clearHover()
    if (points.length === 0) return
    this.hoverHighlight = this._buildLines(points, HOVER_COLOR, 995)
    this.preview.add(this.hoverHighlight)
    this.hoverKey = key
  }

  private _clearPath(): void {
    this._dispose(this.pathHighlight)
    this.pathHighlight = null
  }

  private _clearHover(): void {
    this._dispose(this.hoverHighlight)
    this.hoverHighlight = null
    this.hoverKey = null
  }

  private _dispose(obj: THREE.LineSegments | null): void {
    if (obj === null) return
    this.preview.remove(obj)
    obj.geometry.dispose()
    const mat = obj.material
    if (Array.isArray(mat)) mat.forEach((m) => m.dispose())
    else mat.dispose()
  }

  private _buildLines(points: number[], color: number, renderOrder: number): THREE.LineSegments {
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(points), 3))
    const mat = new THREE.LineBasicMaterial({ color, depthTest: false })
    const lines = new THREE.LineSegments(geo, mat)
    lines.renderOrder = renderOrder
    return lines
  }

  /**
   * The highlight geometry for a path target: a flat list of world-space
   * line-segment endpoints (pairs) tracing the swept loop/chain, plus a
   * stable identity key so the hover preview only rebuilds when the target
   * changes. A stale handle yields no points (the commit will surface the
   * error instead of a misleading highlight).
   */
  private _targetHighlight(path: PathTarget): { key: string; points: number[] } {
    const points: number[] = []
    try {
      if (path.kind === 'edges') {
        for (const edge of path.edgeHandles) {
          const ends = this.wasmScene.sketch_edge_endpoints(path.sketchHandle, edge)
          if (ends !== undefined && ends.length >= 6) {
            points.push(ends[0], ends[1], ends[2], ends[3], ends[4], ends[5])
          }
        }
        const key = `edges:${path.sketchHandle}:${[...path.edgeHandles].sort().join(',')}`
        return { key, points }
      }
      const loop = this.wasmScene.face_boundary(path.objectHandle, path.faceHandle)
      for (let i = 0; i < loop.length; i += 3) {
        const j = (i + 3) % loop.length
        points.push(loop[i], loop[i + 1], loop[i + 2], loop[j], loop[j + 1], loop[j + 2])
      }
      // The instance is part of the key so it can never collide across two
      // placements of one definition (defensive — a followable face is always
      // a plain world object, so this is `world` in practice).
      const at = path.instance === undefined ? 'world' : path.instance.toString()
      return { key: `face:${path.objectHandle}:${path.faceHandle}:${at}`, points }
    } catch {
      return { key: 'stale', points: [] }
    }
  }
}
