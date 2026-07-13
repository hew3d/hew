/**
 * Pure geometry helpers — no WebGL, no three.js, testable in Node/vitest.
 *
 * Used by RectangleTool (corner math) and PushPullTool (drag-distance
 * projection onto a face/region normal).
 */

/** 3-element number tuple for conciseness */
export type V3 = [number, number, number]

/**
 * Given two diagonally opposite corners of a rectangle on the ground plane
 * (Z=0), returns the four corners in order: [a, (ax,by,0), b, (bx,ay,0)].
 *
 * The winding is counter-clockwise when viewed from above (+Z).
 */
export function rectangleCorners(
  a: [number, number],
  b: [number, number],
): [[number, number, number], [number, number, number], [number, number, number], [number, number, number]] {
  const [ax, ay] = a
  const [bx, by] = b
  return [
    [ax, ay, 0],
    [bx, ay, 0],
    [bx, by, 0],
    [ax, by, 0],
  ]
}

/**
 * Project a cursor ray onto a line defined by an anchor point and direction,
 * returning the signed scalar distance from anchor along that direction.
 *
 * This is the standard "closest point on a line to a ray" computation used
 * by PushPullTool to convert mouse position to extrusion distance.
 *
 * Both `direction` and `rayDir` are assumed to be normalised (unit vectors).
 * If the ray and axis are nearly parallel, returns 0 (degenerate case).
 *
 * The signed distance is measured along `direction`; positive means in the
 * direction vector's direction, negative means opposite.
 */
export function projectRayOntoAxis(
  rayOrigin: V3,
  rayDir: V3,
  anchor: V3,
  direction: V3,
): number {
  // We want to find t such that the point (anchor + t * direction) is
  // "closest" to the ray. We minimise distance between:
  //   P1(t) = anchor + t * direction
  //   P2(s) = rayOrigin + s * rayDir
  //
  // The closest-point system: set up the 2x2 linear system for t and s.
  // Let w = anchor - rayOrigin
  // We minimise |P1(t) - P2(s)|^2
  //
  // Dot equations:
  //   (P1 - P2) · direction = 0
  //   (P1 - P2) · rayDir    = 0  (but we care only about t)
  //
  // Standard closest-line solution:
  //   d1 = direction, d2 = rayDir
  //   b = w · d1
  //   e = w · d2
  //   c = d1 · d1 = 1 (normalised)
  //   f = d2 · d2 = 1 (normalised)
  //   d = d1 · d2
  //
  //   denom = c*f - d*d = 1 - d^2
  //   t = (b*f - e*d) / denom = (b - e*d) / denom

  const [ox, oy, oz] = rayOrigin
  const [dx, dy, dz] = rayDir
  const [ax, ay, az] = anchor
  const [nx, ny, nz] = direction

  // w points from anchor to ray origin (P(0) to Q(0) with the sign that
  // gives t positive when ray is "above" the anchor along the axis).
  const wx = ox - ax
  const wy = oy - ay
  const wz = oz - az

  const b = wx * nx + wy * ny + wz * nz
  const e = wx * dx + wy * dy + wz * dz
  const d = nx * dx + ny * dy + nz * dz

  const denom = 1.0 - d * d
  if (Math.abs(denom) < 1e-10) return 0 // ray nearly parallel to axis

  return (b - e * d) / denom
}

/**
 * Intersect a ray with an arbitrary plane defined by a point and unit normal.
 * Returns the intersection point, or null if the ray is nearly parallel to
 * the plane (|dot(dir, normal)| < 1e-10) or the intersection is behind the
 * ray origin (t < 0).
 *
 * `rayDir` need not be normalized; `normal` is assumed to be a unit vector.
 */
export function rayPlaneIntersect(
  rayOrigin: V3,
  rayDir: V3,
  planePoint: V3,
  normal: V3,
): V3 | null {
  const denom = rayDir[0] * normal[0] + rayDir[1] * normal[1] + rayDir[2] * normal[2]
  if (Math.abs(denom) < 1e-10) return null
  const wx = planePoint[0] - rayOrigin[0]
  const wy = planePoint[1] - rayOrigin[1]
  const wz = planePoint[2] - rayOrigin[2]
  const t = (wx * normal[0] + wy * normal[1] + wz * normal[2]) / denom
  if (t < 0) return null
  return [
    rayOrigin[0] + t * rayDir[0],
    rayOrigin[1] + t * rayDir[1],
    rayOrigin[2] + t * rayDir[2],
  ]
}

/**
 * Build an orthonormal in-plane basis (u, v) for the plane with unit normal
 * `normal`, such that (u, v, normal) is right-handed — i.e. cross(u, v) ==
 * normal. Used to project points onto an arbitrary face plane.
 *
 * Returns null only in the degenerate case where `normal` is not a finite
 * unit-ish vector (length < 1e-12).
 */
export function facePlaneBasis(normal: V3): { u: V3; v: V3 } | null {
  const [nx, ny, nz] = normal

  // Choose a reference axis not parallel to the normal.
  // If |normal.x| dominates, use [0,1,0]; otherwise use [1,0,0].
  const refX = Math.abs(nx) > Math.abs(ny) ? 0 : 1
  const refY = Math.abs(nx) > Math.abs(ny) ? 1 : 0
  const refZ = 0

  // u = normalize(cross(normal, ref))
  const cx = ny * refZ - nz * refY
  const cy = nz * refX - nx * refZ
  const cz = nx * refY - ny * refX
  const len = Math.sqrt(cx * cx + cy * cy + cz * cz)
  if (len < 1e-12) return null
  const u: V3 = [cx / len, cy / len, cz / len]

  // v = cross(normal, u)  — already unit since normal and u are both unit and orthogonal
  const v: V3 = [
    ny * u[2] - nz * u[1],
    nz * u[0] - nx * u[2],
    nx * u[1] - ny * u[0],
  ]

  return { u, v }
}

/**
 * Build a rectangle's 4 world-space corners lying on an arbitrary plane,
 * given two diagonal corner points and the face's unit outward normal.
 *
 * Algorithm:
 *   1. Build an orthonormal in-plane basis (u, v) from the normal (see
 *      `facePlaneBasis`).
 *   2. Project (cursor - anchor) onto u and v to get signed extents du, dv.
 *   3. Return the 4 CCW corners (as seen from the +normal side):
 *      anchor, anchor+u·du, anchor+u·du+v·dv, anchor+v·dv
 *
 * Returns null when either extent is below 1e-7 (degenerate rectangle).
 *
 * Winding note: the result is counter-clockwise when viewed from the +normal
 * side, matching the ground-plane CCW convention used by `rectangleCorners`.
 */
export function faceRectangleCorners(
  anchor: V3,
  cursor: V3,
  normal: V3,
): [V3, V3, V3, V3] | null {
  const basis = facePlaneBasis(normal)
  if (basis === null) return null
  const { u, v } = basis

  // Signed extents along u and v
  const dx = cursor[0] - anchor[0]
  const dy = cursor[1] - anchor[1]
  const dz = cursor[2] - anchor[2]
  const du = dx * u[0] + dy * u[1] + dz * u[2]
  const dv = dx * v[0] + dy * v[1] + dz * v[2]

  if (Math.abs(du) < 1e-7 || Math.abs(dv) < 1e-7) return null

  // Four corners (positions are always the same set regardless of drag direction):
  // A=anchor, B=anchor+u*du, C=anchor+u*du+v*dv, D=anchor+v*dv
  const a: V3 = [...anchor]
  const b: V3 = [anchor[0] + u[0] * du, anchor[1] + u[1] * du, anchor[2] + u[2] * du]
  const c: V3 = [b[0] + v[0] * dv, b[1] + v[1] * dv, b[2] + v[2] * dv]
  const d: V3 = [anchor[0] + v[0] * dv, anchor[1] + v[1] * dv, anchor[2] + v[2] * dv]

  // When du and dv have opposite signs the traversal A→B→C→D is clockwise
  // from the +normal side. Reverse the winding (swap B and D) so the result
  // is always CCW from +normal, matching the promise in the doc comment.
  if (du * dv < 0) {
    return [a, d, c, b]
  }
  return [a, b, c, d]
}

/**
 * Build a faceted regular N-gon's vertices on the ground plane (Z=0), given
 * a center and a point on the rim (the first click's radius/start-angle).
 *
 * Vertex 0 is exactly `rim` (so the preview/commit always passes through the
 * cursor); the remaining N-1 vertices are spaced at equal angular steps
 * counter-clockwise (viewed from +Z) around `center`, at the same radius.
 *
 * Returns `[]` if the radius (distance from `center` to `rim`) is below
 * 1e-7 (degenerate — caller should treat as "no circle").
 */
export function circlePolygonGround(
  center: [number, number],
  rim: [number, number],
  segments: number,
): V3[] {
  const [cx, cy] = center
  const dx = rim[0] - cx
  const dy = rim[1] - cy
  const radius = Math.hypot(dx, dy)
  if (radius < 1e-7) return []

  const startAngle = Math.atan2(dy, dx)
  const verts: V3[] = []
  for (let i = 0; i < segments; i++) {
    const angle = startAngle + (2 * Math.PI * i) / segments
    verts.push([cx + radius * Math.cos(angle), cy + radius * Math.sin(angle), 0])
  }
  return verts
}

/**
 * Build a faceted regular N-gon's vertices on an arbitrary plane, given a
 * center, a point on the rim (defining radius + start angle), and the
 * plane's unit normal.
 *
 * Mirrors `circlePolygonGround` but projects into the in-plane basis (u, v)
 * built by `facePlaneBasis(normal)` — vertex 0 is the rim point's in-plane
 * projection, and winding is counter-clockwise viewed from the +normal side
 * (u→v is a quarter-turn CCW about +normal, matching `faceRectangleCorners`'s
 * convention).
 *
 * Returns `null` if the basis is degenerate (see `facePlaneBasis`) or the
 * radius (distance from `center` to the rim's in-plane projection) is below
 * 1e-7.
 */
export function circlePolygonFace(
  center: V3,
  rim: V3,
  normal: V3,
  segments: number,
): V3[] | null {
  const basis = facePlaneBasis(normal)
  if (basis === null) return null
  const { u, v } = basis

  const dx = rim[0] - center[0]
  const dy = rim[1] - center[1]
  const dz = rim[2] - center[2]
  const du = dx * u[0] + dy * u[1] + dz * u[2]
  const dv = dx * v[0] + dy * v[1] + dz * v[2]
  const radius = Math.hypot(du, dv)
  if (radius < 1e-7) return null

  const startAngle = Math.atan2(dv, du)
  const verts: V3[] = []
  for (let i = 0; i < segments; i++) {
    const angle = startAngle + (2 * Math.PI * i) / segments
    const r_cos = radius * Math.cos(angle)
    const r_sin = radius * Math.sin(angle)
    verts.push([
      center[0] + u[0] * r_cos + v[0] * r_sin,
      center[1] + u[1] * r_cos + v[1] * r_sin,
      center[2] + u[2] * r_cos + v[2] * r_sin,
    ])
  }
  return verts
}

/**
 * Compute the absolute area of a 2D polygon given as a flat xyz vertex array
 * [x0, y0, z0, x1, y1, z1, ...] (z values ignored — polygon is treated as XY).
 *
 * Uses the shoelace formula. Returns the absolute (unsigned) area.
 */
export function polygonAreaXY(vertices: Float32Array): number {
  const n = Math.floor(vertices.length / 3)
  if (n < 3) return 0
  let sum = 0
  for (let i = 0; i < n; i++) {
    const j = (i + 1) % n
    const xi = vertices[i * 3]
    const yi = vertices[i * 3 + 1]
    const xj = vertices[j * 3]
    const yj = vertices[j * 3 + 1]
    sum += xi * yj - xj * yi
  }
  return Math.abs(sum) * 0.5
}

/**
 * Point-in-polygon test using the ray-casting algorithm.
 *
 * Tests whether (px, py) is inside the 2D polygon given by a flat array of
 * vertices [x0, y0, z0, x1, y1, z1, ...] (z values ignored — polygon is
 * treated as XY). The polygon is assumed to be simple (non-self-intersecting)
 * and may be convex or concave.
 *
 * Returns true if the point is strictly inside or on the boundary (tolerance
 * 1e-10).
 */
export function pointInPolygonXY(
  px: number,
  py: number,
  vertices: Float32Array,
): boolean {
  const n = Math.floor(vertices.length / 3)
  if (n < 3) return false

  let inside = false
  let j = n - 1

  for (let i = 0; i < n; i++) {
    const xi = vertices[i * 3]
    const yi = vertices[i * 3 + 1]
    const xj = vertices[j * 3]
    const yj = vertices[j * 3 + 1]

    // Ray-casting: count crossings of horizontal ray from (px, py) to +∞ x
    const intersects =
      yi > py !== yj > py &&
      px < ((xj - xi) * (py - yi)) / (yj - yi) + xi

    if (intersects) inside = !inside
    j = i
  }

  return inside
}
