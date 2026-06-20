/**
 * CueLayer — a THREE.Group rebuilt on every pointer move.
 *
 * Renders:
 *   - Snap marker glyph at the snap point, colored by kind:
 *       endpoint   → green   #00cc44
 *       midpoint   → cyan    #00cccc
 *       on-edge    → red     #cc2200
 *       on-face    → blue    #0055cc
 *       on-axis    → axis color (X=red, Y=green, Z=blue; unknown=magenta)
 *       ground     → gray    #888888  (fallback — no kernel snap)
 *       other      → white   #ffffff
 *   - Dashed guide line through the snap point along direction() when present.
 *
 * The group is added to the scene once; call update() on every pointer move
 * to rebuild its children; call updateMarkerScale(camera) every render frame
 * so the cross marker stays a constant screen size regardless of zoom.
 */

import * as THREE from 'three'
import type { Snap } from '../tools/types'

const SNAP_COLORS: Record<string, number> = {
  endpoint: 0x00cc44,
  midpoint: 0x00cccc,
  intersection: 0xffaa00,
  'on-edge': 0xcc2200,
  'on-face': 0x0055cc,
  ground: 0x888888,
}

const AXIS_COLORS: [number, number, number] = [0xff2222, 0x22cc22, 0x2222ff]

/** Half-length of the dashed guide line (meters) */
const GUIDE_HALF_LENGTH = 5

/**
 * Scale factor for the screen-constant cross marker.
 * worldSize = MARKER_SCREEN_K * distanceToCamera
 * At k=0.008 and 4 m camera distance: half-size ≈ 0.032 m → ~24 px at 800 px
 * viewport height (FOV 45°). Comfortable and clearly visible without dominating.
 */
const MARKER_SCREEN_K = 0.008

function snapColor(kind: string): number {
  if (kind in SNAP_COLORS) return SNAP_COLORS[kind]
  if (kind === 'on-axis') return 0xcc00cc // magenta fallback if direction unknown
  return 0xffffff
}

/**
 * Build a unit cross marker centered at the local origin (arms ±1 along each
 * world axis). Position and uniform scale are set via Object3D properties so
 * the render loop can update scale without rebuilding geometry.
 */
function buildCrossMarker(color: number): THREE.LineSegments {
  const pts = new Float32Array([
    // horizontal bar (±X)
    -1, 0, 0,
     1, 0, 0,
    // vertical bar (±Y)
    0, -1, 0,
    0,  1, 0,
    // depth bar (±Z)
    0, 0, -1,
    0, 0,  1,
  ])
  const geo = new THREE.BufferGeometry()
  geo.setAttribute('position', new THREE.BufferAttribute(pts, 3))
  const mat = new THREE.LineBasicMaterial({ color, depthTest: false })
  const ls = new THREE.LineSegments(geo, mat)
  ls.renderOrder = 999
  return ls
}

function buildGuideLine(
  pos: THREE.Vector3,
  direction: [number, number, number],
  color: number,
): THREE.LineSegments {
  const [dx, dy, dz] = direction
  const len = Math.sqrt(dx * dx + dy * dy + dz * dz)
  if (len < 1e-9) return new THREE.LineSegments()

  const nx = (dx / len) * GUIDE_HALF_LENGTH
  const ny = (dy / len) * GUIDE_HALF_LENGTH
  const nz = (dz / len) * GUIDE_HALF_LENGTH

  // Dashed guide line: 10 segments alternating solid/gap
  const SEGMENTS = 20
  const pts: number[] = []
  for (let i = 0; i < SEGMENTS; i++) {
    // Alternate: even=solid, odd=gap
    if (i % 2 !== 0) continue
    const t0 = (i / SEGMENTS) * 2 - 1 // -1 to +1
    const t1 = ((i + 1) / SEGMENTS) * 2 - 1
    pts.push(
      pos.x + t0 * nx, pos.y + t0 * ny, pos.z + t0 * nz,
      pos.x + t1 * nx, pos.y + t1 * ny, pos.z + t1 * nz,
    )
  }

  const geo = new THREE.BufferGeometry()
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(pts), 3))
  const mat = new THREE.LineBasicMaterial({
    color,
    depthTest: false,
    transparent: true,
    opacity: 0.6,
  })
  const ls = new THREE.LineSegments(geo, mat)
  ls.renderOrder = 998
  return ls
}

export class CueLayer {
  readonly group: THREE.Group
  /** The live cross marker, kept between frames to update its scale. */
  private _marker: THREE.LineSegments | null = null
  /** Current snap position in world space. */
  private _snapPos: THREE.Vector3 | null = null

  constructor() {
    this.group = new THREE.Group()
    this.group.name = 'CueLayer'
  }

  /** Rebuild the cue layer for the current snap result */
  update(snap: Snap | null): void {
    // Dispose old geometry
    this.group.traverse((child) => {
      if (child instanceof THREE.LineSegments) {
        child.geometry.dispose()
        if (child.material instanceof THREE.Material) {
          child.material.dispose()
        }
      }
    })
    this.group.clear()
    this._marker = null
    this._snapPos = null

    if (snap === null) return

    const pos = new THREE.Vector3(snap.x, snap.y, snap.z)
    this._snapPos = pos.clone()

    // Determine color
    let color = snapColor(snap.kind)
    if (snap.kind === 'on-axis' && snap.direction !== undefined) {
      // Infer axis color from dominant direction component
      const [dx, dy, dz] = snap.direction
      const abs = [Math.abs(dx), Math.abs(dy), Math.abs(dz)]
      const axis = abs.indexOf(Math.max(...abs)) as 0 | 1 | 2
      color = AXIS_COLORS[axis]
    }

    // Build unit cross at origin; position+scale set by updateMarkerScale()
    const marker = buildCrossMarker(color)
    marker.position.copy(pos)
    // Set a placeholder scale — updateMarkerScale() will correct it next frame
    marker.scale.setScalar(MARKER_SCREEN_K * 4) // ~4 m fallback distance
    this._marker = marker
    this.group.add(marker)

    if (snap.direction !== undefined) {
      this.group.add(buildGuideLine(pos, snap.direction, color))
    }
  }

  /**
   * Call once per render frame (inside the animation loop, after controls.update()).
   * Scales the cross marker so it stays a constant screen size regardless of
   * how far the camera is from the snap point.
   *
   * Formula: worldHalfSize = MARKER_SCREEN_K * distanceToMarker
   * This keeps the projected pixel footprint constant for any perspective view.
   */
  updateMarkerScale(camera: THREE.Camera): void {
    if (this._marker === null || this._snapPos === null) return
    const dist = camera.position.distanceTo(this._snapPos)
    const scale = MARKER_SCREEN_K * dist
    this._marker.scale.setScalar(scale)
  }

  /** Clear without disposing (called on cleanup) */
  clear(): void {
    this._marker = null
    this._snapPos = null
    this.group.clear()
  }
}
