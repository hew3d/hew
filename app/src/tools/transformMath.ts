/**
 * transformMath — pure affine math for Move / Rotate / Scale tools.
 *
 * No three.js or DOM imports — fully testable in Node/vitest.
 *
 * Affine format: row-major 3×4, length 12.
 *   [m00 m01 m02 tx,  m10 m11 m12 ty,  m20 m21 m22 tz]
 * where the first 3 elements of each row are the linear part and the 4th
 * is the translation component.  This matches the `transform_object` WASM API.
 */

/** Row-major 3×4 affine matrix as a plain array (length 12). */
export type Affine = readonly [
  number, number, number, number, // row 0: m00 m01 m02 tx
  number, number, number, number, // row 1: m10 m11 m12 ty
  number, number, number, number, // row 2: m20 m21 m22 tz
]

/** The 3×4 identity transform (no-op). */
export const IDENTITY: Affine = [
  1, 0, 0, 0,
  0, 1, 0, 0,
  0, 0, 1, 0,
]

/**
 * Pure translation affine: identity linear part, (tx, ty, tz) translation.
 */
export function translationAffine(tx: number, ty: number, tz: number): Affine {
  return [
    1, 0, 0, tx,
    0, 1, 0, ty,
    0, 0, 1, tz,
  ]
}

/**
 * Rotation about world +Z by angle θ (radians), about the origin.
 *
 * The resulting matrix in row-major 3×4 form:
 *   [ cos(θ)  -sin(θ)  0  0 ]
 *   [ sin(θ)   cos(θ)  0  0 ]
 *   [  0        0      1  0 ]
 */
export function rotationZAffine(theta: number): Affine {
  const c = Math.cos(theta)
  const s = Math.sin(theta)
  return [
    c, -s, 0, 0,
    s,  c, 0, 0,
    0,  0, 1, 0,
  ]
}

/**
 * Uniform scale about the origin.
 *
 * Factor f must be > 0 (callers are responsible for guarding against ≤ 0).
 */
export function uniformScaleAffine(f: number): Affine {
  return [
    f, 0, 0, 0,
    0, f, 0, 0,
    0, 0, f, 0,
  ]
}

/**
 * Compose two 3×4 affine matrices: A then B → B(A(x)).
 *
 * Computes M = B * A (treating each as a 4×4 with an implicit [0 0 0 1]
 * bottom row).
 */
export function composeAffine(A: Affine, B: Affine): Affine {
  // A rows: [a00..a03], [a10..a13], [a20..a23], implicit [0,0,0,1]
  // B rows: [b00..b03], [b10..b13], [b20..b23], implicit [0,0,0,1]
  // Result M = B * A
  const [a00, a01, a02, a03,
         a10, a11, a12, a13,
         a20, a21, a22, a23] = A
  const [b00, b01, b02, b03,
         b10, b11, b12, b13,
         b20, b21, b22, b23] = B

  return [
    b00*a00 + b01*a10 + b02*a20,  b00*a01 + b01*a11 + b02*a21,  b00*a02 + b01*a12 + b02*a22,  b00*a03 + b01*a13 + b02*a23 + b03,
    b10*a00 + b11*a10 + b12*a20,  b10*a01 + b11*a11 + b12*a21,  b10*a02 + b11*a12 + b12*a22,  b10*a03 + b11*a13 + b12*a23 + b13,
    b20*a00 + b21*a10 + b22*a20,  b20*a01 + b21*a11 + b22*a21,  b20*a02 + b21*a12 + b22*a22,  b20*a03 + b21*a13 + b22*a23 + b23,
  ]
}

/**
 * Rotation about an arbitrary axis through the origin by angle θ (radians),
 * via Rodrigues' rotation formula. The axis (ax, ay, az) is normalized
 * internally; if it is ~zero length, returns IDENTITY (no-op).
 */
export function rotationAxisAffine(
  ax: number,
  ay: number,
  az: number,
  theta: number,
): Affine {
  const len = Math.sqrt(ax * ax + ay * ay + az * az)
  if (len < 1e-12) return IDENTITY

  const x = ax / len
  const y = ay / len
  const z = az / len
  const c = Math.cos(theta)
  const s = Math.sin(theta)
  const t = 1 - c

  return [
    c + x * x * t,        x * y * t - z * s,    x * z * t + y * s,    0,
    y * x * t + z * s,    c + y * y * t,        y * z * t - x * s,    0,
    z * x * t - y * s,    z * y * t + x * s,    c + z * z * t,        0,
  ]
}

/**
 * Build a rotation-about-pivot affine for an arbitrary axis:
 *   1. Translate pivot to origin: T(-pivot)
 *   2. Rotate by θ radians about the (normalized) axis: R(θ)
 *   3. Translate back: T(+pivot)
 *
 * Resulting transform: T(pivot) * R(θ) * T(-pivot)
 */
export function rotateAboutPivotAxis(
  pivotX: number,
  pivotY: number,
  pivotZ: number,
  axisX: number,
  axisY: number,
  axisZ: number,
  theta: number,
): Affine {
  const toPivot = translationAffine(-pivotX, -pivotY, -pivotZ)
  const rot = rotationAxisAffine(axisX, axisY, axisZ, theta)
  const fromPivot = translationAffine(pivotX, pivotY, pivotZ)
  // Apply in order: toPivot → rot → fromPivot
  return composeAffine(composeAffine(toPivot, rot), fromPivot)
}

/**
 * Build a rotation-about-pivot affine (world +Z, ground-plane spin).
 * Delegates to `rotateAboutPivotAxis` with axis (0, 0, 1).
 *
 * Resulting transform: T(pivot) * R_Z(θ) * T(-pivot)
 */
export function rotateAboutPivotZ(
  pivotX: number,
  pivotY: number,
  pivotZ: number,
  theta: number,
): Affine {
  return rotateAboutPivotAxis(pivotX, pivotY, pivotZ, 0, 0, 1, theta)
}

/**
 * Project a vector onto the plane perpendicular to a unit axis:
 * v - (v·a)a. The caller is responsible for passing a normalized axis
 * (ax, ay, az); the result is not renormalized.
 */
export function projectOntoPlane(
  vx: number,
  vy: number,
  vz: number,
  ax: number,
  ay: number,
  az: number,
): [number, number, number] {
  const d = vx * ax + vy * ay + vz * az
  return [vx - d * ax, vy - d * ay, vz - d * az]
}

/**
 * Normalize a 3-vector to unit length, or return null if it is ~zero
 * (length < 1e-9) — callers treat that as a degenerate direction rather than
 * risking a divide-by-zero / NaN axis.
 */
export function normalize3(
  v: readonly [number, number, number],
): [number, number, number] | null {
  const len = Math.sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
  if (len < 1e-9) return null
  return [v[0] / len, v[1] / len, v[2] / len]
}

/**
 * Build two unit vectors (u, v) spanning the plane ⊥ `normal`: u is a stable
 * in-plane vector (pick the world axis LEAST parallel to `normal`, then
 * orthogonalize against it), v = normal × u. `normal` must already be unit
 * length. Used to lay out disk/ring gizmo geometry in an arbitrary plane
 * (Protractor, Rotate).
 */
export function planeBasis(
  normal: readonly [number, number, number],
): { u: [number, number, number]; v: [number, number, number] } {
  const [nx, ny, nz] = normal
  // Pick whichever world axis has the smallest |dot| with normal (least parallel).
  const ax = Math.abs(nx), ay = Math.abs(ny), az = Math.abs(nz)
  let seed: [number, number, number]
  if (ax <= ay && ax <= az) seed = [1, 0, 0]
  else if (ay <= ax && ay <= az) seed = [0, 1, 0]
  else seed = [0, 0, 1]

  const dot = seed[0] * nx + seed[1] * ny + seed[2] * nz
  const raw: [number, number, number] = [
    seed[0] - dot * nx,
    seed[1] - dot * ny,
    seed[2] - dot * nz,
  ]
  const u = normalize3(raw) ?? [1, 0, 0]
  const v: [number, number, number] = [
    ny * u[2] - nz * u[1],
    nz * u[0] - nx * u[2],
    nx * u[1] - ny * u[0],
  ]
  return { u, v }
}

/**
 * Signed angle (radians) from vector f to vector t, measured in the plane
 * perpendicular to the unit axis a (right-hand rule about a).
 *
 * Both f and t are projected onto the plane ⊥ a before measuring. If either
 * projection is ~zero (degenerate — the vector lies along the axis), returns
 * 0 since no reference direction can be formed.
 *
 * Axis (ax, ay, az) is assumed to already be a unit vector (callers
 * typically pass a normalized faceAxis / world axis).
 */
export function signedAngleAboutAxis(
  ax: number,
  ay: number,
  az: number,
  fx: number,
  fy: number,
  fz: number,
  tx: number,
  ty: number,
  tz: number,
): number {
  const [fpx, fpy, fpz] = projectOntoPlane(fx, fy, fz, ax, ay, az)
  const [tpx, tpy, tpz] = projectOntoPlane(tx, ty, tz, ax, ay, az)

  const fLen = Math.sqrt(fpx * fpx + fpy * fpy + fpz * fpz)
  const tLen = Math.sqrt(tpx * tpx + tpy * tpy + tpz * tpz)
  if (fLen < 1e-9 || tLen < 1e-9) return 0

  const dot = fpx * tpx + fpy * tpy + fpz * tpz
  // cross(fProj, tProj) · a
  const cx = fpy * tpz - fpz * tpy
  const cy = fpz * tpx - fpx * tpz
  const cz = fpx * tpy - fpy * tpx
  const crossDotAxis = cx * ax + cy * ay + cz * az

  return Math.atan2(crossDotAxis, dot)
}

/**
 * Non-uniform scale about a world-space pivot: `diag(sx, sy, sz)` applied
 * about `pivot` in world axes —
 *   1. Translate pivot to origin: T(-pivot)
 *   2. Per-axis scale: diag(sx, sy, sz)
 *   3. Translate back: T(+pivot)
 *
 * A factor of 1 on an axis leaves it untouched, so a single-axis (face) grip
 * passes 1 for the two axes it doesn't drive, and a two-axis (edge) grip
 * passes 1 for the one it doesn't. Each factor must be > 0 (callers are
 * responsible for clamping — the kernel refuses a zero/negative factor as
 * singular/reflecting).
 */
export function nonUniformScaleAboutPivot(
  sx: number,
  sy: number,
  sz: number,
  pivot: readonly [number, number, number],
): Affine {
  const toPivot = translationAffine(-pivot[0], -pivot[1], -pivot[2])
  const scale: Affine = [
    sx, 0, 0, 0,
    0, sy, 0, 0,
    0, 0, sz, 0,
  ]
  const fromPivot = translationAffine(pivot[0], pivot[1], pivot[2])
  return composeAffine(composeAffine(toPivot, scale), fromPivot)
}

/**
 * Build a uniform-scale-about-center affine:
 *   1. Translate center to origin: T(-center)
 *   2. Uniform scale by f: S(f)
 *   3. Translate back: T(+center)
 *
 * f must be > 0 (guarded by the caller). The `sx === sy === sz`,
 * `pivot === center` special case of {@link nonUniformScaleAboutPivot}.
 */
export function scaleAboutCenter(
  cx: number,
  cy: number,
  cz: number,
  f: number,
): Affine {
  return nonUniformScaleAboutPivot(f, f, f, [cx, cy, cz])
}

/**
 * Snap an angle (radians) to the nearest multiple of `snapDeg` degrees.
 * Returns the snapped value in radians.
 *
 * e.g. snapAngleDeg(angle, 15) snaps to 0°, 15°, 30°, …
 */
export function snapAngleDeg(theta: number, snapDeg: number): number {
  const snapRad = (snapDeg * Math.PI) / 180
  return Math.round(theta / snapRad) * snapRad
}

/**
 * Compute the angle (radians, atan2) from a pivot to a point in the XY plane.
 */
export function angleFromPivot(
  pivotX: number,
  pivotY: number,
  pointX: number,
  pointY: number,
): number {
  return Math.atan2(pointY - pivotY, pointX - pivotX)
}

/**
 * Compute the bounding-box center of a mesh's position buffer.
 *
 * `positions` is a flat Float32Array of [x,y,z] triples (the same format
 * returned by MeshJs.positions()).  Returns [cx, cy, cz].
 */
export function meshBoundingBoxCenter(positions: Float32Array): [number, number, number] {
  if (positions.length < 3) return [0, 0, 0]
  let minX = positions[0], maxX = positions[0]
  let minY = positions[1], maxY = positions[1]
  let minZ = positions[2], maxZ = positions[2]
  for (let i = 3; i < positions.length; i += 3) {
    const x = positions[i], y = positions[i + 1], z = positions[i + 2]
    if (x < minX) minX = x; if (x > maxX) maxX = x
    if (y < minY) minY = y; if (y > maxY) maxY = y
    if (z < minZ) minZ = z; if (z > maxZ) maxZ = z
  }
  return [(minX + maxX) / 2, (minY + maxY) / 2, (minZ + maxZ) / 2]
}

/**
 * Convert a 12-element Affine to a Float64Array for passing to transform_object.
 */
export function affineToFloat64(a: Affine): Float64Array {
  return new Float64Array(a)
}
