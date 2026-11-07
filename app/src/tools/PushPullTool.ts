/**
 * PushPullTool — hover-highlight + drag to extrude/push-pull.
 *
 * Gesture (two-click mode):
 *   1. Hover: snap() → highlight face when on-face snap with element
 *   2. First click: enter drag mode, record anchor point + normal axis
 *   3. Move: project ray onto normal axis from anchor → ghost preview height
 *   4. Second click: commit extrude_region or push_pull
 *   5. Esc: cancel
 *
 * Calls onCommit after each successful commit.
 * Throws kernel "CODE: message" errors as toasts via onToast.
 */

import { t } from '@lingui/core/macro'
import * as THREE from 'three'
import type { Tool, Snap, EditContext, SnapConstraint } from './types'
import { editContextEq } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { projectRayOntoAxis, applyAffine3x4, transformNormalThroughPose } from '../viewport/geoHelpers'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { editLengthBuffer, isLengthInputKey } from './moveInput'
import { RetypeWindow, idleRetypeCapturesKey, retypeStaleMessage } from './retypeWindow'
import { displayedLength } from './displayedValue'
import { readHealthReport } from '../health/healthReport'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { buildSweptPrismPreview, clearPreview } from './transformPreview'
import {
  readPreviewTint,
  PREVIEW_EXTRUDE_NEW_FALLBACK,
  PREVIEW_EXTRUDE_NEW_TOKEN,
  PREVIEW_PUSHPULL_FALLBACK,
  PREVIEW_PUSHPULL_TOKEN,
} from './previewTint'
import { defaultFaceEligible, ineligibleFaceHint, sessionIsOpen, worldFaceNormal, EDIT_CONTEXT_HINT_CODE, FacePickCache, type FaceEligible, type RawFacePick } from './faceDraw'
import { ctxOfFace, ctxOfSketch, readPlaneUnits, type PlaneUnitsScene } from './planeUnits'
import { pickPolicyGeneration, pickRenderMode } from './pickPolicy'
import { regionUnderRay } from './regionPick'

/** Snap kinds whose point is a deliberate depth reference for push/pull — the
 * cursor was parked on a real feature. `on-face` is excluded on purpose: it
 * fires continuously during a drag and would hijack the free-drag depth.
 * `on-axis` is excluded too: the drag carries no anchor, so the only axis
 * candidates are the drawing axes through the origin, and the point on one
 * of those nearest the cursor ray is a fact about where the eye happens to
 * sit, not a depth the user chose — pulling a ground rectangle up past the
 * blue axis flipped the preview BELOW the ground the instant the cursor
 * crossed the axis line, and crossing the red or green axis dropped it to
 * zero. A guide line stays: the user placed it on purpose. */
const HARD_SNAP_KINDS = new Set([
  'endpoint',
  'center',
  'quadrant',
  'tangent',
  'midpoint',
  'intersection',
  'on-edge',
  'on-intersection-line',
  'on-guide',
])

/** Metres of perpendicular depth below which a hard snap counts as lying ON
 *  the plane of the face being pushed — the face's own boundary edges, the
 *  midpoints of those edges, a neighbouring coplanar face's corner. Such a
 *  point names the plane the face is already in, so "pull to it" means a
 *  zero-length push/pull: never what a drag past it intends, and exactly
 *  what made a steady pull up from a ground rectangle drop to 0 m every time
 *  the cursor crossed one of the rectangle's own edges. The kernel's plane
 *  tolerance scale (`GROUND_PLANE_EPS`), well below any drawn feature. */
const COPLANAR_SNAP_EPS_M = 1e-9

/** Ray depth (metres) a drawn sketch region must be NEARER than the object
 *  face on the same ray to take the pick over it — a real "in front of" by
 *  more than rounding, so a region drawn ON a face (coplanar with it) still
 *  yields to the face it lies on. */
const REGION_IN_FRONT_EPS_M = 1e-9

/**
 * Minimum inward (negative) live-drag distance, in meters, that counts as a
 * deliberate "push inward" when committing a typed push/pull. A real inward
 * drag moves the cursor by many pixels — millimeters or more of world
 * distance — so this sits far below any intentional pull yet far above the
 * ~1e-15 m floating-point wobble a still cursor picks up from the camera
 * projection. Its sole job is to stop that wobble from flipping the extrude
 * direction when the user typed a distance without dragging (see
 * `_commitFromTyped`).
 */
const MIN_INWARD_DRAG_M = 1e-9

/** A `pick_sketch_region` hit: the region handles plus its ray depth in
 *  metres along the normalized ray (the scale `pick_face` reports in). */
type RegionPick = {
  sketch: bigint
  region: bigint
  depth: number
  /** The region's sketch is a detached shape (separate-shapes design D8):
   *  drawn to stay apart from the face it lies on. */
  detached: boolean
}

/**
 * Whether a picked sketch region sits IN FRONT of the object face picked on
 * the same ray — nearer by more than `REGION_IN_FRONT_EPS_M`. A drawn region
 * is opaque to the pick exactly as it is to snapping (the inference crate's
 * occlusion cull already counts sketch faces): a vertical rectangle standing
 * on a cube's top edge, seen with the cube's top face visible THROUGH it,
 * must push/pull as the rectangle everywhere on its fill, not as the cube
 * wherever the cube happens to lie behind (GitHub issue 13 — "On Face" on
 * the part of the rectangle with nothing behind it, "On Plane" and the CUBE
 * extruding on the rest). No face on the ray at all also counts as "in
 * front"; no region never does.
 */
export function regionInFrontOfFace(
  region: RegionPick | null,
  face: RawFacePick | null,
  faceEligible = true,
  regionInScope: boolean | null = null,
): boolean {
  if (region === null) return false
  if (face === null) return true
  // Shaded or X-ray with an editing scope: the kernel's picks landed on the nearest
  // thing INSIDE the scope, falling back to the nearest of any owner when
  // nothing in scope lay under the ray. When exactly one of the two is the
  // in-scope one it wins, whatever the depth — depth only orders two
  // candidates of the same standing.
  if (regionInScope !== null) {
    if (faceEligible && !regionInScope) return false
    if (!faceEligible && regionInScope) return true
  }
  // A detached shape lying on a face is what the cursor is on: it was drawn
  // to stay apart from the face, so a tie goes to it (D8). So does free
  // drawing lying ON a face that cannot be pushed from here (a group's or
  // component's face, drawn on from outside). Either pulls into a new object
  // of its own.
  if (region.detached || !faceEligible) return region.depth <= face.depth + REGION_COPLANAR_EPS_M
  return region.depth < face.depth - REGION_IN_FRONT_EPS_M
}

/** How far behind a non-pushable face a region may lie and still count as
 *  lying on it (metres along the ray): the two are the same plane, picked
 *  through two code paths. */
const REGION_COPLANAR_EPS_M = 1e-7

export type PushPullTarget =
  | {
      kind: 'region'
      sketchHandle: bigint
      regionHandle: bigint
      normal: [number, number, number]
      /** The instance to extrude THROUGH (component-edit-parity.md phase
       *  A2) — set when the region's sketch is a member of the entered
       *  instance's definition, else null (a world sketch, extruded via the
       *  plain `extrude_region`). Doubles as the ghost preview's pose source
       *  (`_drawGhostPreview`): `region_boundary` answers in DEFINITION-local
       *  coordinates for a def-owned sketch, so the swept-prism ghost maps
       *  it through this instance's pose before drawing. */
      instance: bigint | null
    }
  | {
      kind: 'face'
      objectHandle: bigint
      faceHandle: bigint
      normal: [number, number, number]
      /** The instance the face was picked THROUGH (component-edit-parity.md
       *  phase A2) — null for a plain world object. `face_boundary` answers
       *  in the object's own LOCAL (definition) frame for a component
       *  member, exactly like `region_boundary` above, so the ghost preview
       *  maps it through this same pose. */
      instance: bigint | null
    }

export type OnPushPullCommit = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void
export type OnExtrudeAsNewModeChange = (on: boolean) => void

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'dragging'
      target: PushPullTarget
      anchor: [number, number, number]
      /** Last computed signed distance */
      distance: number
    }

/**
 * The push/pull just committed, kept in a `RetypeWindow` (retypeWindow.ts)
 * so a distance typed AFTER the commit click redoes it at that distance —
 * SketchUp's "pull roughly, then type the exact depth". A positive value
 * keeps the committed direction, a negative one flips it. `extrudeAsNew`
 * is the Ctrl mode in force at the commit, re-applied for the redo. A
 * REGION target's handle does not survive the redo's undo — extruding a
 * region consumes it, and the kernel's undo re-mints the sketch's edges
 * with fresh handles (`restore_edges`: "callers re-query") — so the region
 * is re-picked at the remembered click point before every re-commit.
 */
type RetypeSpec = {
  target: PushPullTarget
  distance: number
  extrudeAsNew: boolean
  /** The first click's point on the face/region — where a REGION target is
   *  re-picked after the retype's undo (see `_commitHot`). */
  anchor: [number, number, number]
}

export class PushPullTool implements Tool {
  readonly name = 'Push/Pull'
  /** Click-drag-release works as well as click-click (Tool.dragCommits). */
  readonly dragCommits = true

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    if (this.stage.kind === 'idle') {
      // A face the tool cannot push explains itself while hovered — the
      // same line the refused click toasts, shown BEFORE the click, in the
      // status bar and (`hoverHint`) beside the cursor.
      if (this._hoverRefusal !== null) return this._hoverRefusal
      if (this.retype.isOpen) {
        return t`Type an exact distance to redo the push/pull you just made — or click another face; double-click repeats the last distance.`
      }
      return this.extrudeAsNewMode
        ? t`Click a face to push or pull it — double-click repeats the last distance. Ctrl is on — extrudes a new object.`
        : t`Click a face to push or pull it — double-click repeats the last distance.`
    }
    return this.extrudeAsNewMode
      ? t`Move to extrude a NEW object, click to commit — or type an exact distance. Tap Ctrl to push/pull instead.`
      : t`Move to extrude, click to commit — or type an exact distance. Tap Ctrl to extrude a new object instead.`
  }

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnPushPullCommit
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** VCB buffer — raw string being typed by the user. While IDLE with the
   *  retype window open it is the post-commit distance being typed. */
  private typed: string = ''

  /** The push/pull just committed, redoable at a typed distance until the
   *  next pointer action, Escape, tool switch, or other document mutation
   *  (see `RetypeSpec` and retypeWindow.ts). */
  private readonly retype: RetypeWindow<RetypeSpec>

  /** The snap last seen on hover (for highlight logic) */
  lastSnap: Snap | null = null

  /** Per-pointer-event `pick_face` memo for `snapConstraint`'s idle hover —
   *  see `FacePickCache` in faceDraw.ts (same pattern as the draw tools). */
  private readonly _pickCache = new FacePickCache()

  /**
   * Last successfully committed signed distance (meters, along the face
   * normal — negative recesses), session-lived on this tool instance:
   * double-click repeats it on whatever face/region is clicked next. `null`
   * until the first commit this activation; a fresh `PushPullTool` (every
   * tool switch mints one — see Viewport's `makePushPullTool`) starts with
   * no repeat armed, matching SketchUp's per-activation memory.
   */
  private lastCommittedDistance: number | null = null

  /**
   * Durable per-gesture toggle (tap Ctrl/Cmd to flip — the Move-copy idiom:
   * `MoveTool.ts` reads Alt the same way in `onKey`, not a live per-click
   * modifier read like Paint's whole-object fill in `Viewport.tsx`). A live
   * read doesn't fit here: Push/Pull is a click-drag-click gesture, and the
   * modifier must survive released keys through the whole drag exactly like
   * Move's copy toggle, not just whatever was held at one instant. While
   * on, a solid-face commit routes through `extrude_face_as_new_object`
   * (a NEW coincident object, source untouched) instead of `push_pull`.
   */
  private extrudeAsNewMode: boolean = false
  private onExtrudeAsNewModeChange: OnExtrudeAsNewModeChange

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnPushPullCommit,
    onToast: OnToast,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
    onExtrudeAsNewModeChange: OnExtrudeAsNewModeChange = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onCommit = onCommit
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
    this.retype = new RetypeWindow(wasmScene)
    this.onExtrudeAsNewModeChange = onExtrudeAsNewModeChange
  }

  // ── Optional Tool interface extensions ─────────────────────────────────────

  capturingInput(): boolean {
    return this.stage.kind === 'dragging'
  }

  /** Armed for Escape's purposes while dragging or while a post-commit
   *  distance is being typed (see `toolHasArmedGesture` in tools/types.ts). */
  hasArmedGesture(): boolean {
    return this.capturingInput() || this.typed !== ''
  }

  /** Per-key capture (see Tool.capturesKey): a drag keeps the whole
   *  keyboard as before; the IDLE retype window takes only what a typed
   *  distance needs (`idleRetypeCapturesKey`). */
  capturesKey(key: string): boolean {
    if (this.capturingInput()) return true
    if (!this.retype.isOpen) return false
    return idleRetypeCapturesKey(key, this.typed, isLengthInputKey)
  }

  /** Quietly close the retype window — the host calls this before an
   *  explicit undo/redo/delete (`disarmActivePostCommitWindow`). */
  disarmRetype(): void {
    this.retype.close()
    if (this.stage.kind === 'idle' && this.typed !== '') {
      this.typed = ''
      this.onMeasurementCb('')
    }
  }

  /**
   * Double-click repeats `lastCommittedDistance` on the clicked face/region
   * (SketchUp parity), honoring the same validity refusals as any other
   * commit (`_commit` already toasts a kernel refusal). The gesture's FIRST
   * click ran through the normal `onPointerDown` and picked/validated a
   * target into `this.stage` — the Viewport skips routing the second,
   * phantom pointerdown once a tool implements `onDoubleClick` (see the
   * Tool interface doc) — so this reuses that already-validated target
   * rather than re-picking. Always resets the phantom drag click one
   * started, so it never lingers as a stray in-progress gesture regardless
   * of the outcome.
   *
   * Returns `false` — falling through to the Viewport's default "enter
   * context" gesture, exactly PushPullTool's behavior before this method
   * existed — when there is nothing to repeat: no target was picked (an
   * ineligible-face click already toasted and stayed idle), or nothing has
   * been committed yet this tool activation. Once a repeat IS attempted,
   * returns `true` unconditionally (even on a refused commit) — a
   * double-click is a deliberate repeat attempt, not a fallback-worthy miss.
   */
  onDoubleClick(_snap: Snap | null, _ray: Ray): boolean {
    const stage = this.stage
    this.stage = { kind: 'idle' }
    this.typed = ''
    clearPreview(this.preview)
    this.onMeasurementCb('')

    if (stage.kind !== 'dragging' || this.lastCommittedDistance === null) {
      return false
    }
    this._commitAndArm(stage.target, this.lastCommittedDistance, stage.anchor)
    return true
  }

  /**
   * Idle hover only (design v1.1 Lane E "Push/Pull face-first"): constrain
   * the next snap query to the plane of the face/region under the cursor,
   * restricted to its `on-face` point (`facesOnly`). Without this,
   * inference's hard total rank order (`crates/inference` `rank_group`,
   * pinned by `inference_specs.rs:2569`) means a hover a few pixels from a
   * box corner resolves — and a drag from there would push/pull from —
   * that corner's `endpoint` chip instead of the face itself, since
   * `on-face` can never outrank a precise point kind. `SnapService.resolve`
   * honors `facesOnly` by replacing any non-`on-face` winner with the
   * `constraintPlane` ray intersection (kind `'plane'`), so the hover cue
   * is always the on-face dot under the cursor on the face that will
   * actually be pushed — no endpoint/midpoint/edge chip ever wins the pick.
   *
   * Mirrors `onPointerDown`'s two paths exactly, read-only — including its
   * FAIL-CLOSED rule: an INELIGIBLE face (grouped/instanced, hit but
   * rejected by `_isEligible`) offers no constraint at all, the same way
   * `onPointerDown` consumes that click with a toast rather than falling
   * through to Path B. Without this, a hover over a group's face would let
   * a sketch region BEHIND the group become the constraint plane, so the
   * hover cue would land on the ground plane through the solid instead of
   * showing nothing — a hover-time lie about what a click would do. Path A
   * (`pick_face` + the shared eligibility policy, memoized per ray via
   * `FacePickCache` like the draw tools) runs first; Path B
   * (`pick_sketch_region`, instance-scoped the same way, also memoized per
   * ray) only runs when Path A found NO face at all (`rawPickFor` null) —
   * never when it found one and rejected it. `null` — no constraint — once
   * a drag has anchored (`this.stage.kind !== 'idle'`): the drag itself
   * must keep `HARD_SNAP_KINDS` working unconstrained ("pull to that
   * edge"), and the anchor is already fixed by then regardless.
   */
  snapConstraint(ray?: Ray): SnapConstraint | null {
    if (this.stage.kind !== 'idle' || ray === undefined) return null

    // Path A: the same eligible-face pick `onPointerDown` would commit to.
    const eligible = this._pickCache.pickFor(this.wasmScene, ray, (object, instance) =>
      this._isEligible(object, instance))
    const raw = this._pickCache.rawPickFor(ray)
    // Path B's region, picked up front so the two paths can be ordered by
    // DEPTH (`regionInFrontOfFace`): a region nearer the eye than the face
    // on the same ray is what the cursor is on, whatever lies behind it —
    // eligible face or not. Scoped exactly as `onPointerDown` scopes it:
    // top-level (or the entered instance's own def-owned sketches) only,
    // never inside an OBJECT editing context.
    const activeInstance = this._activeInstance
    const region = this._activeContext === null ? this._pickRegionFor(ray, activeInstance) : null
    if (!regionInFrontOfFace(region, raw, eligible !== null, this._regionInScope(region, activeInstance))) {
      if (eligible !== null) {
        const a = this.wasmScene.face_plane(eligible.object, eligible.face)
        return {
          constraintPlane: { point: [a[0], a[1], a[2]], normal: [a[3], a[4], a[5]] },
          facesOnly: true,
        }
      }
      // A face WAS hit but rejected by eligibility — fail closed exactly
      // like onPointerDown's ineligible branch (toast, click consumed): no
      // constraint, never Path B's sketch-region fallback BEHIND it.
      return null
    }

    // Path B: the hovered sketch region's own plane — in front of any face
    // on the ray, or the only thing on it.
    if (region === null) return null
    const plane = this.wasmScene.sketch_plane(region.sketch)
    if (plane === undefined) return null // stale handle — no constraint
    let point: [number, number, number] = [plane[0], plane[1], plane[2]]
    let normal: [number, number, number] = [plane[3], plane[4], plane[5]]
    if (activeInstance !== null) {
      // A def-owned sketch's plane is DEFINITION-LOCAL (`sketch_plane` has
      // no `_in_instance` sibling) — map both the point (full affine) and
      // the normal (inverse-transpose of the linear part, via the same
      // `transformNormalThroughPose` helper `worldFaceNormal` above uses)
      // through the instance's real pose into world space.
      const pose = this.wasmScene.instance_pose(activeInstance)
      if (pose === undefined) return null
      point = applyAffine3x4(pose, point)
      const mappedNormal = transformNormalThroughPose(pose, normal)
      if (mappedNormal === null) return null
      normal = mappedNormal
    }
    return { constraintPlane: { point, normal }, facesOnly: true }
  }

  /**
   * The face to push/pull for a ray that struck `face` of `object`, resolved
   * through the plane's listing (unified drafting §4.6): the region under the
   * cursor names the face it lies over (`over.face`), so a region drawn on
   * the face pushes as its own sub-face whichever face the ray struck. A
   * region over the object with NO face (`over.face` null: a sliver the
   * solid cannot hold) answers `{ refuse: <code> }`: RegionSliver, or
   * RegionNotPushable by the listing's `over.why`. Free regions never
   * reach here — they resolve through Path B (`extrude_region`). Anything
   * the listing cannot say (no listing, no region under the ray, a stale
   * handle) leaves the struck face as it was.
   */
  private _routeThroughListing(
    object: bigint,
    face: bigint,
    instance: bigint | undefined,
    ray: Ray,
  ): bigint | { refuse: 'RegionSliver' | 'RegionNotPushable' } {
    let hit: ReturnType<typeof regionUnderRay> = null
    try {
      const ctx = ctxOfFace(this.wasmScene, object, face, instance ?? null)
      if (ctx !== null) hit = regionUnderRay(this.wasmScene as unknown as PlaneUnitsScene, ctx, ray, 0)
    } catch {
      return face
    }
    const over = hit?.region.over ?? null
    if (over === null || over.object !== object) return face
    if (over.face !== null) return over.face
    // oxlint-disable-next-line lingui/no-unlocalized-strings -- kernel refusal codes, not copy
    return { refuse: over.why === 'sliver' ? 'RegionSliver' : 'RegionNotPushable' }
  }

  /**
   * Per-pointer-event `pick_sketch_region`/`pick_sketch_region_in_instance`
   * memo, mirroring `FacePickCache`'s ray-reference keying (faceDraw.ts) —
   * `snapConstraint`'s hover probe and `onPointerDown`'s commit are called
   * back-to-back by the Viewport with the SAME `Ray` object, so this keeps
   * Path B to one pick per ray too, not one from each caller. Keyed on the
   * active instance as well as the ray: a context change between two calls
   * that happen to share a (stale) ray reference must still re-pick.
   */
  private _regionPickCache: {
    ray: Ray
    instance: bigint | null
    /** `pickPolicyGeneration()` at pick time: a render-mode or editing-scope
     *  change invalidates the memo even when the ray is the same object. */
    gen: number
    region: RegionPick | null
  } | null = null

  /**
   * Whether the picked region is inside the editing scope, or null when the
   * question does not apply (Wireframe, or nothing is open for editing) or
   * cannot be answered (the region is not in its plane's listing — then the
   * plain depth rule stands). Only a non-Wireframe mode with a scope makes the kernel's
   * picks prefer in-scope candidates, so only there can a region and a face
   * differ in standing. A region picked through the entered instance
   * (`_in_instance`) is in scope by construction; a world region is in scope
   * when it is drawn on an eligible object's face, or is a free sketch of
   * the open session.
   */
  private _regionInScope(region: RegionPick | null, activeInstance: bigint | null): boolean | null {
    if (region === null || pickRenderMode() === 'wireframe') return null
    if (activeInstance !== null) return true
    try {
      if (!sessionIsOpen(this.wasmScene)) return null
      const ctx = ctxOfSketch(this.wasmScene, region.sketch)
      if (ctx === null) return null
      const pu = readPlaneUnits(this.wasmScene as unknown as PlaneUnitsScene, ctx)
      const hit = pu?.regions.find((r) => r.extrude?.sketch === region.sketch && r.extrude.region === region.region)
      if (hit === undefined) return null
      if (hit.over !== null) return this._isEligible(hit.over.object, undefined)
      return Array.from(this.wasmScene.explode_session_sketches() ?? []).includes(region.sketch)
    } catch {
      return null
    }
  }

  private _pickRegionFor(ray: Ray, activeInstance: bigint | null): RegionPick | null {
    if (
      this._regionPickCache !== null &&
      this._regionPickCache.ray === ray &&
      this._regionPickCache.instance === activeInstance &&
      this._regionPickCache.gen === pickPolicyGeneration()
    ) {
      return this._regionPickCache.region
    }
    const regionPick =
      activeInstance !== null
        ? this.wasmScene.pick_sketch_region_in_instance(
            activeInstance,
            ray.origin[0], ray.origin[1], ray.origin[2],
            ray.direction[0], ray.direction[1], ray.direction[2],
          )
        : this.wasmScene.pick_sketch_region(
            ray.origin[0], ray.origin[1], ray.origin[2],
            ray.direction[0], ray.direction[1], ray.direction[2],
          )
    let region: RegionPick | null = null
    if (regionPick !== undefined) {
      try {
        // `pick_sketch_region`'s depth is the ray parameter for the RAW
        // direction; `pick_face`'s is metres along the normalized ray.
        // Bring the world region's onto the same scale so the two compare.
        // The `_in_instance` sibling already answers in world metres (it
        // multiplies its definition-local parameter by the world
        // direction's length itself) — scaling it again would double-count
        // any non-unit direction.
        const dirLen = activeInstance !== null ? 1 : Math.hypot(ray.direction[0], ray.direction[1], ray.direction[2])
        region = { sketch: regionPick.sketch(), region: regionPick.region(), depth: regionPick.depth() * dirLen, detached: regionPick.detached() }
      } finally {
        regionPick.free()
      }
    }
    this._regionPickCache = { ray, instance: activeInstance, gen: pickPolicyGeneration(), region }
    return region
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    this.lastSnap = snap

    if (this.stage.kind === 'dragging') {
      const { target, anchor } = this.stage
      const distance = this._axisDistance(snap, ray, anchor, target.normal)
      this.stage = { ...this.stage, distance }
      this._drawGhostPreview(anchor, target.normal, distance)
      this._reportMeasurement(distance)
    } else {
      this._updateHoverRefusal(ray)
    }
    // Hover highlight is handled via CueLayer for on-face snaps (M1 shortcut
    // per docs/dev/DEVELOPMENT.md: show snap marker at face location; per-face highlight
    // requires face→triangle table in MeshJs which is a WASM_API addendum).
  }

  /** The hovered face's refusal while idle (see `_updateHoverRefusal`), or
   *  null when the cursor is over something the tool can act on, nothing,
   *  or a gesture is in progress. */
  private _hoverRefusal: string | null = null

  /** Cursor-side guidance (Tool.hoverHint): the same line the refused
   *  click would toast, shown while the face is merely hovered. */
  hoverHint(): string | null {
    return this.stage.kind === 'idle' ? this._hoverRefusal : null
  }

  /**
   * Resolves whether the face under `ray` would refuse a click, exactly as
   * `onPointerDown`'s Path A decides it — the same memoized pick
   * (`_pickCache`, keyed on this ray object) and the same region-in-front
   * rule, so the hover never says one thing and the click another. A
   * component face hovered outside its edit session (the common
   * SketchUp-habit case: push on a copied component without entering it)
   * is what this exists for; a grouped face and an out-of-scope face get
   * their own lines from the same helper.
   */
  private _updateHoverRefusal(ray: Ray): void {
    let next: string | null = null
    try {
      const eligible = this._pickCache.pickFor(this.wasmScene, ray, (object, instance) =>
        this._isEligible(object, instance))
      const raw = this._pickCache.rawPickFor(ray)
      if (raw !== null && eligible === null) {
        const activeInstance = this._activeInstance
        const region = this._activeContext === null ? this._pickRegionFor(ray, activeInstance) : null
        if (!regionInFrontOfFace(region, raw, false, this._regionInScope(region, activeInstance))) next = this._ineligibleFaceHint(raw.instance)
      }
    } catch {
      next = null
    }
    this._hoverRefusal = next
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    // Any pointer action ends the retype window — the next push/pull begins.
    this.disarmRetype()
    // The click decides for itself (and re-toasts a refusal); a hover memo
    // left standing here would survive a double-click that opens the very
    // component it describes.
    this._hoverRefusal = null
    if (this.stage.kind === 'idle') {
      let target: PushPullTarget | null = null
      let anchor: [number, number, number] = [0, 0, 0]

      // --- Path A: ray-cast for the nearest object face (ignores snap priority) ---
      // pick_face bypasses the drawing snap bias toward vertices/edges, so it
      // reliably returns the surface under the cursor even when snap prefers a
      // nearby endpoint.  We call this FIRST; Path B only fires when no object
      // face is hit (bare ground or no objects yet).
      //
      // Reuses `_pickCache` — the SAME memoized pick `snapConstraint(ray)`
      // just computed for this exact `ray` object (the Viewport calls the
      // two back-to-back with the identical `Ray`), so a click costs one
      // `pick_face`, not two. A direct `onPointerDown` call with no prior
      // `snapConstraint` for this ray (every existing unit test) is still
      // correct: `pickFor`'s cache simply misses and it raycasts fresh.
      const eligible = this._pickCache.pickFor(this.wasmScene, ray, (object, instance) =>
        this._isEligible(object, instance))
      const raw = this._pickCache.rawPickFor(ray)
      // The region pick runs FIRST here too, so a drawn region nearer the
      // eye than the face behind it takes the click (`regionInFrontOfFace`,
      // GitHub issue 13) — Path A is skipped entirely in that case, exactly
      // as `snapConstraint` skips it at hover time. The two must agree.
      const activeInstance = this._activeInstance
      const region = this._activeContext === null ? this._pickRegionFor(ray, activeInstance) : null
      const regionWins = regionInFrontOfFace(region, raw, eligible !== null, this._regionInScope(region, activeInstance))
      if (raw !== null && !regionWins) {
        // Same face-eligibility policy as the draw tools (faceDraw.ts): at
        // the top level only PLAIN objects are directly push/pullable —
        // faces inside a group or component instance keep their explicit
        // editing step. Inside an editing context only that context's
        // scope is editable, so isolated editing can't disturb neighbors.
        if (eligible !== null) {
          const objectHandle = eligible.object
          const instanceHandle = raw.instance
          // The face's plane lists what is drawn on it; the region under the
          // cursor says which face to push — or that the solid cannot hold
          // it (a pinch or sliver of an owned sketch).
          const routed = this._routeThroughListing(objectHandle, eligible.face, instanceHandle, ray)
          if (typeof routed === 'object') {
            this.onToast(kernelErrorMessage(routed.refuse, ''), routed.refuse)
            return
          }
          const faceHandle = routed
          // `face_normal` answers in `objectHandle`'s own LOCAL frame — the
          // instance the face was actually picked THROUGH (`instanceHandle`,
          // not necessarily `this._activeInstance`: an injected eligibility
          // predicate could in principle allow a different one) carries the
          // real, un-baked pose that maps it into world space. `null` means
          // a stale instance or a degenerate mapped normal — treated exactly
          // like a miss (component-edit-parity.md phase A2).
          const normal = worldFaceNormal(this.wasmScene, objectHandle, faceHandle, instanceHandle ?? null)
          if (normal === null) return
          // The anchor is the (facesOnly) snap point — `snapConstraint`
          // above constrains every idle hover to this same face's plane,
          // so `snap` is always on it by construction, never a ground hit
          // through the face. A null snap can no longer happen here in
          // practice (the constraint plane's ray intersection always
          // produces one); ray-origin is a defensive last resort only —
          // see PushPullTool.test's "anchor lies on the face plane" spec.
          anchor = snap !== null ? [snap.x, snap.y, snap.z] : [...ray.origin]
          target = { kind: 'face', objectHandle, faceHandle, normal, instance: instanceHandle ?? null }
        } else {
          // FAIL CLOSED: an ineligible face CONSUMES the click. Falling
          // through to Path B would let a sketch region along the same ray
          // (a ground sketch behind the group — ordinary mid-modeling
          // state) silently start a drag and extrude geometry the user
          // did not aim at. Refuse explicably instead. `snapConstraint`
          // fails closed the same way at hover time (see its own doc) —
          // the two must never disagree.
          this.onToast(this._ineligibleFaceHint(raw.instance), EDIT_CONTEXT_HINT_CODE)
          return
        }
      }

      // --- Path B: a sketch region — in front of any face on the ray, or
      //     the only thing on it (bare ground click, or no objects yet) ---
      // `pick_sketch_region` resolves the smallest
      // containing region across ALL live sketches kernel-side (nested rings
      // resolve to the innermost — the app no longer has to walk sketch_regions
      // + region_boundary + point-in-polygon itself). `pick_sketch_region`
      // only ever walks WORLD-tree sketches (`Document::sketch_ids()`
      // deliberately excludes definition-owned ones) — it can NEVER find a
      // region drawn inside a component's own definition, so an instance
      // context calls the `_in_instance` sibling instead, which is scoped to
      // (and pose-maps the ray for) exactly that definition's own sketches;
      // there is nothing left to filter by membership afterward, unlike the
      // stale approach this replaced.
      // Region extrusion is a top-level (or instance-context) act; suppress
      // it inside an OBJECT editing context (component-edit-parity.md phase
      // A2 — an instance context now has its own def-owned regions too).
      //
      // Reuses `_pickRegionFor` — the SAME memoized pick `snapConstraint(ray)`
      // just computed for this exact `ray` (mirrors Path A's `_pickCache`
      // reuse above), so this costs one `pick_sketch_region` per ray too.
      if (target === null && this._activeContext === null) {
        if (region !== null) {
          const sketchHandle = region.sketch
          const regionHandle = region.region
          // The kernel extrudes along the profile plane's own normal, so
          // the drag axis/ghost must match it (sketches on any plane —
          // Phase 1). A stale handle between the region pick and this
          // query is a miss, not a fallback to ground.
          const plane = this.wasmScene.sketch_plane(sketchHandle)
          if (plane === undefined) return // stale handle — treat as a miss
          let normal: [number, number, number] = [plane[3], plane[4], plane[5]]
          if (activeInstance !== null) {
            // A def-owned sketch's plane is DEFINITION-LOCAL — `sketch_plane`
            // has no `_in_instance` sibling, so the normal is pose-mapped
            // here the same approximate way `sketchGesture.ts`'s
            // `isStillOnPlane` does (linear part, re-normalized): exact for
            // rotation/uniform-scale/mirror/translation, and even a non-
            // uniform-scale pose can only skew the drag axis/preview, never
            // the actual commit, which goes through the kernel's own exact
            // pose⁻¹ regardless.
            const pose = this.wasmScene.instance_pose(activeInstance)
            if (pose === undefined) return
            const [nx, ny, nz] = normal
            const rnx = pose[0] * nx + pose[1] * ny + pose[2] * nz
            const rny = pose[4] * nx + pose[5] * ny + pose[6] * nz
            const rnz = pose[8] * nx + pose[9] * ny + pose[10] * nz
            const len = Math.hypot(rnx, rny, rnz)
            if (len <= 1e-12) return
            normal = [rnx / len, rny / len, rnz / len]
          }
          // The anchor is the (facesOnly) snap point — see the matching
          // Path A comment above; `snapConstraint`'s region branch
          // constrains every idle hover to this same region's plane.
          anchor = snap !== null ? [snap.x, snap.y, snap.z] : [...ray.origin]
          target = {
            kind: 'region',
            sketchHandle,
            regionHandle,
            normal,
            instance: activeInstance,
          }
        }
      }

      if (target === null) return

      this.typed = ''
      this.stage = { kind: 'dragging', target, anchor, distance: 0 }
    } else if (this.stage.kind === 'dragging') {
      // Second click: commit with current distance
      const { target, anchor, distance } = this.stage
      this.stage = { kind: 'idle' }
      this.typed = ''
      clearPreview(this.preview)
      this.onMeasurementCb('')

      // Final depth at the click — perpendicular to the face, projecting the
      // snapped reference point onto the axis when one is present (see
      // _axisDistance) so e.g. clicking an edge midpoint cuts to exactly that
      // depth rather than the cursor ray's diagonal closest-approach.
      const finalDistance = this._axisDistance(snap, ray, anchor, target.normal)
      const raw = finalDistance === 0 ? distance : finalDistance
      // A depth that follows the cursor (not borrowed from a hard snap)
      // commits the DISPLAYED distance through the typed path — sign rules
      // included — exactly as if typed. A hard-snap depth stays exact.
      const shown = displayedLength(Math.abs(raw))
      if (!this._depthFromHardSnap(snap, anchor, target.normal) && shown > 0) {
        this._commitFromTypedClick(target, raw, shown, anchor)
        return
      }
      this._commitAndArm(target, raw, anchor)
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
      return
    }

    // NB: the Ctrl/Cmd extrude-as-new toggle is NOT handled here — a bare
    // Control/Meta keydown reports ctrlKey/metaKey: true on itself, so the
    // Viewport's generic key path (gated on `!isMod`) never routes it to a
    // tool's onKey (same reason Scale's Ctrl center-anchor toggle and
    // Shift's axis lock each have their own listener). It arrives via
    // `toggleExtrudeAsNew()`, driven by a dedicated Ctrl/Cmd listener in the
    // Viewport.
    // ── Idle retype window (see `RetypeSpec`) ──
    if (this.stage.kind === 'idle' && this.retype.isOpen && this.capturesKey(ev.key)) {
      if (ev.key === 'Enter') {
        const meters = parseLengthToMeters(this.typed)
        this.typed = ''
        this.onMeasurementCb('')
        if (meters !== null) this._retypeDistance(meters)
        return
      }
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
      this.onMeasurementCb(this.typed === '' ? '' : this._typedReadout())
      return
    }

    if (this.stage.kind !== 'dragging') return

    // ── Numeric VCB ──
    if (ev.key === 'Enter') {
      const meters = parseLengthToMeters(this.typed)
      if (meters !== null) {
        this._commitFromTyped(meters)
      }
      return
    }

    // Feed length-input keys (digits, dot, minus, feet/inch/fraction marks,
    // explicit unit-suffix letters, Backspace) into the buffer.
    if (isLengthInputKey(ev.key)) {
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
      // Report the typed buffer as the measurement readout, tagged with the
      // current display unit so the user knows what they're typing in.
      this.onMeasurementCb(this._typedReadout())
    }
  }

  /** The typed-buffer readout, suffixed for metric formats (imperial tokens
   * like `'`/`"` are already visible in the buffer itself). */
  private _typedReadout(): string {
    return typedReadout(this.typed)
  }

  /**
   * Flip the durable extrude-as-new-object mode (SketchUp's Ctrl/Cmd "leave
   * original face", reinterpreted per the tool-parity design). A TAP toggle,
   * not hold: set it before starting a drag, or flip it mid-drag to change
   * what the NEXT click commits. Called by the Viewport's dedicated
   * Ctrl/Cmd listener on a clean tap of either key — see the onKey note
   * above for why this can't ride the generic key path. Autorepeat / the
   * combo-vs-tap distinction are the Viewport listener's concern.
   */
  toggleExtrudeAsNew(): void {
    this.extrudeAsNewMode = !this.extrudeAsNewMode
    this.onExtrudeAsNewModeChange(this.extrudeAsNewMode)
  }

  cancel(): void {
    this.retype.close()
    this.stage = { kind: 'idle' }
    this.typed = ''
    clearPreview(this.preview)
    this.onMeasurementCb('')
    this.lastSnap = null
    // A context change or tool switch arrives without a pointer move; the
    // hovered face's refusal would otherwise outlive the state it described
    // (the chip still saying "double-click it" over a face just entered).
    this._hoverRefusal = null
  }

  /** The pointer left the canvas: nothing is hovered (Tool.onPointerLeave). */
  onPointerLeave(): void {
    this._hoverRefusal = null
  }

  /**
   * Commit from the typed VCB buffer. `dist` is a signed distance in meters,
   * already converted from the display unit — and `editLengthBuffer` keeps a
   * leading `-` as a sign, so `dist` is negative exactly when the user typed
   * one (e.g. `-0.5` → recess by 0.5).
   *
   * The typed value always supplies the MAGNITUDE. The DIRECTION follows a
   * fixed precedence, most deliberate signal first:
   *
   *   1. An explicit typed sign wins outright. A negative `dist` is the user
   *      spelling out "recess / push inward" — the single most deliberate
   *      signal there is — so it beats both the live drag and the outward
   *      default. Typing `-0.5` goes inward even mid-outward-drag.
   *   2. Otherwise (an unsigned, positive magnitude) the direction comes from
   *      the live drag, but only once the drag clears [`MIN_INWARD_DRAG_M`].
   *      Below that threshold the drag supplies no direction: with no
   *      meaningful drag — the user clicked a face or ground region and typed
   *      an exact distance — the ray-projected `distance` is essentially zero,
   *      and its raw sign can flip negative on nothing more than the ~1e-15 m
   *      low-bit wobble of the camera projection matrix (which varies per page
   *      load). So below the threshold we extrude OUTWARD along the face
   *      normal — the unambiguous intent of "type a distance and commit".
   *
   * The four cases: `0.5`+no-drag → out; `-0.5`+no-drag → in (recess);
   * `0.5`+inward-drag → in; `-0.5`+anything → in (explicit sign wins).
   */
  private _commitFromTyped(dist: number): void {
    if (this.stage.kind !== 'dragging') return
    const { target, distance, anchor } = this.stage

    // Inward when the user typed an explicit `-` (dist < 0), OR when a genuine
    // inward drag cleared the noise threshold; outward otherwise. The explicit
    // typed sign wins over the drag — a negative `dist` forces inward no matter
    // which way the live drag was pointing.
    const sign = dist < 0 || distance < -MIN_INWARD_DRAG_M ? -1 : 1
    const signed = Math.abs(dist) * sign

    this.stage = { kind: 'idle' }
    this.typed = ''
    clearPreview(this.preview)
    this.onMeasurementCb('')

    this._commitAndArm(target, signed, anchor)
  }

  /** The current editing context (component-edit-parity.md phase A1) — a
   *  single value replacing the old `_activeContext`/`_activeComponent`/
   *  `_contextScoped` duck-typed fields. The getters below preserve their
   *  exact old read semantics so the rest of this file is unchanged. */
  private _editContext: EditContext = { kind: 'top' }
  setEditContext(ctx: EditContext): void {
    if (editContextEq(ctx, this._editContext)) return
    this._editContext = ctx
    this.cancel()
  }

  /** The entered OBJECT id, or null — unchanged meaning from the old
   *  `_activeContext` field. When set, push/pull only acts on that object's
   *  faces (scoped editing). */
  private get _activeContext(): bigint | null {
    return this._editContext.kind === 'object' ? this._editContext.id : null
  }

  /** The entered component INSTANCE id, or null (component-edit-parity.md
   *  phase A2) — new: `extrude_region_in_instance` needs the instance (not
   *  just the definition) to map a region's birth through its pose. */
  private get _activeInstance(): bigint | null {
    return this._editContext.kind === 'instance' ? this._editContext.id : null
  }

  /** Optional richer eligibility, injected by the Viewport (which knows the
   *  full group/instance context path the tool can't see). Null = the shared
   *  default policy in faceDraw.ts. */
  private _faceEligible: FaceEligible | null = null
  setFaceEligibility(pred: FaceEligible | null): void {
    this._faceEligible = pred
  }

  /** The draw tools' plain-object policy, applied to push/pull. The one
   *  tool-local addition: inside a component editing context (the Viewport
   *  pairs `setComponentContext` with its injected predicate in production)
   *  instanced picks are the editable set — the commit routes through
   *  `push_pull_in_component`. */
  private _isEligible(object: bigint, instance: bigint | undefined): boolean {
    if (this._faceEligible !== null) return this._faceEligible(object, instance)
    if (this._activeComponent !== null) return instance !== undefined
    return defaultFaceEligible(this.wasmScene, this._activeContext, object, instance)
  }

  /**
   * The component DEFINITION being edited (double-click into an instance),
   * or null; when set, push/pull routes face operations through
   * `push_pull_in_component` instead of `push_pull`. Unchanged meaning from
   * the old `_activeComponent` field (the definition handle from
   * `instance_def`, NOT the instance itself — `_activeInstance` above is the
   * new addition).
   */
  private get _activeComponent(): bigint | null {
    return this._editContext.kind === 'instance' ? this._editContext.component : null
  }

  /**
   * True while ANY editing context is entered — object, GROUP, or component.
   * Unchanged meaning from the old `_contextScoped` field. Affects only the
   * refusal hint's wording; eligibility itself comes from the injected
   * predicate, which already understands the full context path.
   */
  private get _contextScoped(): boolean {
    return this._editContext.kind !== 'top'
  }

  /** Why an ineligible face refused, phrased as the way in. Inside ANY
   *  editing context the refusal is the scope — the clicked face may not be
   *  in any group, and double-click can't enter an out-of-scope container
   *  from here, so 'step out' is the only honest guidance. At the top level
   *  (where plain objects always pass) an instanced pick belongs to a
   *  component and anything else was a grouped face. */
  private _ineligibleFaceHint(instance: bigint | undefined): string {
    return ineligibleFaceHint(
      t`Push/pull`,
      this._contextScoped || this._activeContext !== null || this._activeComponent !== null,
      instance,
      sessionIsOpen(this.wasmScene),
    )
  }

  /**
   * Signed perpendicular depth along the push axis. When the cursor is snapped
   * to a real reference (vertex / midpoint / edge / on-face point), project THAT
   * POINT onto the axis through `anchor` so the depth is the perpendicular
   * distance to it — e.g. snapping the midpoint of an object's vertical edge
   * pushes exactly half-way. Push/pull is always a straight move along the face
   * normal, so the diagonal distance to the reference is never what we want.
   * Falls back to the cursor ray's axis projection over empty space (no snap).
   * `normal` is unit (kernel face/profile normals are).
   */
  private _axisDistance(
    snap: Snap | null,
    ray: Ray,
    anchor: [number, number, number],
    normal: [number, number, number],
  ): number {
    // Only a DELIBERATE point inference (an endpoint / midpoint / edge / axis /
    // guide / intersection the user parked the cursor on) borrows its depth.
    // A bare `on-face` snap fires almost continuously during a drag — the cursor
    // is always over *some* face — so using it would kill the free drag and make
    // the depth jump to whatever face got snapped (e.g. the far/bottom wall).
    // Free drag (and on-face) follows the cursor ray projected onto the axis.
    if (snap !== null && HARD_SNAP_KINDS.has(snap.kind)) {
      const depth =
        (snap.x - anchor[0]) * normal[0] +
        (snap.y - anchor[1]) * normal[1] +
        (snap.z - anchor[2]) * normal[2]
      // A reference ON the face's own plane (its boundary edges, their
      // midpoints — see `COPLANAR_SNAP_EPS_M`) is not a depth at all; the
      // drag keeps following the cursor instead of collapsing to zero.
      if (Math.abs(depth) > COPLANAR_SNAP_EPS_M) return depth
    }
    return projectRayOntoAxis(ray.origin, ray.direction, anchor, normal)
  }

  /** True when `_axisDistance` took its depth from a hard snap (so the
   *  depth is a deliberate reference, not a free cursor position). */
  private _depthFromHardSnap(
    snap: Snap | null,
    anchor: [number, number, number],
    normal: [number, number, number],
  ): boolean {
    if (snap === null || !HARD_SNAP_KINDS.has(snap.kind)) return false
    const depth =
      (snap.x - anchor[0]) * normal[0] +
      (snap.y - anchor[1]) * normal[1] +
      (snap.z - anchor[2]) * normal[2]
    return Math.abs(depth) > COPLANAR_SNAP_EPS_M
  }

  /** The click commit of a free depth: `_commitFromTyped` with the shown
   *  magnitude, the live drag distance standing in for the stage's. */
  private _commitFromTypedClick(
    target: PushPullTarget,
    rawDistance: number,
    shown: number,
    anchor: [number, number, number],
  ): void {
    this.stage = { kind: 'dragging', target, anchor, distance: rawDistance }
    this._commitFromTyped(shown)
  }

  /** Commit through `_commit`, then open the retype window on the result
   *  (entry count measured from the history generation, so a through
   *  push/pull that records a boolean's steps retracts them all). */
  private _commitAndArm(target: PushPullTarget, distance: number, anchor: [number, number, number]): void {
    const genBefore = this.wasmScene.history_generation()
    if (this._commit(target, distance)) {
      this.retype.armFrom({ target, distance, extrudeAsNew: this.extrudeAsNewMode, anchor }, genBefore)
    }
  }

  /**
   * Redo the push/pull just committed at |`d`| — the committed direction
   * for a positive value, the opposite for a negative one — through the
   * shared window (undo the commit, re-commit, roll back on refusal).
   */
  private _retypeDistance(d: number): void {
    const spec = this.retype.spec
    if (spec === null) return
    if (Math.abs(d) < 1e-6) {
      this.onToast(t`Move more before committing push/pull`)
      return
    }
    const signed = (spec.distance < 0 ? -1 : 1) * d
    const outcome = this.retype.apply(
      (hot) => this._commitHot(hot, signed),
      (hot) => this._commitHot(hot, hot.distance),
      (hot) => ({ ...hot, distance: signed }),
    )
    if (outcome === 'stale') this.onToast(retypeStaleMessage('push/pull'))
  }

  /** Lay the hot push/pull down at `distance`, under the Ctrl mode it was
   *  committed with. */
  private _commitHot(hot: RetypeSpec, distance: number): boolean {
    let target = hot.target
    if (target.kind === 'region') {
      // The retype's undo re-minted the sketch's edges, so the region under
      // the original click has a NEW handle: re-pick it there (a ray dropped
      // onto the plane at the anchor), exactly as the first click did.
      const fresh = this._repickRegion(hot.anchor, target.normal, target.instance)
      if (fresh === null) {
        this.onToast(t`Couldn't find the profile to redo the push/pull`)
        return false
      }
      target = { ...target, sketchHandle: fresh.sketch, regionHandle: fresh.region }
    }
    const prev = this.extrudeAsNewMode
    this.extrudeAsNewMode = hot.extrudeAsNew
    try {
      return this._commit(target, distance)
    } finally {
      this.extrudeAsNewMode = prev
    }
  }

  /** The sketch region under `anchor`, picked along −`normal` from just above
   *  the plane (instance-scoped like the first click's pick). */
  private _repickRegion(
    anchor: [number, number, number],
    normal: [number, number, number],
    instance: bigint | null,
  ): { sketch: bigint; region: bigint } | null {
    const o: [number, number, number] = [anchor[0] + normal[0], anchor[1] + normal[1], anchor[2] + normal[2]]
    const d: [number, number, number] = [-normal[0], -normal[1], -normal[2]]
    const pick = instance !== null
      ? this.wasmScene.pick_sketch_region_in_instance(instance, o[0], o[1], o[2], d[0], d[1], d[2])
      : this.wasmScene.pick_sketch_region(o[0], o[1], o[2], d[0], d[1], d[2])
    if (pick === undefined) return null
    try {
      return { sketch: pick.sketch(), region: pick.region() }
    } finally {
      pick.free()
    }
  }

  /** Whether `object` is a lone flat face (Model Health verdict `flat_sheet`,
   *  one face): a pane or a sign. A sheet of several faces (a region drawn on
   *  it, a triangulated panel) is not, since pulling one of its faces must
   *  not thicken the others; it gets the kernel's own answer. A failed
   *  diagnosis just means "not known to be one". */
  private _isFlatSheet(object: bigint): boolean {
    try {
      const rows = readHealthReport(this.wasmScene, 0, [object]).rows
      return rows.length === 1 && rows[0].verdict.kind === 'flat_sheet' && rows[0].faces === 1
    } catch {
      return false
    }
  }

  /** True when the kernel accepted the commit (false = refused, toasted). */
  private _commit(target: PushPullTarget, distance: number): boolean {
    if (Math.abs(distance) < 1e-6) {
      this.onToast(t`Move more before committing push/pull`)
      return false
    }

    try {
      if (target.kind === 'region') {
        // A def-owned region (component-edit-parity.md phase A2) births the
        // solid as a new DEFINITION member via `extrude_region_in_instance`
        // (seen by every instance at once), instead of a world Object.
        const objectId = target.instance !== null
          ? this.wasmScene.extrude_region_in_instance(
              target.instance,
              target.sketchHandle,
              target.regionHandle,
              distance,
            )
          : this.wasmScene.extrude_region(
              target.sketchHandle,
              target.regionHandle,
              distance,
            )
        this.onCommit(objectId)
      } else if (this.extrudeAsNewMode && this._activeComponent === null) {
        // Ctrl/Cmd modifier (design tool-parity §2): extrude a NEW
        // coincident object from the clicked face instead of pushing/
        // pulling the source — SketchUp's "leave original face". Not
        // offered inside a component editing context: the kernel op only
        // reaches WORLD faces (a definition member has no face to extrude
        // at world scale), so that combination silently falls back to the
        // ordinary `push_pull_in_component` commit below.
        const objectId = this.wasmScene.extrude_face_as_new_object(
          target.objectHandle,
          target.faceHandle,
          distance,
        )
        this.onCommit(objectId)
      } else if (this._isFlatSheet(target.objectHandle)) {
        // A lone flat surface (Model Health verdict `flat_sheet`) has no
        // solid to push or pull: pulling it gives a solid (design F7,
        // SketchUp's "pull a lone face, get a box"), out of the face's front
        // by the dragged distance. A bent or otherwise open object keeps the
        // kernel's ordinary refusal below.
        if (target.instance !== null) {
          this.wasmScene.thicken_sheet_in_instance(target.instance, target.objectHandle, distance)
        } else {
          this.wasmScene.thicken_sheet(target.objectHandle, 0, distance)
        }
        this.onCommit(target.objectHandle)
      } else {
        // Route through push_pull_in_component when the face was picked
        // through a component instance — `target.instance` (the instance the
        // ghost preview's pose came from), not necessarily `this._activeInstance`
        // (an injected eligibility predicate could in principle allow a
        // different one; same rationale as the `normal` lookup above).
        // `distance` is the WORLD drag distance the ghost swept; the kernel
        // maps it through `target.instance`'s pose (delta-review fix —
        // previously committed raw, diverging from the ghost on a scaled
        // instance).
        const report = target.instance !== null
          ? this.wasmScene.push_pull_in_component(
              target.instance,
              target.objectHandle,
              target.faceHandle,
              distance,
            )
          : this.wasmScene.push_pull(
              target.objectHandle,
              target.faceHandle,
              distance,
            )
        try {
          // A through-cut consumes the source object and replaces it with
          // one or more new objects; commit the first of those so selection/
          // highlight lands on real geometry instead of the now-gone source.
          if (report.is_through()) {
            const results = report.result_objects()
            this.onCommit(results.length > 0 ? results[0] : target.objectHandle)
          } else {
            this.onCommit(target.objectHandle)
          }
        } finally {
          report.free()
        }
      }
      // Record on every successful commit, however it was invoked (drag,
      // typed Enter, or a double-click repeat) — a double-click always
      // repeats the MOST RECENT distance, chainable like SketchUp's.
      this.lastCommittedDistance = distance
      return true
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      const message = kernelErrorMessage(code ?? 'Unknown', rawMsg)
      this.onToast(message, code ?? undefined)
      return false
    }
  }

  /**
   * Report the live distance measurement while dragging.
   * When the user has typed something, that buffer (tagged with the display
   * unit) is the readout; otherwise show the signed live distance so a
   * recess (pushed inward) reads negative.
   */
  private _reportMeasurement(distance: number): void {
    if (this.typed !== '') {
      this.onMeasurementCb(this._typedReadout())
      return
    }
    this.onMeasurementCb(formatLength(distance))
  }

  /** The ghost is blue ("a new object appears, nothing existing changes")
   *  in extrude-as-new mode and for every region target — a free sketch
   *  region always extrudes into a new object, whichever way it is pushed.
   *  Inside a component edit the Ctrl toggle's commit falls back to an
   *  ordinary push (see `_commit`), so the ghost stays on the push colors. */
  private _ghostMakesNewObject(): boolean {
    return (this.extrudeAsNewMode && this._activeComponent === null) || (this.stage.kind === 'dragging' && this.stage.target.kind === 'region')
  }

  private _drawGhostPreview(
    anchor: [number, number, number],
    normal: [number, number, number],
    distance: number,
  ): void {
    clearPreview(this.preview)

    if (Math.abs(distance) < 1e-6) return

    // Prefer the live swept-solid ghost (the actual prism push/pull will
    // produce) — plain object/region AND in-instance targets alike
    // (component-edit-parity.md Finding 3: the in-instance path used to
    // fall straight to the bare arrow, the only visible difference from a
    // plain-object push/pull). `face_boundary`/`region_boundary` answer in
    // DEFINITION-local coordinates for a component member/def-owned sketch,
    // so `target.instance` (non-null there) maps the boundary through that
    // instance's pose into world space before building the ghost — the same
    // frame `_commit` already puts the actual solid in.
    if (this.stage.kind === 'dragging') {
      const { target } = this.stage
      try {
        const boundary = target.kind === 'region'
          ? this.wasmScene.region_boundary(target.sketchHandle, target.regionHandle)
          : this.wasmScene.face_boundary(target.objectHandle, target.faceHandle)
        const worldBoundary =
          target.instance !== null ? this._poseMapBoundary(boundary, target.instance) : boundary
        // A stale/unresolvable instance pose (rare: the instance vanished
        // mid-drag) degrades to the arrow rather than drawing a misplaced
        // ghost — `worldBoundary` is null exactly then.
        if (worldBoundary !== null) {
          // A new object (extrude-as-new mode, or any free-sketch region,
          // which always extrudes into a new object) tints the ghost blue
          // on both signs; a face push/pull keeps its orange add / red cut
          // look.
          const prism = buildSweptPrismPreview(
            worldBoundary,
            normal,
            distance,
            this._ghostMakesNewObject()
              ? readPreviewTint(PREVIEW_EXTRUDE_NEW_TOKEN, PREVIEW_EXTRUDE_NEW_FALLBACK)
              : distance > 0
                ? readPreviewTint(PREVIEW_PUSHPULL_TOKEN, PREVIEW_PUSHPULL_FALLBACK)
                : undefined,
          )
          if (prism !== null) {
            this.preview.add(prism)
            return
          }
        }
      } catch {
        // Stale handle mid-drag (e.g. the target object/sketch changed
        // underneath us) — fall through to the arrow, no toast.
      }
    }

    this._drawArrowFallback(anchor, normal, distance)
  }

  /**
   * Map a flat `[x0,y0,z0, x1,y1,z1, …]` boundary from DEFINITION-local
   * coordinates into WORLD space through `instance`'s current pose
   * (row-major 3×4 affine, the same convention `SceneRenderer`/the kernel
   * use everywhere else). Returns `null` for a stale/unknown instance so the
   * caller can fall back to the arrow instead of drawing a misplaced ghost.
   */
  private _poseMapBoundary(boundary: Float32Array | number[], instance: bigint): Float32Array | null {
    const pose = this.wasmScene.instance_pose(instance)
    if (pose === undefined) return null
    const n = Math.floor(boundary.length / 3)
    const out = new Float32Array(n * 3)
    for (let i = 0; i < n; i++) {
      const x = boundary[i * 3]
      const y = boundary[i * 3 + 1]
      const z = boundary[i * 3 + 2]
      out[i * 3] = pose[0] * x + pose[1] * y + pose[2] * z + pose[3]
      out[i * 3 + 1] = pose[4] * x + pose[5] * y + pose[6] * z + pose[7]
      out[i * 3 + 2] = pose[8] * x + pose[9] * y + pose[10] * z + pose[11]
    }
    return out
  }

  /**
   * Bare arrow + tip-cross preview — the original push/pull ghost. Used as a
   * fallback when the swept-prism ghost can't be built: inside a component
   * editing context (face_boundary is definition-local there, not world), or
   * when the boundary fetch fails (stale handle) or the prism is degenerate.
   */
  private _drawArrowFallback(
    anchor: [number, number, number],
    normal: [number, number, number],
    distance: number,
  ): void {
    const [ax, ay, az] = anchor
    const [nx, ny, nz] = normal

    const tip: [number, number, number] = [
      ax + nx * distance,
      ay + ny * distance,
      az + nz * distance,
    ]

    const pts = new Float32Array([ax, ay, az, ...tip])
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(pts, 3))
    const arrowColor = this._ghostMakesNewObject()
      ? readPreviewTint(PREVIEW_EXTRUDE_NEW_TOKEN, PREVIEW_EXTRUDE_NEW_FALLBACK)
      : readPreviewTint(PREVIEW_PUSHPULL_TOKEN, PREVIEW_PUSHPULL_FALLBACK)
    const mat = new THREE.LineBasicMaterial({
      color: arrowColor,
      depthTest: false,
    })
    const line = new THREE.LineSegments(geo, mat)
    line.renderOrder = 996
    this.preview.add(line)

    // Also render a small cross at the tip
    const s = 0.05
    const crossPts = new Float32Array([
      tip[0] - s, tip[1], tip[2],  tip[0] + s, tip[1], tip[2],
      tip[0], tip[1] - s, tip[2],  tip[0], tip[1] + s, tip[2],
    ])
    const crossGeo = new THREE.BufferGeometry()
    crossGeo.setAttribute('position', new THREE.BufferAttribute(crossPts, 3))
    const crossMat = new THREE.LineBasicMaterial({ color: arrowColor, depthTest: false })
    const cross = new THREE.LineSegments(crossGeo, crossMat)
    cross.renderOrder = 996
    this.preview.add(cross)
  }
}
