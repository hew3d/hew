/**
 * profilePick — pick a profile (a sketch region, or a solid face) under the
 * cursor. Shared by Follow Me and Revolve: both start from "which shape am I
 * sweeping", and both must read the same answer from the kernel — including
 * the instance-scoped region pick used inside a component's own editing
 * context.
 *
 * The wasm pick objects are freed here; callers only see plain handles.
 */

import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

export type Vec3 = [number, number, number]

/** A sketch region under the cursor. */
export interface RegionHit {
  sketchHandle: bigint
  regionHandle: bigint
}

/** A solid face under the cursor. `instance` is set for a face reached
 *  through a component placement. */
export interface FaceHit {
  objectHandle: bigint
  faceHandle: bigint
  instance: bigint | undefined
}

/**
 * The sketch region under `ray`, or null. `pick_sketch_region` only walks
 * world-tree sketches, so inside an instance's own editing context the
 * scoped sibling is used instead (`instance` non-null).
 */
export function pickRegionAt(
  scene: WasmScene,
  ray: Ray,
  instance: bigint | null,
): RegionHit | null {
  const { origin: o, direction: d } = ray
  const pick = instance !== null
    ? scene.pick_sketch_region_in_instance(instance, o[0], o[1], o[2], d[0], d[1], d[2])
    : scene.pick_sketch_region(o[0], o[1], o[2], d[0], d[1], d[2])
  if (pick === undefined) return null
  try {
    return { sketchHandle: pick.sketch(), regionHandle: pick.region() }
  } finally {
    pick.free()
  }
}

/** The solid face under `ray`, or null. */
export function pickFaceAt(scene: WasmScene, ray: Ray): FaceHit | null {
  const { origin: o, direction: d } = ray
  const pick = scene.pick_face(o[0], o[1], o[2], d[0], d[1], d[2])
  if (pick === undefined) return null
  try {
    return { objectHandle: pick.object(), faceHandle: pick.face(), instance: pick.instance() }
  } finally {
    pick.free()
  }
}

/** A region's outer boundary as vertices; empty for a stale handle. */
export function regionRing(scene: WasmScene, sketch: bigint, region: bigint): Vec3[] {
  const ring: Vec3[] = []
  try {
    const loop = scene.region_boundary(sketch, region)
    for (let i = 0; i + 2 < loop.length; i += 3) ring.push([loop[i], loop[i + 1], loop[i + 2]])
  } catch {
    // Stale region mid-query: no ring.
  }
  return ring
}

/** A closed ring as a flat line-segment list (a,b, b,c, … z,a) for fat lines. */
export function ringSegments(ring: ArrayLike<ArrayLike<number>>): number[] {
  const points: number[] = []
  for (let i = 0; i < ring.length; i++) {
    const j = (i + 1) % ring.length
    points.push(ring[i][0], ring[i][1], ring[i][2], ring[j][0], ring[j][1], ring[j][2])
  }
  return points
}

/**
 * A point strictly inside a (possibly concave) planar ring: the centroid of
 * the first fan triangle whose centroid lies inside the ring. Falls back to
 * the vertex mean.
 */
export function interiorPoint(ring: readonly Vec3[], normal: Vec3): Vec3 {
  const n = ring.length
  const mean: Vec3 = [0, 0, 0]
  for (const p of ring) {
    mean[0] += p[0] / n
    mean[1] += p[1] / n
    mean[2] += p[2] / n
  }
  if (n < 3) return mean
  // Project to 2D by dropping the normal's dominant axis.
  const ax = Math.abs(normal[0]) >= Math.abs(normal[1]) && Math.abs(normal[0]) >= Math.abs(normal[2])
    ? 0
    : Math.abs(normal[1]) >= Math.abs(normal[2]) ? 1 : 2
  const [iu, iv] = ax === 0 ? [1, 2] : ax === 1 ? [0, 2] : [0, 1]
  const inside = (p: Vec3): boolean => {
    let c = false
    for (let i = 0, j = n - 1; i < n; j = i++) {
      const a = ring[i], b = ring[j]
      if ((a[iv] > p[iv]) !== (b[iv] > p[iv]) &&
          p[iu] < ((b[iu] - a[iu]) * (p[iv] - a[iv])) / (b[iv] - a[iv]) + a[iu]) c = !c
    }
    return c
  }
  if (inside(mean)) return mean
  const candidates: Vec3[] = []
  for (let i = 1; i + 1 < n; i++) {
    const a = ring[0], b = ring[i], c = ring[i + 1]
    candidates.push([(a[0] + b[0] + c[0]) / 3, (a[1] + b[1] + c[1]) / 3, (a[2] + b[2] + c[2]) / 3])
  }
  return candidates.find(inside) ?? mean
}

/** A ray that looks straight at the plane of `ring` from its normal side,
 *  through an interior point — to find the same profile again by handle-free
 *  picking (an undo re-keys handles, a retype must not trust old ones). */
export function rayAtProfile(ring: readonly Vec3[], normal: Vec3): Ray {
  const p = interiorPoint(ring, normal)
  const len = Math.hypot(normal[0], normal[1], normal[2]) || 1
  const n: Vec3 = [normal[0] / len, normal[1] / len, normal[2] / len]
  return {
    origin: [p[0] + n[0] * 0.25, p[1] + n[1] * 0.25, p[2] + n[2] * 0.25],
    direction: [-n[0], -n[1], -n[2]],
  }
}
