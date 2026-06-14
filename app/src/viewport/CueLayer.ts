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
 * to rebuild its children.
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

/** Size of the snap marker cross glyph (meters) */
const MARKER_HALF_SIZE = 0.04

function snapColor(kind: string): number {
  if (kind in SNAP_COLORS) return SNAP_COLORS[kind]
  if (kind === 'on-axis') return 0xcc00cc // magenta fallback if direction unknown
  return 0xffffff
}

function buildCrossMarker(pos: THREE.Vector3, color: number): THREE.LineSegments {
  const s = MARKER_HALF_SIZE
  const pts = new Float32Array([
    // horizontal bar
    pos.x - s, pos.y, pos.z,
    pos.x + s, pos.y, pos.z,
    // vertical bar
    pos.x, pos.y - s, pos.z,
    pos.x, pos.y + s, pos.z,
    // depth bar
    pos.x, pos.y, pos.z - s,
    pos.x, pos.y, pos.z + s,
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
      // Infer axis color from dominant direction component
      const [dx, dy, dz] = snap.direction
      const abs = [Math.abs(dx), Math.abs(dy), Math.abs(dz)]
      const axis = abs.indexOf(Math.max(...abs)) as 0 | 1 | 2
      color = AXIS_COLORS[axis]
    }

    this.group.add(buildCrossMarker(pos, color))

    if (snap.direction !== undefined) {
      this.group.add(buildGuideLine(pos, snap.direction, color))
    }
  }

  /** Clear without disposing (called on cleanup) */
  clear(): void {
    this.group.clear()
  }
}
