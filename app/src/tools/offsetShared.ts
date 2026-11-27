/**
 * offsetShared — small helpers the Shell and Project tools share: reading a
 * kernel change into the ids the renderer refreshes, a face outline as fat
 * line segments, a translucent fill for a face, and the default wall depth.
 *
 * Pure of tool state; the only three.js use is the fill mesh.
 */

import * as THREE from 'three'
import type { DocChangeJs } from '../wasm/loader'

/** The ids a committed change touched, read once so the wasm object can be freed. */
export interface TouchedIds {
  objectIds: bigint[]
  componentIds: bigint[]
  instanceIds: bigint[]
}

/** Read and free a `DocChangeJs`. */
export function takeTouched(change: DocChangeJs): TouchedIds {
  try {
    return {
      objectIds: Array.from(change.objects_touched()),
      componentIds: Array.from(change.components_touched()),
      instanceIds: Array.from(change.instances_touched()),
    }
  } finally {
    change.free()
  }
}

/** A closed polygon (`[x,y,z, …]` ring) as flat segment endpoint pairs. */
export function ringSegments(ring: ArrayLike<number>): number[] {
  const n = Math.floor(ring.length / 3)
  const out: number[] = []
  for (let i = 0; i < n; i += 1) {
    const j = (i + 1) % n
    out.push(ring[i * 3], ring[i * 3 + 1], ring[i * 3 + 2], ring[j * 3], ring[j * 3 + 1], ring[j * 3 + 2])
  }
  return out
}

/** A translucent fill for a planar polygon ring (`[x,y,z, …]`), or null for
 *  a ring that does not triangulate. Ear clipping runs in the ring's own
 *  plane (Newell normal). */
export function ringFill(ring: ArrayLike<number>, color: number, opacity = 0.35): THREE.Mesh | null {
  const n = Math.floor(ring.length / 3)
  if (n < 3) return null
  let nx = 0
  let ny = 0
  let nz = 0
  for (let i = 0; i < n; i += 1) {
    const j = (i + 1) % n
    const ax = ring[i * 3]
    const ay = ring[i * 3 + 1]
    const az = ring[i * 3 + 2]
    const bx = ring[j * 3]
    const by = ring[j * 3 + 1]
    const bz = ring[j * 3 + 2]
    nx += (ay - by) * (az + bz)
    ny += (az - bz) * (ax + bx)
    nz += (ax - bx) * (ay + by)
  }
  const len = Math.hypot(nx, ny, nz)
  if (!(len > 0)) return null
  const normal = new THREE.Vector3(nx / len, ny / len, nz / len)
  const helper = Math.abs(normal.x) < 0.9 ? new THREE.Vector3(1, 0, 0) : new THREE.Vector3(0, 1, 0)
  const u = new THREE.Vector3().crossVectors(normal, helper).normalize()
  const v = new THREE.Vector3().crossVectors(normal, u)
  const flat: THREE.Vector2[] = []
  for (let i = 0; i < n; i += 1) {
    const p = new THREE.Vector3(ring[i * 3], ring[i * 3 + 1], ring[i * 3 + 2])
    flat.push(new THREE.Vector2(p.dot(u), p.dot(v)))
  }
  const tris = THREE.ShapeUtils.triangulateShape(flat, [])
  if (tris.length === 0) return null
  const geo = new THREE.BufferGeometry()
  geo.setAttribute('position', new THREE.BufferAttribute(Float32Array.from(Array.from(ring).slice(0, n * 3)), 3))
  geo.setIndex(tris.flat())
  const mat = new THREE.MeshBasicMaterial({
    color,
    transparent: true,
    opacity,
    depthWrite: false,
    depthTest: false,
    side: THREE.DoubleSide,
  })
  const mesh = new THREE.Mesh(geo, mat)
  mesh.renderOrder = 994
  return mesh
}

/** Dispose a fill mesh made by `ringFill`. */
export function disposeFill(mesh: THREE.Mesh): void {
  mesh.geometry.dispose()
  ;(mesh.material as THREE.Material).dispose()
}

/** Round `value` down to a 1, 2 or 5 times a power of ten. */
export function niceStep(value: number): number {
  if (!(value > 0) || !Number.isFinite(value)) return 0
  const exp = Math.floor(Math.log10(value))
  const base = 10 ** exp
  const m = value / base
  const pick = m >= 5 ? 5 : m >= 2 ? 2 : 1
  return pick * base
}

/**
 * A wall thickness that suits an object: about a twentieth of its smallest
 * side, rounded down to a plain number, never under a millimeter. `positions`
 * is the object's flat vertex array.
 */
export function defaultWallThickness(positions: ArrayLike<number>): number {
  if (positions.length < 3) return 0.01
  const min = [Infinity, Infinity, Infinity]
  const max = [-Infinity, -Infinity, -Infinity]
  for (let i = 0; i + 2 < positions.length; i += 3) {
    for (let k = 0; k < 3; k += 1) {
      const c = positions[i + k]
      if (c < min[k]) min[k] = c
      if (c > max[k]) max[k] = c
    }
  }
  const sides = [0, 1, 2].map((k) => max[k] - min[k]).filter((s) => s > 1e-6)
  if (sides.length === 0) return 0.01
  return Math.max(0.001, niceStep(Math.min(...sides) / 20))
}
