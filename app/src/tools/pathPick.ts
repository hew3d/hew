/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * pathPick — hover and click a path in the viewport.
 *
 * A path is a drawn sketch line or curve (its whole connected run) or the
 * outer edge of a solid's face. The pick mirrors what Follow Me's path stage
 * accepts, and `pathJson` writes the pick in the form the kernel's
 * path-taking calls read. Only plain objects (not instances) qualify: the
 * face-loop path has no instance form.
 */

import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

export type PathTarget =
  | { kind: 'edges'; sketchHandle: bigint; edgeHandles: bigint[] }
  | { kind: 'face'; objectHandle: bigint; faceHandle: bigint }

export type PathPick =
  | { kind: 'target'; path: PathTarget }
  | { kind: 'ineligible-face'; message: string }
  | { kind: 'none' }

export type PathFaceEligible = (object: bigint, instance: bigint | undefined) => boolean

/** What is under `ray` that can serve as a path: a sketch edge (taken with
 *  its whole connected run) before a solid face. */
export function pickPath(wasm: WasmScene, ray: Ray, faceEligible: PathFaceEligible): PathPick {
  const edgePick = wasm.pick_sketch_edge(
    ray.origin[0], ray.origin[1], ray.origin[2],
    ray.direction[0], ray.direction[1], ray.direction[2],
  )
  if (edgePick !== undefined) {
    let sketchHandle: bigint
    let edgeHandle: bigint
    try {
      sketchHandle = edgePick.sketch()
      edgeHandle = edgePick.edge()
    } finally {
      edgePick.free()
    }
    let edgeHandles = [edgeHandle]
    try {
      const island = wasm.sketch_edge_island(sketchHandle, edgeHandle)
      if (island !== undefined) edgeHandles = [...wasm.sketch_island_edges(sketchHandle, island)]
    } catch {
      // A stale island: the single edge is still a usable path.
    }
    return { kind: 'target', path: { kind: 'edges', sketchHandle, edgeHandles } }
  }
  const facePick = wasm.pick_face(
    ray.origin[0], ray.origin[1], ray.origin[2],
    ray.direction[0], ray.direction[1], ray.direction[2],
  )
  if (facePick === undefined) return { kind: 'none' }
  let object: bigint
  let face: bigint
  let instance: bigint | undefined
  try {
    object = facePick.object()
    face = facePick.face()
    instance = facePick.instance()
  } finally {
    facePick.free()
  }
  if (instance !== undefined || !faceEligible(object, instance)) {
    return {
      kind: 'ineligible-face',
      message: 'That face is inside a group or component. Enter it first, or pick a drawn line or curve.',
    }
  }
  return { kind: 'target', path: { kind: 'face', objectHandle: object, faceHandle: face } }
}

/** The path's line segments (pairs of endpoints, flat xyz) for a highlight,
 *  with a key that changes only when the target does. A stale handle gives
 *  no points. */
export function pathHighlight(wasm: WasmScene, path: PathTarget): { key: string; points: number[] } {
  const points: number[] = []
  try {
    if (path.kind === 'edges') {
      for (const edge of path.edgeHandles) {
        const ends = wasm.sketch_edge_endpoints(path.sketchHandle, edge)
        if (ends !== undefined && ends.length >= 6) points.push(ends[0], ends[1], ends[2], ends[3], ends[4], ends[5])
      }
      return { key: `edges:${path.sketchHandle}:${[...path.edgeHandles].sort().join(',')}`, points }
    }
    const loop = wasm.face_boundary(path.objectHandle, path.faceHandle)
    for (let i = 0; i < loop.length; i += 3) {
      const j = (i + 3) % loop.length
      points.push(loop[i], loop[i + 1], loop[i + 2], loop[j], loop[j + 1], loop[j + 2])
    }
    return { key: `face:${path.objectHandle}:${path.faceHandle}`, points }
  } catch {
    return { key: 'stale', points: [] }
  }
}

/** The path in the JSON form `array_along_path` takes (handles as decimal
 *  strings, which survive JavaScript's 53-bit integers). */
export function pathJson(path: PathTarget): string {
  return path.kind === 'edges'
    ? JSON.stringify({
        kind: 'sketch_edges',
        sketch: path.sketchHandle.toString(),
        edges: path.edgeHandles.map((e) => e.toString()),
      })
    : JSON.stringify({
        kind: 'face_loop',
        object: path.objectHandle.toString(),
        face: path.faceHandle.toString(),
      })
}
