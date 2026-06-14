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
 * Parse a "CODE: message" thrown error string, returning the code prefix.
 * Returns null if the format doesn't match.
 */
export function parseKernelErrorCode(err: unknown): string | null {
  const msg = err instanceof Error ? err.message : String(err)
  const match = /^([A-Za-z][A-Za-z0-9_]*):\s/.exec(msg)
  return match ? match[1] : null
}

/**
 * Human-readable descriptions for known kernel error codes.
 * Falls back to the raw code when unknown.
 */
export function kernelErrorMessage(code: string, rawMsg: string): string {
  const descriptions: Record<string, string> = {
    WouldVanish: "Can't remove all material — object would vanish",
    NonManifoldResult: 'Operation would create non-manifold geometry',
    ObjectNotSolid: 'Object is not a solid',
    PathNotSimple: 'Path must be a simple (non-self-intersecting) polygon',
    NothingToUndo: 'Nothing to undo',
    NothingToRedo: 'Nothing to redo',
    UnknownObject: 'Unknown object handle (stale reference)',
  }
  return descriptions[code] ?? `${code}: ${rawMsg}`
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
