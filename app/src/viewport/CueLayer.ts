/**
 * CueLayer — a THREE.Group rebuilt on every pointer move.
 *
 * Renders:
 *   - Snap marker glyph at the snap point, colored by kind:
 *       endpoint   → green   #00cc44
 *       midpoint   → cyan    #00cccc
 *       on-edge    → red     #cc2200
 *       on-face    → blue    #0055cc
 *       on-guide   → purple  #9933cc  (construction guide)
 *       on-axis    → axis color (X=red, Y=green, Z=blue; unknown=magenta)
 *       ground     → gray    #888888  (fallback — no kernel snap)
 *       plane      → gray    #888888  (constraint-plane fallback, same role)
 *       other      → white   #ffffff
 *   - Dashed guide line through the snap point along direction() when present.
 *
 * The group is added to the scene once; call update() on every pointer move
 * to rebuild its children; call updateMarkerScale(camera) every render frame
 * so the cross marker stays a constant screen size regardless of zoom.
 */

import * as THREE from 'three'
import type { Snap } from '../tools/types'
import { axisColorsForTheme } from './axisColors'
import { getResolvedTheme } from '../settings/theme'

const SNAP_COLORS: Record<string, number> = {
  endpoint: 0x00cc44,
  center: 0x00aa88,
  quadrant: 0x00aa88,
  tangent: 0xb050d0,
  midpoint: 0x00cccc,
  intersection: 0xffaa00,
  'on-edge': 0xcc2200,
  'on-face': 0x0055cc,
  'on-guide': 0x9933cc,
  ground: 0x888888,
  plane: 0x888888,
}

/** Half-length of the dashed guide line (meters) */
const GUIDE_HALF_LENGTH = 5

function snapColor(kind: string): number {
  if (kind in SNAP_COLORS) return SNAP_COLORS[kind]
  if (kind === 'on-axis') return 0xcc00cc // magenta fallback if direction unknown
  return 0xffffff
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

    if (snap === null) return

    const pos = new THREE.Vector3(snap.x, snap.y, snap.z)

    // Determine color
    let color = snapColor(snap.kind)
    if (snap.kind === 'on-axis' && snap.direction !== undefined) {
      // Infer axis color from dominant direction component (—
      // theme-aware: light/dark axis colors differ, per 01_design_tokens.md).
      const [dx, dy, dz] = snap.direction
      const abs = [Math.abs(dx), Math.abs(dy), Math.abs(dz)]
      const axis = abs.indexOf(Math.max(...abs)) as 0 | 1 | 2
      color = axisColorsForTheme(getResolvedTheme())[axis]
    }

    // The snap POINT itself is now marked by the DOM `SnapDot` overlay
    // (Refinement pass, issue B) — a bright pulsing dot that reads clearly
    // against any geometry, unlike the thin 1px three.js cross this used to
    // draw. CueLayer keeps only the in-scene dashed GUIDE line (which the DOM
    // layer can't do — it needs depth + world extent along the axis/edge).
    if (snap.direction !== undefined) {
      this.group.add(buildGuideLine(pos, snap.direction, color))
    }
  }

  /** Clear without disposing (called on cleanup) */
  clear(): void {
    this.group.clear()
  }
}
