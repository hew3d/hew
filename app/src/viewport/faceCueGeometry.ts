/**
 * faceCueGeometry — the pure geometry the face cue (`FaceCueLayer`) is built
 * from: the triangles of one face out of a mesh's buffers, a planar loop
 * triangulated, and a point list mapped through a pose. No three.js scene,
 * no wasm: `SceneRenderer.faceTrianglePositions` and
 * `regionTrianglePositions` feed these the buffers it already holds.
 */
import * as THREE from 'three'

/** A face with more vertices than this in its range gets the outline only
 *  (the tint would copy that many triangles on every target change). */
export const FACE_CUE_MAX_VERTICES = 100_000

/**
 * The triangles of the face whose vertices are `[base, base + count)` — the
 * `face_mesh_range` — as flat `x,y,z` triples, three per triangle, copied out
 * of `positions` through `indices`. A triangle belongs to the face when its
 * first vertex is in the range (the rule `SceneRenderer.setFaceMask` uses).
 * `null` for a range past the buffer or one over `FACE_CUE_MAX_VERTICES`.
 */
export function trianglesInVertexRange(
  positions: ArrayLike<number>,
  indices: ArrayLike<number>,
  base: number,
  count: number,
): Float32Array | null {
  if (count > FACE_CUE_MAX_VERTICES) return null
  const vertexCount = Math.floor(positions.length / 3)
  if (base < 0 || count <= 0 || base + count > vertexCount) return null
  const end = base + count
  const out: number[] = []
  for (let i = 0; i + 3 <= indices.length; i += 3) {
    const a = indices[i]
    if (a < base || a >= end) continue
    for (let k = 0; k < 3; k++) {
      const v = indices[i + k]
      out.push(positions[v * 3], positions[v * 3 + 1], positions[v * 3 + 2])
    }
  }
  return out.length === 0 ? null : new Float32Array(out)
}

/** Map flat `x,y,z` points through a row-major 3×4 affine (an instance's
 *  pose), into a new array. */
export function mapThroughPose(points: ArrayLike<number>, pose: ArrayLike<number>): Float32Array {
  const n = Math.floor(points.length / 3)
  const out = new Float32Array(n * 3)
  for (let i = 0; i < n; i++) {
    const x = points[i * 3]
    const y = points[i * 3 + 1]
    const z = points[i * 3 + 2]
    out[i * 3] = pose[0] * x + pose[1] * y + pose[2] * z + pose[3]
    out[i * 3 + 1] = pose[4] * x + pose[5] * y + pose[6] * z + pose[7]
    out[i * 3 + 2] = pose[8] * x + pose[9] * y + pose[10] * z + pose[11]
  }
  return out
}

/**
 * Triangulate one planar closed loop (`boundary` flat `x,y,z`, n ≥ 3
 * vertices) into flat triangle positions at the loop's own 3D vertices.
 * Newell's method gives the polygon normal (the ground OR a face), the loop
 * is projected into that plane and ear-clipped, so a non-convex loop (an
 * L-shape, a freehand polyline) fills correctly where a fan from vertex 0
 * would not. `null` for a degenerate loop (under 3 vertices, or a
 * near-zero normal). The region fill meshes and the face cue share this.
 */
export function triangulatePlanarLoop(boundary: ArrayLike<number>): number[] | null {
  const n = Math.floor(boundary.length / 3)
  if (n < 3) return null
  const verts3: THREE.Vector3[] = []
  for (let i = 0; i < n; i++) {
    verts3.push(new THREE.Vector3(boundary[i * 3], boundary[i * 3 + 1], boundary[i * 3 + 2]))
  }
  const normal = new THREE.Vector3()
  for (let i = 0; i < n; i++) {
    const cur = verts3[i]
    const nxt = verts3[(i + 1) % n]
    normal.x += (cur.y - nxt.y) * (cur.z + nxt.z)
    normal.y += (cur.z - nxt.z) * (cur.x + nxt.x)
    normal.z += (cur.x - nxt.x) * (cur.y + nxt.y)
  }
  if (normal.lengthSq() < 1e-12) return null
  normal.normalize()

  const u = new THREE.Vector3()
  if (Math.abs(normal.z) < 0.9) u.set(0, 0, 1).cross(normal).normalize()
  else u.set(1, 0, 0).cross(normal).normalize()
  const v = new THREE.Vector3().crossVectors(normal, u).normalize()

  const pts2 = verts3.map((p) => new THREE.Vector2(p.dot(u), p.dot(v)))
  const tris = THREE.ShapeUtils.triangulateShape(pts2, [])

  const positions: number[] = []
  for (const tri of tris) {
    for (const idx of tri) {
      const p = verts3[idx]
      positions.push(p.x, p.y, p.z)
    }
  }
  return positions.length === 0 ? null : positions
}

/** Closed-loop segment pairs through flat `x,y,z` `loop` (the outline's
 *  `LineSegments2` input). */
export function loopSegments(loop: ArrayLike<number>): Float32Array {
  const n = Math.floor(loop.length / 3)
  const out = new Float32Array(n * 6)
  for (let i = 0; i < n; i++) {
    const j = (i + 1) % n
    out.set([loop[i * 3], loop[i * 3 + 1], loop[i * 3 + 2], loop[j * 3], loop[j * 3 + 1], loop[j * 3 + 2]], i * 6)
  }
  return out
}
