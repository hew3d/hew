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

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import { intersectGroundPlane } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { projectRayOntoAxis } from '../viewport/geoHelpers'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { editLengthBuffer, isLengthInputKey } from './moveInput'
import { formatLength, parseLengthToMeters, getLengthUnit, typedReadout } from '../settings/units'
import { buildSweptPrismPreview, clearPreview } from './transformPreview'
import { defaultFaceEligible, type FaceEligible } from './faceDraw'

/** Snap kinds whose point is a deliberate depth reference for push/pull — the
 * cursor was parked on a real feature. `on-face` is excluded on purpose: it
 * fires continuously during a drag and would hijack the free-drag depth. */
const HARD_SNAP_KINDS = new Set([
  'endpoint',
  'center',
  'quadrant',
  'tangent',
  'midpoint',
  'intersection',
  'on-edge',
  'on-guide',
  'on-axis',
])

export type PushPullTarget =
  | { kind: 'region'; sketchHandle: bigint; regionHandle: bigint; normal: [number, number, number] }
  | { kind: 'face'; objectHandle: bigint; faceHandle: bigint; normal: [number, number, number] }

export type OnPushPullCommit = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'dragging'
      target: PushPullTarget
      anchor: [number, number, number]
      /** Last computed signed distance */
      distance: number
    }

export class PushPullTool implements Tool {
  readonly name = 'Push/Pull'

  /** Live status-bar guidance for the current stage (see Tool.statusHint). */
  statusHint(): string {
    return this.stage.kind === 'idle'
      ? 'Click a face to push or pull it.'
      : 'Move to extrude, click to commit — or type an exact distance.'
  }

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onCommit: OnPushPullCommit
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** VCB buffer — raw string being typed by the user */
  private typed: string = ''

  /** The snap last seen on hover (for highlight logic) */
  lastSnap: Snap | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onCommit: OnPushPullCommit,
    onToast: OnToast,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onCommit = onCommit
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
  }

  // ── Optional Tool interface extensions ─────────────────────────────────────

  capturingInput(): boolean {
    return this.stage.kind === 'dragging'
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    this.lastSnap = snap

    if (this.stage.kind === 'dragging') {
      const { target, anchor } = this.stage
      const distance = this._axisDistance(snap, ray, anchor, target.normal)
      this.stage = { ...this.stage, distance }
      this._drawGhostPreview(anchor, target.normal, distance)
      this._reportMeasurement(distance)
    }
    // Hover highlight is handled via CueLayer for on-face snaps (M1 shortcut
    // per docs/DEVELOPMENT.md: show snap marker at face location; per-face highlight
    // requires face→triangle table in MeshJs which is a WASM_API addendum).
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    if (this.stage.kind === 'idle') {
      let target: PushPullTarget | null = null
      let anchor: [number, number, number] = [0, 0, 0]

      // --- Path A: ray-cast for the nearest object face (ignores snap priority) ---
      // pick_face bypasses the drawing snap bias toward vertices/edges, so it
      // reliably returns the surface under the cursor even when snap prefers a
      // nearby endpoint.  We call this FIRST; Path B only fires when no object
      // face is hit (bare ground or no objects yet).
      const pick = this.wasmScene.pick_face(
        ray.origin[0], ray.origin[1], ray.origin[2],
        ray.direction[0], ray.direction[1], ray.direction[2],
      )
      if (pick !== undefined) {
        try {
          const objectHandle = pick.object()
          const instanceHandle = pick.instance()
          // Same face-eligibility policy as the draw tools (faceDraw.ts): at
          // the top level only PLAIN objects are directly push/pullable —
          // faces inside a group or component instance keep their explicit
          // editing step. Inside an editing context only that context's
          // scope is editable, so isolated editing can't disturb neighbors.
          if (this._isEligible(objectHandle, instanceHandle)) {
            const faceHandle = pick.face()
            const normalArr = this.wasmScene.face_normal(objectHandle, faceHandle)
            const normal: [number, number, number] = [normalArr[0], normalArr[1], normalArr[2]]
            // Prefer the snap position as anchor (snapped to a real point on the
            // surface); fall back to ground hit, then ray origin.
            if (snap !== null) {
              anchor = [snap.x, snap.y, snap.z]
            } else {
              const hit = intersectGroundPlane(ray)
              anchor = hit !== null ? [hit.x, hit.y, hit.z] : [...ray.origin]
            }
            target = { kind: 'face', objectHandle, faceHandle, normal }
          } else {
            // FAIL CLOSED: an ineligible face CONSUMES the click. Falling
            // through to Path B would let a sketch region along the same ray
            // (a ground sketch behind the group — ordinary mid-modeling
            // state) silently start a drag and extrude geometry the user
            // did not aim at. Refuse explicably instead.
            this.onToast(this._ineligibleFaceHint(instanceHandle))
            return
          }
        } finally {
          pick.free()
        }
      }

      // --- Path B: no object face hit — try picking a sketch region ---
      // Only reached when pick_face returns undefined (bare ground click, or
      // no objects in scene yet). `pick_sketch_region` resolves the smallest
      // containing region across ALL live sketches kernel-side (nested rings
      // resolve to the innermost — the app no longer has to walk sketch_regions
      // + region_boundary + point-in-polygon itself).
      // Region extrusion is a top-level act; suppress it inside a context.
      if (target === null && this._activeContext === null) {
        const regionPick = this.wasmScene.pick_sketch_region(
          ray.origin[0], ray.origin[1], ray.origin[2],
          ray.direction[0], ray.direction[1], ray.direction[2],
        )
        if (regionPick !== undefined) {
          try {
            const sketchHandle = regionPick.sketch()
            const regionHandle = regionPick.region()
            if (snap !== null) {
              anchor = [snap.x, snap.y, snap.z]
            } else {
              const hit = intersectGroundPlane(ray)
              anchor = hit !== null ? [hit.x, hit.y, hit.z] : [...ray.origin]
            }
            target = {
              kind: 'region',
              sketchHandle,
              regionHandle,
              normal: [0, 0, 1], // ground plane normal — all sketches are ground-plane today
            }
          } finally {
            regionPick.free()
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
      this._commit(target, finalDistance === 0 ? distance : finalDistance)
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
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

  cancel(): void {
    this.stage = { kind: 'idle' }
    this.typed = ''
    clearPreview(this.preview)
    this.onMeasurementCb('')
    this.lastSnap = null
  }

  /**
   * Commit from the typed VCB buffer. `dist` is a signed distance in meters,
   * already converted from the display unit. The sign matches the current
   * drag direction (the last live-projected `distance` on the active stage),
   * so typing a positive number continues pushing/pulling the way the cursor
   * was already dragging; typing while dragging the opposite way flips it.
   */
  private _commitFromTyped(dist: number): void {
    if (this.stage.kind !== 'dragging') return
    const { target, distance } = this.stage

    const sign = distance < 0 ? -1 : 1
    const signed = Math.abs(dist) * sign

    this.stage = { kind: 'idle' }
    this.typed = ''
    clearPreview(this.preview)
    this.onMeasurementCb('')

    this._commit(target, signed)
  }

  /** Set the active editing context (entered object), or null for top level.
   *  When set, push/pull only acts on that object's faces ( scoped editing). */
  private _activeContext: bigint | null = null
  setActiveContext(objectId: bigint | null): void {
    this._activeContext = objectId
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
   * Set the active component context: when the user has double-clicked into an
   * instance, push/pull routes face operations through `push_pull_in_component`
   * instead of `push_pull`. `componentId` is the definition handle (from
   * `instance_def`), or null for normal (non-instance) context.
   */
  private _activeComponent: bigint | null = null
  setComponentContext(componentId: bigint | null): void {
    this._activeComponent = componentId
  }

  /**
   * True while ANY editing context is entered — object, GROUP, or component.
   * The two id channels above only carry object/instance contexts (a group
   * context leaves both null), so the Viewport sets this alongside them.
   * Affects only the refusal hint's wording; eligibility itself comes from
   * the injected predicate, which already understands the full context path.
   */
  private _contextScoped = false
  setContextScoped(scoped: boolean): void {
    this._contextScoped = scoped
  }

  /** Why an ineligible face refused, phrased as the way in. Inside ANY
   *  editing context the refusal is the scope — the clicked face may not be
   *  in any group, and double-click can't enter an out-of-scope container
   *  from here, so 'step out' is the only honest guidance. At the top level
   *  (where plain objects always pass) an instanced pick belongs to a
   *  component and anything else was a grouped face. */
  private _ineligibleFaceHint(instance: bigint | undefined): string {
    if (this._contextScoped || this._activeContext !== null || this._activeComponent !== null) {
      return 'Push/pull is scoped to what you are editing — press Esc to step out first'
    }
    if (instance !== undefined) {
      return 'That face is part of a component — double-click it to edit the component'
    }
    return 'That face is inside a group — double-click to enter the group and edit it'
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
      return (
        (snap.x - anchor[0]) * normal[0] +
        (snap.y - anchor[1]) * normal[1] +
        (snap.z - anchor[2]) * normal[2]
      )
    }
    return projectRayOntoAxis(ray.origin, ray.direction, anchor, normal)
  }

  private _commit(target: PushPullTarget, distance: number): void {
    if (Math.abs(distance) < 1e-6) {
      this.onToast('Move more before committing push/pull')
      return
    }

    try {
      if (target.kind === 'region') {
        const objectId = this.wasmScene.extrude_region(
          target.sketchHandle,
          target.regionHandle,
          distance,
        )
        this.onCommit(objectId)
      } else {
        // Route through push_pull_in_component when inside a component editing context.
        const report = this._activeComponent !== null
          ? this.wasmScene.push_pull_in_component(
              this._activeComponent,
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
          if (this._activeComponent === null && report.is_through()) {
            const results = report.result_objects()
            this.onCommit(results.length > 0 ? results[0] : target.objectHandle)
          } else {
            this.onCommit(target.objectHandle)
          }
        } finally {
          report.free()
        }
      }
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      const message = kernelErrorMessage(code ?? 'Unknown', rawMsg)
      this.onToast(message, code ?? undefined)
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

  private _drawGhostPreview(
    anchor: [number, number, number],
    normal: [number, number, number],
    distance: number,
  ): void {
    clearPreview(this.preview)

    if (Math.abs(distance) < 1e-6) return

    // Prefer the live swept-solid ghost (the actual prism push/pull will
    // produce). It needs a real face/region boundary in world space:
    //  - `face_boundary` returns DEFINITION-local coords inside a component
    //    editing context, which would not match the placed instance's world
    //    pose — fall back to the arrow there rather than draw a misplaced
    //    ghost.
    //  - A stale handle mid-drag (e.g. the target object/sketch changed
    //    underneath us) throws from wasm; fall back silently, no toast.
    if (this.stage.kind === 'dragging') {
      const { target } = this.stage
      const insideComponent = target.kind === 'face' && this._activeComponent !== null
      if (!insideComponent) {
        try {
          const boundary = target.kind === 'region'
            ? this.wasmScene.region_boundary(target.sketchHandle, target.regionHandle)
            : this.wasmScene.face_boundary(target.objectHandle, target.faceHandle)
          const prism = buildSweptPrismPreview(boundary, normal, distance)
          if (prism !== null) {
            this.preview.add(prism)
            return
          }
        } catch {
          // Stale handle mid-drag — fall through to the arrow.
        }
      }
    }

    this._drawArrowFallback(anchor, normal, distance)
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
    const mat = new THREE.LineBasicMaterial({
      color: 0xee8800,
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
    const crossMat = new THREE.LineBasicMaterial({ color: 0xee8800, depthTest: false })
    const cross = new THREE.LineSegments(crossGeo, crossMat)
    cross.renderOrder = 996
    this.preview.add(cross)
  }
}
