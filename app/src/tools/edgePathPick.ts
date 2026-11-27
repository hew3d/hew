/**
 * pathPick — pick a drawn path under the cursor, and read it as a polyline.
 * Shared by Follow Me and Deform's Bend to Path: both click a drawn line or
 * curve and take its whole connected run.
 */

import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import { chainPath, type PathSegment } from './followMeStart'

type Vec3 = [number, number, number]

/** A run of sketch edges taken as a path. */
export interface EdgePath {
  sketchHandle: bigint
  edgeHandles: bigint[]
}

/**
 * The sketch edge under `ray` and its whole connected island, or null when
 * no sketch edge is hit. A stale island lookup falls back to the one edge.
 */
export function pickEdgePath(scene: WasmScene, ray: Ray): EdgePath | null {
  const pick = scene.pick_sketch_edge(
    ray.origin[0], ray.origin[1], ray.origin[2],
    ray.direction[0], ray.direction[1], ray.direction[2],
  )
  if (pick === undefined) return null
  let sketchHandle: bigint
  let edgeHandle: bigint
  try {
    sketchHandle = pick.sketch()
    edgeHandle = pick.edge()
  } finally {
    pick.free()
  }
  try {
    const island = scene.sketch_edge_island(sketchHandle, edgeHandle)
    const edgeHandles = island !== undefined
      ? [...scene.sketch_island_edges(sketchHandle, island)]
      : [edgeHandle]
    return { sketchHandle, edgeHandles }
  } catch {
    return { sketchHandle, edgeHandles: [edgeHandle] }
  }
}

/**
 * The edges of `island` that chain with `start` without a fork: the run
 * reaches out from `start` through every vertex where exactly two of the
 * island's edges meet, and stops at an end or where three or more meet.
 * A Loft rail drawn from a corner of a rectangle in its own sketch is then
 * the rail alone, not the rail and the rectangle (which fork at the
 * corner). Edges whose ends cannot be read leave the island as it is.
 */
export function edgeRun(
  scene: WasmScene,
  sketchHandle: bigint,
  start: bigint,
  island: readonly bigint[],
): bigint[] {
  const ends = new Map<bigint, [string, string]>()
  const at = new Map<string, bigint[]>()
  try {
    for (const edge of island) {
      const e = scene.sketch_edge_endpoints(sketchHandle, edge)
      if (e === undefined || e.length < 6) return [...island]
      const a = `${e[0]},${e[1]},${e[2]}`
      const b = `${e[3]},${e[4]},${e[5]}`
      ends.set(edge, [a, b])
      for (const k of [a, b]) {
        const list = at.get(k)
        if (list === undefined) at.set(k, [edge])
        else list.push(edge)
      }
    }
  } catch {
    return [...island]
  }
  const first = ends.get(start)
  if (first === undefined) return [start]
  const run = new Set<bigint>([start])
  for (const from of first) {
    let key = from
    for (;;) {
      const meeting = at.get(key) ?? []
      if (meeting.length !== 2) break
      const next = meeting.find((e) => !run.has(e))
      if (next === undefined) break
      run.add(next)
      const [a, b] = ends.get(next) ?? ['', '']
      key = a === key ? b : a
    }
  }
  return island.filter((e) => run.has(e))
}

/**
 * The sketch edge under `ray` and the run of edges chained with it without
 * a fork (`edgeRun`), or null when no sketch edge is hit.
 */
export function pickEdgeRun(scene: WasmScene, ray: Ray): EdgePath | null {
  const pick = scene.pick_sketch_edge(
    ray.origin[0], ray.origin[1], ray.origin[2],
    ray.direction[0], ray.direction[1], ray.direction[2],
  )
  if (pick === undefined) return null
  let sketchHandle: bigint
  let edgeHandle: bigint
  try {
    sketchHandle = pick.sketch()
    edgeHandle = pick.edge()
  } finally {
    pick.free()
  }
  try {
    const island = scene.sketch_edge_island(sketchHandle, edgeHandle)
    const all = island !== undefined ? [...scene.sketch_island_edges(sketchHandle, island)] : [edgeHandle]
    return { sketchHandle, edgeHandles: edgeRun(scene, sketchHandle, edgeHandle, all) }
  } catch {
    return { sketchHandle, edgeHandles: [edgeHandle] }
  }
}

/** A path as the polyline the kernel's Bend to Path takes: the distinct
 *  vertices in order (a closed path leaves off the repeated first point). */
export interface PolylinePath {
  points: Vec3[]
  closed: boolean
}

/** The chained polyline of a run of sketch edges; null when they branch or
 *  are disconnected (the kernel would refuse them too). */
export function edgePathPolyline(
  scene: WasmScene,
  sketchHandle: bigint,
  edgeHandles: readonly bigint[],
): PolylinePath | null {
  const segments: PathSegment[] = []
  try {
    for (const edge of edgeHandles) {
      const ends = scene.sketch_edge_endpoints(sketchHandle, edge)
      if (ends === undefined || ends.length < 6) return null
      segments.push({ a: [ends[0], ends[1], ends[2]], b: [ends[3], ends[4], ends[5]], curve: null })
    }
  } catch {
    return null
  }
  const chained = chainPath(segments)
  return chained === null ? null : polylineOfSegments(chained.segments, chained.closed)
}

/** The polyline of a closed face loop given as flat world coordinates. */
export function loopPolyline(loop: ArrayLike<number>): PolylinePath | null {
  if (loop.length < 9) return null
  const points: Vec3[] = []
  for (let i = 0; i + 2 < loop.length; i += 3) points.push([loop[i], loop[i + 1], loop[i + 2]])
  return { points, closed: true }
}

/** Flat line-segment pairs tracing a polyline, for a highlight. */
export function polylineSegments(path: PolylinePath): number[] {
  const out: number[] = []
  const n = path.points.length
  const last = path.closed ? n : n - 1
  for (let i = 0; i < last; i++) {
    const a = path.points[i]
    const b = path.points[(i + 1) % n]
    out.push(a[0], a[1], a[2], b[0], b[1], b[2])
  }
  return out
}

function polylineOfSegments(segments: PathSegment[], closed: boolean): PolylinePath {
  const points: Vec3[] = segments.map((s) => [s.a[0], s.a[1], s.a[2]])
  if (!closed) {
    const b = segments[segments.length - 1].b
    points.push([b[0], b[1], b[2]])
  }
  return { points, closed }
}
