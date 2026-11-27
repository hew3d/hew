/**
 * filletPick — what is under the cursor, for the Fillet tool.
 *
 * One pointer ray resolves to one of: a sharp edge (its whole chain), a face
 * (the edges of the loop nearest the cursor), an existing blend (to resize or
 * remove), a drawn 2D corner, a seam between two separate objects, or a
 * smooth edge that cannot be rounded. All geometry here is in the picked
 * object's own frame; `Frame` carries the placing instance's pose so the
 * caller can draw and size in the right space.
 */

import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import {
  applyAffine3x4,
  applyAffine3x4Linear,
  invertAffine3x4,
  type Affine3x4,
  type V3,
} from '../viewport/geoHelpers'
import { ctxOfFace, ctxOfSketch, readPlaneUnits, type PlaneUnitsScene } from './planeUnits'

/** How far the cursor may be from an edge or a drawn corner and still pick it,
 *  in CSS pixels. */
export const PICK_APERTURE_PX = 8

/** The placement of the picked object: identity for a world object, else its
 *  instance's pose. Maps the object's frame to the world and back. */
export interface Frame {
  instance: bigint | null
  /** Row-major 3x4 pose, or null for the world frame. */
  pose: Affine3x4 | null
  inverse: number[] | null
  /** The pose's uniform scale (1 for the world). */
  scale: number
}

export const WORLD_FRAME: Frame = { instance: null, pose: null, inverse: null, scale: 1 }

export function frameOf(scene: WasmScene, instance: bigint | null): Frame | null {
  if (instance === null) return WORLD_FRAME
  const pose = scene.instance_pose(instance)
  if (pose === undefined) return null
  const m = Array.from(pose) as unknown as Affine3x4
  const inverse = invertAffine3x4(m)
  if (inverse === null) return null
  const scale = Math.hypot(m[0], m[4], m[8])
  return { instance, pose: m, inverse, scale: scale > 0 ? scale : 1 }
}

export function toWorldPoint(f: Frame, p: V3): V3 {
  return f.pose === null ? p : applyAffine3x4(f.pose, p)
}

export function toLocalPoint(f: Frame, p: V3): V3 {
  return f.inverse === null ? p : applyAffine3x4(f.inverse as unknown as Affine3x4, p)
}

export function toLocalDir(f: Frame, v: V3): V3 {
  return f.inverse === null ? v : applyAffine3x4Linear(f.inverse as unknown as Affine3x4, v)
}

export function rayToLocal(f: Frame, ray: Ray): Ray {
  if (f.inverse === null) return ray
  return {
    origin: toLocalPoint(f, ray.origin),
    direction: toLocalDir(f, ray.direction),
  }
}

/** A logical edge chain, copied out of the wasm report. */
export interface ChainInfo {
  edges: bigint[]
  /** Line segments, xyz pairs, object-local. */
  segments: Float32Array
  dihedral: number
  /** 0 convex, 1 concave, 2 mixed. */
  convexity: number
  closed: boolean
}

/** A drawn corner the tool can round: the vertex, its two edges' far ends,
 *  and how to commit it. */
export interface CornerTarget {
  /** Where the corner is, world. */
  point: V3
  p: V3
  q: V3
  /** The drawing plane's world normal. */
  normal: V3
  /** The context string of the drawing (for a face drawing). */
  ctx: string
  commit:
    | { kind: 'sketch'; sketch: bigint; vertex: bigint }
    | { kind: 'face'; object: bigint }
}

export type Hover =
  | { kind: 'none' }
  /** Separate objects meet here; `names` are theirs (empty when unnamed). */
  | { kind: 'seam'; names: string[]; /** The group they are pieces of ('' unnamed). */ group?: string }
  | { kind: 'smooth' }
  | {
      kind: 'edge'
      object: bigint
      frame: Frame
      edge: bigint
      /** The face under the cursor, when it borders the edge. */
      hitFace: bigint | null
      chain: ChainInfo
      point: V3
    }
  | {
      kind: 'face'
      object: bigint
      frame: Frame
      face: bigint
      chain: ChainInfo
      point: V3
    }
  | {
      kind: 'blend'
      object: bigint
      frame: Frame
      blendId: number
      face: bigint
      faces: bigint[]
      point: V3
    }
  | { kind: 'corner'; corner: CornerTarget }

export interface PickOptions {
  /** Ctrl/Cmd held: do not extend a pick along tangent continuations. */
  noPropagate: boolean
  /** World-per-pixel at a camera distance (cached from the render loop). */
  worldPerPixel: ((dist: number) => number) | null
}

function readChain(chain: {
  edges(): BigUint64Array
  segments(): Float32Array
  dihedral(): number
  convexity(): number
  closed(): boolean
  free(): void
}): ChainInfo {
  try {
    return {
      edges: Array.from(chain.edges()),
      segments: chain.segments(),
      dihedral: chain.dihedral(),
      convexity: chain.convexity(),
      closed: chain.closed(),
    }
  } finally {
    chain.free()
  }
}

/**
 * The blend under `ray`, by the face it hits alone (no edge or corner takes
 * priority): how an edit re-opens on the band a kind switch just rebuilt,
 * where `resolveHover` would prefer the band's own boundary edge.
 */
export function blendHoverAt(scene: WasmScene, ray: Ray): Extract<Hover, { kind: 'blend' }> | null {
  const fp = scene.pick_face(
    ray.origin[0], ray.origin[1], ray.origin[2],
    ray.direction[0], ray.direction[1], ray.direction[2],
  )
  if (fp === undefined) return null
  let object: bigint
  let face: bigint
  let instance: bigint | null
  let point: V3
  try {
    const depth = fp.depth()
    const dl = Math.hypot(ray.direction[0], ray.direction[1], ray.direction[2]) || 1
    object = fp.object()
    face = fp.face()
    instance = fp.instance() ?? null
    point = [
      ray.origin[0] + (ray.direction[0] / dl) * depth,
      ray.origin[1] + (ray.direction[1] / dl) * depth,
      ray.origin[2] + (ray.direction[2] / dl) * depth,
    ]
  } finally {
    fp.free()
  }
  let blendId: number | undefined
  try {
    blendId = scene.blend_of_face(object, face)
  } catch {
    return null
  }
  if (blendId === undefined) return null
  const frame = frameOf(scene, instance)
  if (frame === null) return null
  return {
    kind: 'blend',
    object,
    frame,
    blendId,
    face,
    faces: Array.from(scene.blend_faces(object, blendId)),
    point: toLocalPoint(frame, point),
  }
}

/** The aperture (world meters) for `PICK_APERTURE_PX` at `depth`. */
export function apertureAt(opts: PickOptions, depth: number): number {
  const wpp = opts.worldPerPixel
  return wpp !== null ? PICK_APERTURE_PX * wpp(Math.max(depth, 1e-3)) : 0.01 * Math.max(depth, 0.1)
}

function dist3(a: V3, b: V3): number {
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2])
}

/** The nearest drawing vertex of `ctx`'s listing to `at`, within `tol`, with
 *  the far ends of exactly two edges meeting there. */
function cornerFromListing(
  scene: WasmScene,
  ctx: string,
  at: V3,
  tol: number,
): { point: V3; p: V3; q: V3 } | null {
  const pu = readPlaneUnits(scene as unknown as PlaneUnitsScene, ctx)
  if (pu === null) return null
  let best: { d: number; v: V3 } | null = null
  for (const u of pu.units) {
    for (const e of u.edges) {
      for (const v of [e.a, e.b]) {
        const d = dist3(v, at)
        if (d <= tol && (best === null || d < best.d)) best = { d, v: [v[0], v[1], v[2]] }
      }
    }
  }
  if (best === null) return null
  const ends: V3[] = []
  for (const u of pu.units) {
    for (const e of u.edges) {
      if (dist3(e.a, best.v) < 1e-7) ends.push([e.b[0], e.b[1], e.b[2]])
      else if (dist3(e.b, best.v) < 1e-7) ends.push([e.a[0], e.a[1], e.a[2]])
    }
  }
  if (ends.length !== 2) return null
  return { point: best.v, p: ends[0], q: ends[1] }
}

function planeNormalOfCtx(ctx: string): V3 {
  try {
    const v = JSON.parse(ctx) as { plane?: number[] }
    if (Array.isArray(v.plane) && v.plane.length === 6) return [v.plane[3], v.plane[4], v.plane[5]]
  } catch {
    /* fall through */
  }
  return [0, 0, 1]
}

/** A drawn corner under the ray, on a free sketch or on a solid's face. */
function pickCorner(scene: WasmScene, ray: Ray, faceHit: { object: bigint; face: bigint; point: V3; instance: bigint | null; depth: number } | null, aperture: number): Hover | null {
  const vp = scene.pick_sketch_vertex(
    ray.origin[0], ray.origin[1], ray.origin[2],
    ray.direction[0], ray.direction[1], ray.direction[2],
  )
  if (vp !== undefined) {
    let sketch: bigint
    let vertex: bigint
    let at: V3
    try {
      sketch = vp.sketch()
      vertex = vp.vertex()
      at = [vp.x(), vp.y(), vp.z()]
    } finally {
      vp.free()
    }
    const found = scene.sketch_blendable_corner_at(sketch, at[0], at[1], at[2], aperture)
    if (found === vertex) {
      const ctx = ctxOfSketch(scene, sketch, null)
      if (ctx !== null) {
        const c = cornerFromListing(scene, ctx, at, aperture)
        if (c !== null) {
          return {
            kind: 'corner',
            corner: { ...c, normal: planeNormalOfCtx(ctx), ctx, commit: { kind: 'sketch', sketch, vertex } },
          }
        }
      }
    }
  }
  if (faceHit !== null && faceHit.instance === null) {
    const ctx = ctxOfFace(scene, faceHit.object, faceHit.face, null)
    if (ctx !== null) {
      const c = cornerFromListing(scene, ctx, faceHit.point, aperture)
      if (c !== null) {
        const ok = scene.draft_blendable_corner_at(ctx, c.point[0], c.point[1], c.point[2], aperture)
        if (ok !== undefined) {
          return {
            kind: 'corner',
            corner: { ...c, normal: planeNormalOfCtx(ctx), ctx, commit: { kind: 'face', object: faceHit.object } },
          }
        }
      }
    }
  }
  return null
}

/**
 * Resolve the pointer ray to a hover (§3.3). `null` when nothing useful is
 * under it. Never throws for a stale handle: a vanished object is a miss.
 */
export function resolveHover(scene: WasmScene, ray: Ray, opts: PickOptions): Hover {
  const fp = scene.pick_face(
    ray.origin[0], ray.origin[1], ray.origin[2],
    ray.direction[0], ray.direction[1], ray.direction[2],
  )
  let faceHit: { object: bigint; face: bigint; point: V3; instance: bigint | null; depth: number } | null = null
  if (fp !== undefined) {
    try {
      const depth = fp.depth()
      const dl = Math.hypot(ray.direction[0], ray.direction[1], ray.direction[2]) || 1
      faceHit = {
        object: fp.object(),
        face: fp.face(),
        instance: fp.instance() ?? null,
        depth,
        point: [
          ray.origin[0] + (ray.direction[0] / dl) * depth,
          ray.origin[1] + (ray.direction[1] / dl) * depth,
          ray.origin[2] + (ray.direction[2] / dl) * depth,
        ],
      }
    } finally {
      fp.free()
    }
  }
  const depth = faceHit?.depth ?? 5
  const aperture = apertureAt(opts, depth)

  // Drawn corners first: a vertex of a drawing is a precise target.
  const corner = pickCorner(scene, ray, faceHit, aperture)
  if (corner !== null) return corner
  if (faceHit === null) return { kind: 'none' }

  const frame = frameOf(scene, faceHit.instance)
  if (frame === null) return { kind: 'none' }

  // Edge: the nearest sharp edge of the face under the cursor.
  const ep = scene.pick_solid_edge(
    ray.origin[0], ray.origin[1], ray.origin[2],
    ray.direction[0], ray.direction[1], ray.direction[2],
    aperture,
  )
  const localPoint = toLocalPoint(frame, faceHit.point)
  let onSmoothSeam = false
  if (ep !== undefined) {
    let object: bigint, edge: bigint, smooth: boolean
    try {
      object = ep.object()
      edge = ep.edge()
      smooth = ep.smooth()
    } finally {
      ep.free()
    }
    // A seam inside one blend (two of its strips meeting at a mitered
    // corner) is part of that fillet, not an edge to round: the blend
    // hover below takes it, so a click there resizes the fillet.
    const bordering = smooth ? new BigUint64Array(0) : scene.edge_faces(object, edge)
    const seamBlend =
      bordering.length === 2 ? scene.blend_of_face(object, bordering[0]) : undefined
    const insideOneBlend =
      seamBlend !== undefined && scene.blend_of_face(object, bordering[1]) === seamBlend
    if (!smooth && !insideOneBlend) {
      const chain = readChain(scene.picked_edge(object, edge, !opts.noPropagate))
      if (chain.edges.length > 0) {
        return {
          kind: 'edge',
          object,
          frame,
          edge,
          hitFace: Array.from(bordering).includes(faceHit.face) ? faceHit.face : null,
          chain,
          point: localPoint,
        }
      }
    } else if (smooth && scene.blend_of_face(faceHit.object, faceHit.face) === undefined) {
      // A facet seam of a curved wall is not an edge to round, but the face
      // under it still is: fall through to the face pick, and report
      // 'smooth' only when that face has no sharp edge to offer.
      onSmoothSeam = true
    }
  }

  // Face: a blend, or the loop of edges nearest the cursor.
  const blendId = scene.blend_of_face(faceHit.object, faceHit.face)
  if (blendId !== undefined) {
    return {
      kind: 'blend',
      object: faceHit.object,
      frame,
      blendId,
      face: faceHit.face,
      faces: Array.from(scene.blend_faces(faceHit.object, blendId)),
      point: localPoint,
    }
  }
  // Two separate objects touching here: blending needs them united first.
  const touching = scene.objects_touching_point(faceHit.point[0], faceHit.point[1], faceHit.point[2], aperture)
  if (touching.length >= 2) {
    const names = Array.from(touching, (id) => scene.object_name(id))
    // Pieces of one group (a Union whose operands only touched leaves its
    // pieces grouped): the hint says so, since Union refuses grouped pieces.
    const parents = Array.from(touching, (id) => scene.node_parent(0, id))
    const parent = parents[0]
    const grouped = parent !== undefined && parent !== null && parents.every((g) => g === parent)
    return {
      kind: 'seam',
      names: names.every((n) => n !== undefined && n !== '') ? (names as string[]) : [],
      ...(grouped ? { group: scene.group_name(parent) ?? '' } : {}),
    }
  }
  const chain = readChain(
    scene.face_loop_edges(faceHit.object, faceHit.face, localPoint[0], localPoint[1], localPoint[2]),
  )
  if (chain.edges.length === 0) return onSmoothSeam ? { kind: 'smooth' } : { kind: 'none' }
  return { kind: 'face', object: faceHit.object, frame, face: faceHit.face, chain, point: localPoint }
}
