/**
 * DrawPlaneCueLayer — the drawing-plane cue (sketches on any plane, Phase 4
 * — the sketch-planes design §6 bullet 1): a subtle finite grid patch
 * rendered on a draw tool's active NON-ground plane, so the user sees where
 * a gesture is about to land — anchored on a face/plane off the ground, or
 * idle with an arrow-key plane lock and a live hover.
 *
 * Purely visual: never a snap participant, never pickable, and (like
 * `CueLayer`'s dashed guide line) opts OUT of depth testing entirely rather
 * than fighting for a rung on the `depthPolicy.ts` ladder — the patch can
 * float anywhere in space, not just on a surface already on that ladder, so
 * there is no coincident-geometry tie to resolve deterministically. Colored
 * by the plane normal's axis (X=red/Y=green/Z=blue, `axisColorForDirection`)
 * at low opacity when the normal is axis-aligned, neutral gray otherwise —
 * mirrors CueLayer's on-axis coloring.
 *
 * The Viewport calls `update()` after resolving the active tool's
 * `activeDrawPlaneCue()` (duck-typed, like `snapConstraint`) on every
 * pointer move and idle-lock key toggle, and `clear()` on tool switch,
 * document reset, and unmount.
 */
import * as THREE from 'three'
import type { DrawPlane } from '../tools/drawPlane'
import type { V3 } from './geoHelpers'
import { axisColorForDirection, axisColorsForTheme } from './axisColors'
import { getResolvedTheme } from '../settings/theme'

/** Half-extent of the grid patch (meters) — "a few meters", fixed size,
 *  independent of camera zoom (unlike the adaptive InfiniteGrid). */
const PATCH_HALF_EXTENT = 2
/** Cell size (meters) — 4 cells per side across the full patch. */
const PATCH_CELL = 1
/** Neutral (off-axis normal) line color — matches other tool-preview neutrals. */
const NEUTRAL_COLOR = 0x888888
/** Axis-aligned-normal tolerance: tight — only a normal that's essentially
 *  exactly on an axis (as `axisDrawPlane`/ground-locked planes always are)
 *  gets tinted; an arbitrarily tilted sketch plane reads neutral even if
 *  it's close to an axis. */
const AXIS_TOL_DOT = Math.cos((1 * Math.PI) / 180)
const LINE_OPACITY = 0.35

/** Build the grid-line vertex buffer for `plane`, centered on `through`. */
function buildPatchPositions(plane: DrawPlane, through: V3): Float32Array {
  const steps = Math.round((PATCH_HALF_EXTENT * 2) / PATCH_CELL)
  const pts: number[] = []
  const [ox, oy, oz] = through
  const [ux, uy, uz] = plane.u
  const [vx, vy, vz] = plane.v

  for (let i = 0; i <= steps; i++) {
    const s = -PATCH_HALF_EXTENT + i * PATCH_CELL

    // Line parallel to v, at u = s.
    pts.push(
      ox + ux * s + vx * -PATCH_HALF_EXTENT, oy + uy * s + vy * -PATCH_HALF_EXTENT, oz + uz * s + vz * -PATCH_HALF_EXTENT,
      ox + ux * s + vx * PATCH_HALF_EXTENT, oy + uy * s + vy * PATCH_HALF_EXTENT, oz + uz * s + vz * PATCH_HALF_EXTENT,
    )
    // Line parallel to u, at v = s.
    pts.push(
      ox + vx * s + ux * -PATCH_HALF_EXTENT, oy + vy * s + uy * -PATCH_HALF_EXTENT, oz + vz * s + uz * -PATCH_HALF_EXTENT,
      ox + vx * s + ux * PATCH_HALF_EXTENT, oy + vy * s + uy * PATCH_HALF_EXTENT, oz + vz * s + uz * PATCH_HALF_EXTENT,
    )
  }

  return new Float32Array(pts)
}

/** The patch's line color: the plane normal's axis color (theme-aware) when
 *  the normal is (essentially exactly) axis-aligned, neutral gray otherwise. */
function patchColor(plane: DrawPlane): number {
  const match = axisColorForDirection(plane.normal, AXIS_TOL_DOT, axisColorsForTheme(getResolvedTheme()))
  return match !== null ? match.color : NEUTRAL_COLOR
}

export class DrawPlaneCueLayer {
  readonly group: THREE.Group
  private mesh: THREE.LineSegments | null = null

  constructor() {
    this.group = new THREE.Group()
    this.group.name = 'DrawPlaneCueLayer'
  }

  /** Rebuild the cue for `cue` (from the active tool's `activeDrawPlaneCue()`),
   *  or clear it when `cue` is null. */
  update(cue: { plane: DrawPlane; through: V3 } | null): void {
    this._dispose()
    if (cue === null) return

    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(buildPatchPositions(cue.plane, cue.through), 3))
    const mat = new THREE.LineBasicMaterial({
      color: patchColor(cue.plane),
      transparent: true,
      opacity: LINE_OPACITY,
      depthTest: false,
    })
    const mesh = new THREE.LineSegments(geo, mat)
    mesh.name = 'DrawPlaneCue'
    this.mesh = mesh
    this.group.add(mesh)
  }

  /** Dispose and hide the cue — tool switch, cancel, document reset. */
  clear(): void {
    this._dispose()
  }

  private _dispose(): void {
    if (this.mesh === null) return
    this.mesh.geometry.dispose()
    if (this.mesh.material instanceof THREE.Material) this.mesh.material.dispose()
    this.group.remove(this.mesh)
    this.mesh = null
  }
}
