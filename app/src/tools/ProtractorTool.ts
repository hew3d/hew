/**
 * ProtractorTool — SketchUp-style Protractor: measure an angle and
 * drop an **angular construction guide line** through the apex, along the
 * swept direction.
 *
 * Plane model (matches SketchUp): the protractor's measurement PLANE is
 * inferred and can be locked *before* the apex is even placed.
 *
 *   - Idle/hover phase: every pointer move (regardless of stage —
 *     `onPointerMove` always runs) computes a `candidateNormal`:
 *       - If a plane is locked (`lockedNormal`), use it.
 *       - Else if the snap is on a live world-Object face
 *         (`elementKind === 'face'` with `object`/`element` defined, and
 *         `wasmScene.face_normal` resolves), use that face's normal.
 *       - Else default to world up (0,0,1) — ground = the blue plane.
 *     A disk preview (`THREE.LineLoop`, ~48 points, world-space radius 0.4)
 *     is drawn centered at the snap point, lying in the plane ⊥ the
 *     candidate normal, colored by `axisColorForDirection` (X=red/Y=green/
 *     Z=blue) or neutral purple if off-axis. Locked disks render emphasized
 *     (full opacity + a short normal-axis tick); merely-inferred disks render
 *     lighter, so lock state is visually obvious.
 *   - Shift toggles the plane lock (ignoring keydown autorepeat): unlocked →
 *     locks to the current `candidateNormal`; locked → unlocks. Arrow keys
 *     force-lock to a world axis: Up/Down → Z, Right → X, Left → Y (a
 *     best-effort mapping of SketchUp's arrow-axis locking — flagged for
 *     confirmation, see report). The lock PERSISTS across stages and across
 *     a commit (back to idle, still locked) — only Escape or a Shift-toggle
 *     -off clears it.
 *
 * Gesture (3 picks, or 2 picks + typed angle), once the plane is settled:
 *   1. Click 1 — apex. `planeNormal = lockedNormal ?? candidateNormal ?? +Z`.
 *      The disk preview re-centers on the apex and keeps that orientation
 *      for the rest of the gesture.
 *   2. Click 2 — baseline (the 0° reference ray): the apex→click2 vector,
 *      projected onto the measurement plane and normalized. If that
 *      projection is ~zero (click2 sits on the plane normal through apex),
 *      the click is ignored and we stay waiting for a usable baseline.
 *   3. Move — sweep: cursor direction (apex→cursor, projected + normalized)
 *      vs. the baseline gives a signed angle about the plane normal. The
 *      live preview is a long line through the apex along the swept
 *      direction; the readout is in degrees, reported from the *snapped*
 *      direction when axis-snapped (see Axis-coloring below) so it reads an
 *      exact 90.0° rather than e.g. 89.4° while the guide is visually on-axis.
 *   4. Click 3 (or Enter with a typed angle) — commit: `add_guide_line`
 *      through the apex along the final swept (or typed) direction.
 *   5. Esc cancels the current stage AND clears the plane lock (full reset
 *      to idle, unlocked); `cancel()` does the same.
 *
 * Axis-coloring (SketchUp-style inference cue), unchanged from before: as
 * the swept DIRECTION passes within ~2° of a world axis, the preview line is
 * colored to match (X=red/Y=green/Z=blue, via `axisColorForDirection` in
 * `./axisColors`) and the direction is snapped exactly onto that axis — both
 * for the preview AND the committed guide. This is independent of (and
 * complements) the plane coloring: the plane disk colors by its NORMAL axis,
 * the swept line colors by its OWN direction axis.
 *
 * Explicitly OUT of scope for this slice (deferrals):
 *   - Instanced-geometry apex/hover faces: `face_normal` resolves a normal
 *     for *any* object record (including definition members), not just live
 *     world Objects — see `Document::object` vs. the stricter
 *     `is_world_object` gate `edge_endpoints` uses. So a hover/apex snapped
 *     onto instanced geometry would silently use the unrotated/unscaled
 *     definition-local normal, which can be wrong for a non-identity
 *     instance pose. The `Snap` type doesn't surface an `instance` handle to
 *     tell the two cases apart from TS, so this slice doesn't attempt to
 *     special-case it (mirrors the TapeMeasureTool deferral for parallel
 *     guides off instanced edges).
 *   - Arc rendering for the swept angle: optional polish, skipped — the
 *     straight preview line + degree readout is sufficient feedback.
 *   - A neutral reference line drawn along the baseline direction: optional
 *     polish, skipped to keep the preview to one line (matches how
 *     TapeMeasureTool keeps a single preview primitive per stage).
 *   - The exact arrow↔axis mapping (Up/Down→Z, Right→X, Left→Y) is a
 *     best-effort guess at SketchUp's convention and may need confirmation.
 */

import * as THREE from 'three'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { editNumericBuffer, parseDistance } from './moveInput'
import { projectOntoPlane, signedAngleAboutAxis, rotationAxisAffine } from './transformMath'
import { axisColorForDirection, axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'

export type OnGuideCreated = () => void
export type OnToast = (message: string, code?: string) => void
export type OnMeasurement = (text: string) => void

/** Neutral (off-axis) preview color — matches TapeMeasure's guide preview. */
const NEUTRAL_PREVIEW_COLOR = 0x9933cc
/** Half-length of the previewed angular guide line (meters). */
const GUIDE_HALF_LENGTH = 50
/** Axis-snap tolerance: ~2° either side, expressed as a cosine threshold. */
const AXIS_SNAP_TOL_DOT = Math.cos((2 * Math.PI) / 180)
/** World up, used as the default measurement-plane normal. */
const WORLD_UP: [number, number, number] = [0, 0, 1]
/**
 * Unit radius the disk ring/tick geometry is built at (local space, around
 * the group's local origin). The group itself is positioned at the disk
 * center and uniformly scaled — see `DISK_SCREEN_K` — so the *world* radius
 * tracks camera distance instead of being fixed.
 */
const DISK_UNIT_RADIUS = 1.0
/** Sample count for the disk preview circle. */
const DISK_SEGMENTS = 48
/** Length of the locked-plane normal tick, as a fraction of the unit radius. */
const DISK_TICK_LENGTH = DISK_UNIT_RADIUS * 0.5
/**
 * Scale factor for the screen-constant disk preview, mirroring CueLayer's
 * MARKER_SCREEN_K: worldRadius = DISK_SCREEN_K * cameraDistance. ~0.03 gives
 * a disk noticeably larger than the 0.008 cursor cross — a comfortable
 * protractor size; tunable.
 */
const DISK_SCREEN_K = 0.03

type Stage =
  | { kind: 'idle' }
  | {
      kind: 'awaiting-baseline'
      apex: [number, number, number]
      planeNormal: [number, number, number]
    }
  | {
      kind: 'sweeping'
      apex: [number, number, number]
      planeNormal: [number, number, number]
      baselineDir: [number, number, number]
      /** Last computed signed angle (radians), for VCB sign + readout continuity. */
      lastAngle: number
      /** Last swept direction (post axis-snap if applicable) — what commit uses. */
      sweptDir: [number, number, number]
    }

function normalize(v: [number, number, number]): [number, number, number] | null {
  const len = Math.sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
  if (len < 1e-9) return null
  return [v[0] / len, v[1] / len, v[2] / len]
}

/**
 * Build two unit vectors (u, v) spanning the plane ⊥ `normal`: u is a stable
 * in-plane vector (pick the world axis LEAST parallel to `normal`, then
 * orthogonalize against it), v = normal × u. `normal` must already be unit
 * length.
 */
function planeBasis(
  normal: [number, number, number],
): { u: [number, number, number]; v: [number, number, number] } {
  const [nx, ny, nz] = normal
  // Pick whichever world axis has the smallest |dot| with normal (least parallel).
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

export class ProtractorTool implements Tool {
  readonly name = 'Protractor'

  private stage: Stage = { kind: 'idle' }
  private preview: THREE.Group
  private wasmScene: WasmScene
  private onGuideCreated: OnGuideCreated
  private onToast: OnToast
  private onMeasurementCb: OnMeasurement

  /** VCB buffer — raw string being typed by the user. */
  private typed: string = ''

  /** THREE.js LineSegments for the preview guide line. */
  private previewLine: THREE.LineSegments | null = null
  /** THREE.js Group for the disk preview (circle + optional lock tick). */
  private previewDisk: THREE.Group | null = null

  /** Plane normal inferred from the current hover (face under cursor, or world up). Null only before the first move. */
  private candidateNormal: [number, number, number] | null = null
  /** Locked plane normal, if the user has locked one (Shift toggle or arrow key). Persists across stages/commits. */
  private lockedNormal: [number, number, number] | null = null

  constructor(
    wasmScene: WasmScene,
    previewGroup: THREE.Group,
    onGuideCreated: OnGuideCreated,
    onToast: OnToast,
    onMeasurement: OnMeasurement = () => { /* no-op */ },
  ) {
    this.wasmScene = wasmScene
    this.preview = previewGroup
    this.onGuideCreated = onGuideCreated
    this.onToast = onToast
    this.onMeasurementCb = onMeasurement
  }

  // ── Optional Tool interface extensions ─────────────────────────────────

  capturingInput(): boolean {
    return this.stage.kind !== 'idle'
  }

  /**
   * Keep the disk preview a constant screen size regardless of camera
   * distance — called from the Viewport render loop every frame, mirroring
   * `CueLayer.updateMarkerScale`. No-op when no disk is currently shown.
   */
  updateDiskScale(camera: THREE.Camera): void {
    if (this.previewDisk === null) return
    const dist = camera.position.distanceTo(this.previewDisk.position)
    this.previewDisk.scale.setScalar(DISK_SCREEN_K * dist)
  }

  // ── Tool interface ──────────────────────────────────────────────────────

  onPointerMove(snap: Snap | null, _ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'sweeping') {
      const cursor: [number, number, number] = [snap.x, snap.y, snap.z]
      const rel: [number, number, number] = [
        cursor[0] - this.stage.apex[0],
        cursor[1] - this.stage.apex[1],
        cursor[2] - this.stage.apex[2],
      ]
      const proj = projectOntoPlane(rel[0], rel[1], rel[2], ...this.stage.planeNormal)
      const cursorDir = normalize(proj)
      if (cursorDir === null) return // degenerate (cursor on the plane normal through apex) — keep last

      const angle = signedAngleAboutAxis(
        ...this.stage.planeNormal,
        ...this.stage.baselineDir,
        ...cursorDir,
      )

      const match = axisColorForDirection(cursorDir, AXIS_SNAP_TOL_DOT, axisColorsForTheme(getResolvedTheme()))
      const sweptDir = match !== null ? match.snapped : cursorDir
      const previewColor = match !== null ? match.color : NEUTRAL_PREVIEW_COLOR

      // Report the angle of the SNAPPED direction when axis-snapped, so the
      // readout reads an exact axis angle (e.g. 90.0°) rather than the
      // un-snapped cursor angle (e.g. 89.4°) while the line is visibly on-axis.
      const reportedAngle = match !== null
        ? signedAngleAboutAxis(
            ...this.stage.planeNormal,
            ...this.stage.baselineDir,
            ...sweptDir,
          )
        : angle

      this.stage.lastAngle = angle
      this.stage.sweptDir = sweptDir
      this._updatePreviewLine(this.stage.apex, sweptDir, previewColor)
      this._reportAngleOrTyped(reportedAngle)
      return
    }

    // Idle / awaiting-baseline: live plane-inference preview (the disk).
    const candidate = this.lockedNormal ?? this._resolvePlaneNormal(snap)
    this.candidateNormal = candidate

    if (this.stage.kind === 'idle') {
      const center: [number, number, number] = [snap.x, snap.y, snap.z]
      const normal = this.lockedNormal ?? candidate
      this._updatePreviewDisk(center, normal, this.lockedNormal !== null)
    }
  }

  onPointerDown(snap: Snap | null, _ray: Ray): void {
    if (snap === null) return

    if (this.stage.kind === 'idle') {
      const apex: [number, number, number] = [snap.x, snap.y, snap.z]
      const planeNormal = this.lockedNormal ?? this.candidateNormal ?? WORLD_UP
      this.stage = { kind: 'awaiting-baseline', apex, planeNormal }
      this._updatePreviewDisk(apex, planeNormal, this.lockedNormal !== null)
      return
    }

    if (this.stage.kind === 'awaiting-baseline') {
      const click2: [number, number, number] = [snap.x, snap.y, snap.z]
      const rel: [number, number, number] = [
        click2[0] - this.stage.apex[0],
        click2[1] - this.stage.apex[1],
        click2[2] - this.stage.apex[2],
      ]
      const proj = projectOntoPlane(rel[0], rel[1], rel[2], ...this.stage.planeNormal)
      const baselineDir = normalize(proj)
      if (baselineDir === null) return // ~zero projection — ignore, stay in this stage

      this.stage = {
        kind: 'sweeping',
        apex: this.stage.apex,
        planeNormal: this.stage.planeNormal,
        baselineDir,
        lastAngle: 0,
        sweptDir: baselineDir,
      }
      return
    }

    if (this.stage.kind === 'sweeping') {
      this._commitGuide(this.stage.apex, this.stage.sweptDir)
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
      return
    }

    // Shift toggles the plane lock — meaningful only in the idle/hover phase
    // for now (no-op while sweeping). Guard ev.repeat so keydown autorepeat
    // doesn't toggle the lock on and off repeatedly.
    if (ev.key === 'Shift') {
      if (!ev.repeat && this.stage.kind === 'idle') {
        if (this.lockedNormal === null) {
          if (this.candidateNormal !== null) this.lockedNormal = this.candidateNormal
        } else {
          this.lockedNormal = null
        }
        this._refreshIdleDiskFromLastKnown()
      }
      return
    }

    if (
      this.stage.kind === 'idle'
      && (ev.key === 'ArrowUp' || ev.key === 'ArrowDown' || ev.key === 'ArrowRight' || ev.key === 'ArrowLeft')
    ) {
      ev.preventDefault()
      if (ev.key === 'ArrowUp' || ev.key === 'ArrowDown') {
        this.lockedNormal = [0, 0, 1]
      } else if (ev.key === 'ArrowRight') {
        this.lockedNormal = [1, 0, 0]
      } else {
        this.lockedNormal = [0, 1, 0]
      }
      this._refreshIdleDiskFromLastKnown()
      return
    }

    if (this.stage.kind !== 'sweeping') return

    if (ev.key === 'Enter') {
      const n = parseDistance(this.typed)
      if (n !== null) {
        this._commitFromTyped(n)
      }
      return
    }

    if (
      (ev.key >= '0' && ev.key <= '9') ||
      ev.key === '.' ||
      ev.key === '-' ||
      ev.key === 'Backspace'
    ) {
      this.typed = editNumericBuffer(this.typed, ev.key)
      this.onMeasurementCb(`${this.typed}°`)
    }
  }

  cancel(): void {
    this.lockedNormal = null
    this._resetToIdle()
  }

  // ── Private helpers ─────────────────────────────────────────────────────

  /**
   * Resolve the candidate measurement-plane normal for a hover/apex snap:
   * the world-Object face normal under the snap, if resolvable, else world up.
   * (Does NOT consult `lockedNormal` — callers combine the two.)
   */
  private _resolvePlaneNormal(snap: Snap): [number, number, number] {
    if (snap.elementKind === 'face' && snap.object !== undefined && snap.element !== undefined) {
      try {
        const n = this.wasmScene.face_normal(snap.object, snap.element)
        const normal = normalize([n[0], n[1], n[2]])
        if (normal !== null) return normal
      } catch {
        // Not a live world-Object face (e.g. instanced geometry) — fall through.
      }
    }
    return WORLD_UP
  }

  /**
   * Commit the angular guide: a line through `apex` along `dir`.
   */
  private _commitGuide(apex: [number, number, number], dir: [number, number, number]): void {
    try {
      this.wasmScene.add_guide_line(apex[0], apex[1], apex[2], dir[0], dir[1], dir[2])
      this.onGuideCreated()
    } catch (err) {
      const raw = err instanceof Error ? err.message : String(err)
      this.onToast(`Couldn't create guide line: ${raw}`)
    }
    this._resetToIdle()
  }

  /**
   * Commit an exact typed angle (Enter in the VCB): rotate the baseline
   * direction about the plane normal by the typed degrees, signed in the
   * direction of the last live swept angle (default positive/CCW if none
   * was ever swept — i.e. the cursor never left the baseline).
   */
  private _commitFromTyped(typedDeg: number): void {
    if (this.stage.kind !== 'sweeping') return
    const { apex, planeNormal, baselineDir, lastAngle } = this.stage

    const sign = lastAngle < 0 ? -1 : 1
    const thetaRad = ((typedDeg * sign) * Math.PI) / 180

    const affine = rotationAxisAffine(planeNormal[0], planeNormal[1], planeNormal[2], thetaRad)
    const [m00, m01, m02, , m10, m11, m12, , m20, m21, m22] = affine
    const [bx, by, bz] = baselineDir
    const rotated: [number, number, number] = [
      m00 * bx + m01 * by + m02 * bz,
      m10 * bx + m11 * by + m12 * bz,
      m20 * bx + m21 * by + m22 * bz,
    ]
    const dir = normalize(rotated) ?? baselineDir

    const match = axisColorForDirection(dir, AXIS_SNAP_TOL_DOT, axisColorsForTheme(getResolvedTheme()))
    const finalDir = match !== null ? match.snapped : dir

    this._commitGuide(apex, finalDir)
  }

  /**
   * Reset to idle, but DO NOT clear `lockedNormal` — the lock persists
   * across a commit/cancel-of-stage, SketchUp-style. Only `cancel()` (Esc /
   * explicit tool reset) and the Shift-toggle-off clear it.
   */
  private _resetToIdle(): void {
    this.stage = { kind: 'idle' }
    this.typed = ''
    this._clearPreviewLine()
    this._clearPreviewDisk()
    this.candidateNormal = null
    this.onMeasurementCb('')
  }

  private _reportAngleOrTyped(angleRad: number): void {
    if (this.typed !== '') {
      this.onMeasurementCb(`${this.typed}°`)
      return
    }
    const deg = (angleRad * 180) / Math.PI
    this.onMeasurementCb(`${deg.toFixed(1)}°`)
  }

  /**
   * Re-draw the idle disk preview after a lock-state change (Shift/arrow)
   * that didn't come with a fresh pointer move. Uses the last-known
   * candidate normal (or the just-set lock) and the disk's last center —
   * cheapest correct option is to just re-center on the current disk
   * position if we have one; if no disk has been drawn yet (no move event
   * has landed), there's nothing to redraw until the next move.
   */
  private _refreshIdleDiskFromLastKnown(): void {
    if (this.stage.kind !== 'idle' || this.previewDisk === null) return
    const center = this.previewDisk.position.toArray() as [number, number, number]
    const normal = this.lockedNormal ?? this.candidateNormal ?? WORLD_UP
    this._updatePreviewDisk(center, normal, this.lockedNormal !== null)
  }

  /**
   * Rebuild the preview line: a long line through `origin` along `dir`,
   * colored `color`. Removes the previous preview (if any) first.
   */
  private _updatePreviewLine(
    origin: [number, number, number],
    dir: [number, number, number],
    color: number,
  ): void {
    this._clearPreviewLine()

    const nx = dir[0] * GUIDE_HALF_LENGTH
    const ny = dir[1] * GUIDE_HALF_LENGTH
    const nz = dir[2] * GUIDE_HALF_LENGTH
    const pts = new Float32Array([
      origin[0] - nx, origin[1] - ny, origin[2] - nz,
      origin[0] + nx, origin[1] + ny, origin[2] + nz,
    ])

    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(pts, 3))
    const mat = new THREE.LineBasicMaterial({ color, depthTest: false })
    const line = new THREE.LineSegments(geo, mat)
    this.preview.add(line)
    this.previewLine = line
  }

  private _clearPreviewLine(): void {
    if (this.previewLine === null) return
    this.previewLine.geometry.dispose()
    if (this.previewLine.material instanceof THREE.Material) {
      this.previewLine.material.dispose()
    }
    this.preview.remove(this.previewLine)
    this.previewLine = null
  }

  /**
   * Rebuild the disk preview: a circle (LineLoop) centered at `center`,
   * lying in the plane ⊥ `normal`, colored by `normal`'s axis (or neutral
   * purple off-axis). When `locked` is true, render at full opacity and add
   * a short tick along the normal so the lock is visually obvious; when
   * merely inferred, render at reduced opacity with no tick.
   */
  private _updatePreviewDisk(
    center: [number, number, number],
    normal: [number, number, number],
    locked: boolean,
  ): void {
    this._clearPreviewDisk()

    const unitNormal = normalize(normal) ?? WORLD_UP
    const { u, v } = planeBasis(unitNormal)

    const match = axisColorForDirection(unitNormal, AXIS_SNAP_TOL_DOT, axisColorsForTheme(getResolvedTheme()))
    const color = match !== null ? match.color : NEUTRAL_PREVIEW_COLOR

    const ringPts = new Float32Array(DISK_SEGMENTS * 3)
    for (let i = 0; i < DISK_SEGMENTS; i++) {
      const theta = (i / DISK_SEGMENTS) * Math.PI * 2
      const c = Math.cos(theta), s = Math.sin(theta)
      ringPts[i * 3 + 0] = DISK_UNIT_RADIUS * (c * u[0] + s * v[0])
      ringPts[i * 3 + 1] = DISK_UNIT_RADIUS * (c * u[1] + s * v[1])
      ringPts[i * 3 + 2] = DISK_UNIT_RADIUS * (c * u[2] + s * v[2])
    }
    const ringGeo = new THREE.BufferGeometry()
    ringGeo.setAttribute('position', new THREE.BufferAttribute(ringPts, 3))
    const ringMat = new THREE.LineBasicMaterial({
      color,
      depthTest: false,
      transparent: !locked,
      opacity: locked ? 1 : 0.45,
    })
    const ring = new THREE.LineLoop(ringGeo, ringMat)

    const group = new THREE.Group()
    group.position.set(center[0], center[1], center[2])
    // Placeholder scale — updateDiskScale() will correct it next render frame
    // (mirrors CueLayer's marker fallback: avoids a one-frame flash at the
    // unit radius before the render loop refines it to the screen-constant size).
    group.scale.setScalar(DISK_SCREEN_K * 4) // ~4 m fallback distance
    group.add(ring)

    if (locked) {
      const tickPts = new Float32Array([
        0, 0, 0,
        unitNormal[0] * DISK_TICK_LENGTH,
        unitNormal[1] * DISK_TICK_LENGTH,
        unitNormal[2] * DISK_TICK_LENGTH,
      ])
      const tickGeo = new THREE.BufferGeometry()
      tickGeo.setAttribute('position', new THREE.BufferAttribute(tickPts, 3))
      const tickMat = new THREE.LineBasicMaterial({ color, depthTest: false })
      const tick = new THREE.LineSegments(tickGeo, tickMat)
      group.add(tick)
    }

    this.preview.add(group)
    this.previewDisk = group
  }

  private _clearPreviewDisk(): void {
    if (this.previewDisk === null) return
    for (const child of this.previewDisk.children) {
      if (child instanceof THREE.LineLoop || child instanceof THREE.LineSegments) {
        child.geometry.dispose()
        if (child.material instanceof THREE.Material) {
          child.material.dispose()
        }
      }
    }
    this.preview.remove(this.previewDisk)
    this.previewDisk = null
  }
}
