/**
 * SliceTool — Tools ▸ Slice: defines a cutting plane and commits a real
 * planar cut that splits the targeted solid into two independent watertight
 * Objects ( — the Fusion *Split Body* / Onshape *Split Part* / Blender
 * *Bisect* model). Re-joining afterward is an explicit Union — no special UI
 * here.
 *
 * Plane model (deliberately simpler than Protractor's, since Slice has no
 * apex/baseline/sweep gesture — just a single plane that follows the
 * cursor):
 *   - Idle: every pointer move recomputes the plane:
 *       - If the cursor is hovering a live world-Object face
 *         (`elementKind === 'face'`), the plane lays flat on that face
 *         (`scene.face_plane`) — the SketchUp/Fusion convention of offering
 *         "cut along this face" as the path of least resistance.
 *       - Otherwise, the normal is the **locked axis** (`lockedAxis`,
 *         default **Z** — the most common "horizontal slice" case,
 *         mirroring SketchUp's ground = blue plane default). Arrow keys
 *         cycle it (Right→X, Left→Y, Up/Down→Z, matching ProtractorTool's
 *         best-effort arrow↔axis mapping). This is the one way Slice
 *         differs from "always start unlocked": the contract calls for an
 *         axis-aligned-by-default plane, so there's no unlocked/neutral
 *         state — just whichever axis is currently selected.
 *   - The plane point follows the cursor's snapped position (so the preview
 *     quad visibly tracks the mouse); an optional typed VCB offset nudges
 *     that point along the normal (reusing `parseLengthToMeters`/
 *     `editLengthBuffer`, format-aware via `getLengthUnit` — same pattern as
 *     TapeMeasureTool/ProtractorTool).
 *   - Click (or Enter with a typed offset) commits: the **target solid** is
 *     whichever live world Object `pick_face` resolves under the same
 *     pointer event (tracked continuously on every move/down — world
 *     Objects only; a pick landing on instanced geometry is ignored,
 *     mirroring PushPullTool/TapeMeasureTool's instanced-geometry
 *     deferral). `scene.slice_object(target, plane)` is called; on success
 *     the **positive** piece (the first of the two returned handles) is
 *     selected via `onSliceCommitted`, mirroring how PushPullTool/runBoolean
 *     select their result so the scene reconciles/redraws/selects
 *     consistently with every other tool. On a thrown kernel error
 *     (`NotSolid`, `PlaneMissesSolid`, `Degenerate`, or a stale/unknown
 *     handle), a toast is shown via `kernelErrorMessage`/
 *     `parseKernelErrorCode`, mirroring `PushPullTool._commit`.
 *   - Esc cancels (no multi-stage gesture to unwind — just clears the
 *     preview and typed buffer).
 *
 * Explicitly OUT of scope for this slice (deferrals):
 *   - Free (non-axis-aligned, non-face) plane orientation: / only
 *     calls for axis-aligned-or-on-a-face, so there's no "drag to rotate the
 *     plane" gesture here.
 *   - Slicing instanced geometry directly (would need an
 *     instance-pose-composed plane + a kernel surface to slice *through* a
 *     component definition without affecting sibling instances) — matches
 *     the existing instanced-geometry deferrals elsewhere.
 *   - Multi-object slice (cutting every solid the plane passes through in
 *     one commit): the kernel surface `slice_object` takes a single object
 *     handle, so this tool targets the one solid under the cursor, the same
 *     way Push/Pull targets the one face under the cursor.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { editLengthBuffer } from './moveInput'
import { parseLengthToMeters, getLengthUnit, getLengthUnitSuffix } from '../settings/units'
import { parseKernelErrorCode, kernelErrorMessage } from '../viewport/geoHelpers'
import { axisColorForDirection, axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'

export type OnSliceCommitted = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/** Neutral (off-axis) preview color — matches Protractor/TapeMeasure's guide preview. */
const NEUTRAL_PREVIEW_COLOR = 0x9933cc
/** Local (pre-scale) half-extent of the preview quad; the group is uniformly
 * scaled each frame (updateDiskScale) so its on-screen size stays constant. */
const PLANE_BASE_HALF = 1
/** Screen-constant scale: world half-extent = PLANE_SCREEN_K · cameraDistance
 * (mirrors ProtractorTool's DISK_SCREEN_K so the gizmo reads at a steady size). */
const PLANE_SCREEN_K = 0.06
/** Opacity of the translucent preview plane fill. */
const PLANE_FILL_OPACITY = 0.25
/** Axis-snap tolerance for coloring a face-laid plane: ~2°, as a cosine threshold. */
const AXIS_COLOR_TOL_DOT = Math.cos((2 * Math.PI) / 180)

type Axis = 0 | 1 | 2
const AXIS_NORMAL: Record<Axis, [number, number, number]> = {
  0: [1, 0, 0],
  1: [0, 1, 0],
  2: [0, 0, 1],
}

function normalize(v: [number, number, number]): [number, number, number] | null {
  const len = Math.sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
  if (len < 1e-9) return null
  return [v[0] / len, v[1] / len, v[2] / len]
}

/**
 * Build two unit vectors (u, v) spanning the plane ⊥ `normal` — identical
 * construction to ProtractorTool's `planeBasis` (kept local; promoting it to
 * a shared helper is a nice-to-have, not required for this slice).
 */
function planeBasis(
  normal: [number, number, number],
): { u: [number, number, number]; v: [number, number, number] } {
  const [nx, ny, nz] = normal
  const ax = Math.abs(nx), ay = Math.abs(ny), az = Math.abs(nz)
  let seed: [number, number, number]
  if (ax <= ay && ax <= az) seed = [1, 0, 0]
  else if (ay <= ax && ay <= az) seed = [0, 1, 0]
  else seed = [0, 0, 1]

  const dot = seed[0] * nx + seed[1] * ny + seed[2] * nz
  const raw: [number, number, number] = [
    seed[0] - dot * nx,
    seed[1] - dot * ny,
    seed[2] - dot * nz,
  ]
  const u = normalize(raw) ?? [1, 0, 0]
  const v: [number, number, number] = [
    ny * u[2] - nz * u[1],
    nz * u[0] - nx * u[2],
    nx * u[1] - ny * u[0],
  ]
  return { u, v }
}

export class SliceTool implements Tool {
  readonly name = 'Slice'

  private preview: THREE.Group
  private wasmScene: WasmScene
  private onSliceCommitted: OnSliceCommitted
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** VCB buffer — raw string being typed by the user (offset along the normal). */
  private typed: string = ''

  /** THREE.js mesh+outline group for the preview plane. */
  private previewPlane: THREE.Group | null = null

  /** World axis arrow keys cycle to (default Z). Used to seed a lock. */
  private lockedAxis: Axis = 2

  /** The pinned plane normal, or null to follow the hovered face / locked axis.
   * Set by **Shift** (locks the CURRENT plane — face or axis — the SketchUp
   * inference-lock convention, so an arbitrary face plane can be pinned) or by
   * an **arrow key** (locks the chosen axis). Persists across commits; cleared
   * by Esc. While set, the hovered face is ignored, so a cut plane stays put. */
  private lockedNormal: [number, number, number] | null = null

  /** Last-known plane point, following the cursor (pre-typed-offset). */
  private lastPoint: [number, number, number] | null = null
  /** Last-known plane normal: the locked axis, or a hovered face's normal. */
  private lastNormal: [number, number, number] = AXIS_NORMAL[2]
  /** World Object under the cursor (via pick_face), or null if none/instanced. Refreshed on every move/down. */
  private lastPickObject: bigint | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onSliceCommitted: OnSliceCommitted,
    onToast: OnToast,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onSliceCommitted = onSliceCommitted
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
  }

  // ── Optional Tool interface extensions ─────────────────────────────────

  capturingInput(): boolean {
    // Slice has no multi-stage gesture, but the VCB offset is always live so
    // typed digits never leak through to tool-switch shortcuts while this
    // tool is active (mirrors Protractor/TapeMeasure capturing during their
    // hover/preview phase).
    return true
  }

  // ── Tool interface ──────────────────────────────────────────────────────

  onPointerMove(snap: Snap | null, ray: Ray): void {
    this.lastPickObject = this._pickWorldObject(ray)
    if (snap === null) return

    this.lastPoint = [snap.x, snap.y, snap.z]
    this.lastNormal = this._resolveNormal(snap)
    this._refreshPreview()
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    this.lastPickObject = this._pickWorldObject(ray)
    if (snap === null) return

    this.lastPoint = [snap.x, snap.y, snap.z]
    this.lastNormal = this._resolveNormal(snap)
    this._refreshPreview()
    this._commitAtCursor()
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      // First Esc lifts the plane lock (back to follow-the-hovered-face mode);
      // a second Esc (unlocked) cancels the tool.
      if (this.lockedNormal !== null) {
        this.lockedNormal = null
        this._refreshPreview()
      } else {
        this.cancel()
      }
      return
    }

    // Shift toggles a plane lock on the CURRENT plane (face or axis) — the
    // SketchUp inference-lock convention, so an arbitrary (e.g. sloped face)
    // plane can be pinned. Guard autorepeat so a held Shift doesn't flicker.
    if (ev.key === 'Shift') {
      if (!ev.repeat) {
        this.lockedNormal = this.lockedNormal === null ? this.lastNormal : null
        this._refreshPreview()
      }
      return
    }

    if (
      ev.key === 'ArrowUp' || ev.key === 'ArrowDown'
      || ev.key === 'ArrowRight' || ev.key === 'ArrowLeft'
    ) {
      ev.preventDefault()
      if (ev.key === 'ArrowUp' || ev.key === 'ArrowDown') {
        this.lockedAxis = 2
      } else if (ev.key === 'ArrowRight') {
        this.lockedAxis = 0
      } else {
        this.lockedAxis = 1
      }
      // Pin to this axis and ignore the hovered face until Esc — otherwise the
      // plane flips to every face the cursor crosses, making an axis cut nearly
      // impossible to place.
      this.lockedNormal = AXIS_NORMAL[this.lockedAxis]
      this.lastNormal = AXIS_NORMAL[this.lockedAxis]
      this._refreshPreview()
      return
    }

    if (ev.key === 'Enter') {
      const meters = parseLengthToMeters(this.typed)
      if (meters !== null) {
        this._commitFromTyped(meters)
      }
      return
    }

    if (
      (ev.key >= '0' && ev.key <= '9') ||
      ev.key === '.' ||
      ev.key === '-' ||
      ev.key === 'Backspace' ||
      ev.key === "'" ||
      ev.key === '"' ||
      ev.key === '/' ||
      ev.key === ' '
    ) {
      this.typed = editLengthBuffer(this.typed, ev.key, getLengthUnit())
      this.onMeasurementCb(this._typedReadout())
      this._refreshPreview()
    }
  }

  /** The typed-buffer readout, suffixed for metric formats (imperial tokens
   * like `'`/`"` are already visible in the buffer itself). */
  private _typedReadout(): string {
    const suffix = getLengthUnitSuffix()
    return suffix === '' ? this.typed : `${this.typed} ${suffix}`
  }

  cancel(): void {
    this.typed = ''
    this._clearPreview()
    this.lastPoint = null
    this.lockedNormal = null
    this.onMeasurementCb('')
  }

  /** Lighter reset after a successful slice: clears the typed buffer + preview
   * but KEEPS the plane lock, so consecutive cuts on the same locked plane don't
   * need the lock re-set (matches the Protractor's persist-across-commits). */
  private _resetAfterCommit(): void {
    this.typed = ''
    this._clearPreview()
    this.lastPoint = null
    this.onMeasurementCb('')
  }

  // ── Private helpers ─────────────────────────────────────────────────────

  /**
   * Resolve the live plane normal for a hover/click snap: the hovered live
   * world-Object face's plane normal if resolvable, else the locked axis.
   */
  private _resolveNormal(snap: Snap): [number, number, number] {
    // A held plane lock (Shift or arrow key) wins over the hovered face.
    if (this.lockedNormal !== null) return this.lockedNormal
    if (snap.elementKind === 'face' && snap.object !== undefined && snap.element !== undefined) {
      try {
        const plane = this.wasmScene.face_plane(snap.object, snap.element)
        const normal = normalize([plane[3], plane[4], plane[5]])
        if (normal !== null) return normal
      } catch {
        // Not a live world-Object face (e.g. instanced geometry) — fall through.
      }
    }
    return AXIS_NORMAL[this.lockedAxis]
  }

  /** Resolve the world Object under the cursor via pick_face; null if none, or the pick is instanced geometry. */
  private _pickWorldObject(ray: Ray): bigint | null {
    const pick = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )
    if (pick === undefined) return null
    try {
      if (pick.instance() !== undefined) return null // instanced geometry — out of scope
      return pick.object()
    } finally {
      pick.free()
    }
  }

  /** Effective plane point: lastPoint nudged along lastNormal by the typed offset, if any. */
  private _effectivePoint(): [number, number, number] | null {
    if (this.lastPoint === null) return null
    const offset = parseLengthToMeters(this.typed)
    if (offset === null) return this.lastPoint
    return [
      this.lastPoint[0] + this.lastNormal[0] * offset,
      this.lastPoint[1] + this.lastNormal[1] * offset,
      this.lastPoint[2] + this.lastNormal[2] * offset,
    ]
  }

  private _commitAtCursor(): void {
    const point = this._effectivePoint()
    if (point === null) return
    this._commit(point, this.lastNormal)
  }

  private _commitFromTyped(offsetMeters: number): void {
    if (this.lastPoint === null) return
    const point: [number, number, number] = [
      this.lastPoint[0] + this.lastNormal[0] * offsetMeters,
      this.lastPoint[1] + this.lastNormal[1] * offsetMeters,
      this.lastPoint[2] + this.lastNormal[2] * offsetMeters,
    ]
    this._commit(point, this.lastNormal)
  }

  /** Commit the slice at the given plane (point + normal) against `lastPickObject`. */
  private _commit(point: [number, number, number], normal: [number, number, number]): void {
    const target = this.lastPickObject
    if (target === null) {
      this.onToast("Hover a solid's face before slicing")
      return
    }

    const plane = new Float64Array([point[0], point[1], point[2], normal[0], normal[1], normal[2]])
    try {
      const results = this.wasmScene.slice_object(target, plane)
      if (results.length > 0) {
        this.onSliceCommitted(results[0])
      }
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      this.onToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
    }
    // Keep any plane lock so the next cut on the same plane needs no re-lock.
    this._resetAfterCommit()
  }

  /**
   * Rebuild the preview plane (quad + outline) at the current effective
   * point/normal, colored by the normal's axis (or neutral purple off-axis —
   * e.g. a non-axis face lay-down).
   */
  private _refreshPreview(): void {
    const point = this._effectivePoint()
    if (point === null) {
      this._clearPreview()
      return
    }
    this._buildPreviewPlane(point, this.lastNormal)
    this._reportOffsetOrTyped()
  }

  private _reportOffsetOrTyped(): void {
    if (this.typed !== '') {
      this.onMeasurementCb(this._typedReadout())
      return
    }
    this.onMeasurementCb('')
  }

  private _buildPreviewPlane(
    center: [number, number, number],
    normal: [number, number, number],
  ): void {
    this._clearPreview()

    const unitNormal = normalize(normal) ?? AXIS_NORMAL[2]
    const { u, v } = planeBasis(unitNormal)
    const match = axisColorForDirection(unitNormal, AXIS_COLOR_TOL_DOT, axisColorsForTheme(getResolvedTheme()))
    const color = match !== null ? match.color : NEUTRAL_PREVIEW_COLOR

    // Quad corners in LOCAL space (offsets from the group origin along u/v); the
    // group is positioned at `center` and uniformly scaled per frame, so the
    // gizmo stays a constant on-screen size (see updateDiskScale).
    const corners: [number, number][] = [
      [-PLANE_BASE_HALF, -PLANE_BASE_HALF],
      [PLANE_BASE_HALF, -PLANE_BASE_HALF],
      [PLANE_BASE_HALF, PLANE_BASE_HALF],
      [-PLANE_BASE_HALF, PLANE_BASE_HALF],
    ]
    const toLocal = ([cu, cv]: [number, number]): [number, number, number] => [
      u[0] * cu + v[0] * cv,
      u[1] * cu + v[1] * cv,
      u[2] * cu + v[2] * cv,
    ]
    const worldCorners = corners.map(toLocal)

    // Fill: two triangles.
    const tri1 = [worldCorners[0], worldCorners[1], worldCorners[2]]
    const tri2 = [worldCorners[0], worldCorners[2], worldCorners[3]]
    const fillPts = new Float32Array(18)
    let i = 0
    for (const p of [...tri1, ...tri2]) {
      fillPts[i++] = p[0]
      fillPts[i++] = p[1]
      fillPts[i++] = p[2]
    }
    const fillGeo = new THREE.BufferGeometry()
    fillGeo.setAttribute('position', new THREE.BufferAttribute(fillPts, 3))
    const fillMat = new THREE.MeshBasicMaterial({
      color,
      transparent: true,
      opacity: PLANE_FILL_OPACITY,
      side: THREE.DoubleSide,
      depthTest: false,
    })
    const fillMesh = new THREE.Mesh(fillGeo, fillMat)

    // Outline: closed loop.
    const outlinePts = new Float32Array(12)
    i = 0
    for (const p of worldCorners) {
      outlinePts[i++] = p[0]
      outlinePts[i++] = p[1]
      outlinePts[i++] = p[2]
    }
    const outlineGeo = new THREE.BufferGeometry()
    outlineGeo.setAttribute('position', new THREE.BufferAttribute(outlinePts, 3))
    const outlineMat = new THREE.LineBasicMaterial({ color, depthTest: false })
    const outline = new THREE.LineLoop(outlineGeo, outlineMat)

    const group = new THREE.Group()
    group.add(fillMesh)
    group.add(outline)
    group.position.set(center[0], center[1], center[2])
    // Placeholder scale; updateDiskScale() corrects it from the camera distance
    // next frame (~4 m fallback so it's visible before the first scale tick).
    group.scale.setScalar(PLANE_SCREEN_K * 4)
    this.preview.add(group)
    this.previewPlane = group
  }

  /**
   * Keep the preview quad a constant on-screen size regardless of zoom — called
   * once per frame by the Viewport render loop (feature-detected, same as
   * ProtractorTool's disk). World half-extent = PLANE_SCREEN_K · cameraDistance.
   */
  updateDiskScale(camera: THREE.Camera): void {
    if (this.previewPlane === null) return
    const dist = camera.position.distanceTo(this.previewPlane.position)
    this.previewPlane.scale.setScalar(PLANE_SCREEN_K * dist)
  }

  private _clearPreview(): void {
    if (this.previewPlane === null) return
    for (const child of this.previewPlane.children) {
      if (child instanceof THREE.Mesh || child instanceof THREE.LineLoop) {
        child.geometry.dispose()
        if (child.material instanceof THREE.Material) {
          child.material.dispose()
        }
      }
    }
    this.preview.remove(this.previewPlane)
    this.previewPlane = null
  }
}
