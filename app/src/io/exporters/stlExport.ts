/**
 * Binary STL export.
 *
 * Sources geometry from the KERNEL, not the viewport: every object's export
 * tessellation comes from `Scene.object_export_triangles`, which re-facets
 * pristine stamped cylinder walls from their analytic definitions at the
 * requested resolution (the true-curves design stage 6 — "true curves
 * for STL") and honestly falls back to stored facets where a wall is no
 * longer fully analytic. The kernel guarantees the soup is manifold at any
 * resolution. Instances are flattened by applying their poses here; STL has
 * no object structure, so the whole scene concatenates into one soup.
 *
 * Two deliberate differences from the GLB path:
 *  - **No Y-up rotation.** STL consumers (slicers) expect Z-up, which is
 *    Hew's native world orientation.
 *  - **Millimeter scale.** STL is unitless; the universal slicer convention
 *    is millimeters, so kernel meters are multiplied by 1000.
 *
 * Binary STL layout (all little-endian):
 *   80-byte header ("Hew <version> binary STL, millimeters", zero-padded)
 *   u32 triangle count
 *   per triangle (50 bytes): 3×f32 normal, 3×(3×f32) vertices, u16
 *   attribute-byte-count = 0
 *
 * Triangle normals are computed from vertex winding (right-hand rule). The
 * kernel emits counter-clockwise-from-outside triangles, so the winding is
 * used as-is; an instance whose pose has negative determinant (mirrored
 * placement) gets its winding flipped so normals still point outward.
 * Zero-area triangles are skipped and counted — never repaired (rule 4 in
 * spirit).
 */

/** Size of the fixed binary STL header. */
export const STL_HEADER_BYTES = 80
/** Bytes per encoded triangle: 12×f32 + u16 attribute count. */
export const STL_TRIANGLE_BYTES = 50
/** Kernel lengths are f64 meters; STL ships at millimeter scale. */
const METERS_TO_MM = 1000
/**
 * Degeneracy threshold on the squared length of the winding cross product,
 * in mm⁴ (cross length = 2×triangle area in mm²). Triangles at or below this
 * have no meaningful area/normal and are skipped, not repaired.
 */
const DEGENERATE_CROSS_LENGTH_SQ_MM4 = 1e-12

/** Result of encoding a triangle soup as binary STL. */
export interface StlBuildResult {
  bytes: Uint8Array
  /** Triangles actually written (after degenerate skipping). */
  triangleCount: number
  /** Zero-area triangles dropped from the output. */
  skippedDegenerate: number
}

/** Injected in tests; real builds get the Vite-defined app version. */
declare const __HEW_VERSION__: string | undefined

function hewVersion(): string {
  return typeof __HEW_VERSION__ !== 'undefined' ? __HEW_VERSION__ : '0.0.0'
}

/**
 * Pure binary-STL writer. `triangles` is a flat triangle soup in world-space
 * METERS, 9 numbers per triangle (v0 v1 v2, counter-clockwise = outward);
 * millimeter scaling happens here. Exported separately from the scene walk
 * so the byte layout is unit-testable without three.js scene setup.
 */
export function writeBinaryStl(
  triangles: ArrayLike<number>,
  version: string = hewVersion(),
): StlBuildResult {
  const inCount = Math.floor(triangles.length / 9)

  // First pass: measure each triangle in mm; remember which ones survive.
  const keep: number[] = []
  const normals: number[] = [] // 3 per kept triangle
  for (let t = 0; t < inCount; t++) {
    const o = t * 9
    const ax = triangles[o] * METERS_TO_MM
    const ay = triangles[o + 1] * METERS_TO_MM
    const az = triangles[o + 2] * METERS_TO_MM
    const bx = triangles[o + 3] * METERS_TO_MM
    const by = triangles[o + 4] * METERS_TO_MM
    const bz = triangles[o + 5] * METERS_TO_MM
    const cx = triangles[o + 6] * METERS_TO_MM
    const cy = triangles[o + 7] * METERS_TO_MM
    const cz = triangles[o + 8] * METERS_TO_MM
    // Right-hand rule over the winding: n = (b−a) × (c−a).
    const ux = bx - ax, uy = by - ay, uz = bz - az
    const vx = cx - ax, vy = cy - ay, vz = cz - az
    const nx = uy * vz - uz * vy
    const ny = uz * vx - ux * vz
    const nz = ux * vy - uy * vx
    const lenSq = nx * nx + ny * ny + nz * nz
    if (lenSq <= DEGENERATE_CROSS_LENGTH_SQ_MM4) continue
    const invLen = 1 / Math.sqrt(lenSq)
    keep.push(t)
    normals.push(nx * invLen, ny * invLen, nz * invLen)
  }

  const outCount = keep.length
  const bytes = new Uint8Array(STL_HEADER_BYTES + 4 + outCount * STL_TRIANGLE_BYTES)
  const view = new DataView(bytes.buffer)

  // Header: informative ASCII, zero-padded to 80 bytes. Never starts with
  // "solid" (the ASCII-STL magic) so sniffing parsers can't misdetect it.
  const header = `Hew ${version} binary STL, millimeters`
  for (let i = 0; i < Math.min(header.length, STL_HEADER_BYTES); i++) {
    bytes[i] = header.charCodeAt(i) & 0x7f
  }

  view.setUint32(STL_HEADER_BYTES, outCount, true)

  let off = STL_HEADER_BYTES + 4
  for (let k = 0; k < outCount; k++) {
    const t = keep[k]
    view.setFloat32(off, normals[k * 3], true)
    view.setFloat32(off + 4, normals[k * 3 + 1], true)
    view.setFloat32(off + 8, normals[k * 3 + 2], true)
    off += 12
    const o = t * 9
    for (let v = 0; v < 9; v++) {
      view.setFloat32(off, triangles[o + v] * METERS_TO_MM, true)
      off += 4
    }
    view.setUint16(off, 0, true) // attribute byte count
    off += 2
  }

  return { bytes, triangleCount: outCount, skippedDegenerate: inCount - outCount }
}

/**
 * The slice of the wasm `Scene` surface the kernel-sourced STL collector
 * needs — structural, so tests can pass a plain mock.
 */
export interface StlExportScene extends SolidQueryScene {
  /** Row-major 3×4 affine pose, or undefined for a stale handle. */
  instance_pose(instance: bigint): Float64Array | undefined
  /**
   * Kernel export tessellation for one object: flat triangle soup, 9 floats
   * per triangle, object-local meters, CCW from outside.
   * `segmentsPerTurn === 0` = stored facets.
   */
  object_export_triangles(object: bigint, segmentsPerTurn: number): Float32Array
}

/**
 * Collect the whole scene (top-level objects + placed instances) as one
 * world-space triangle soup in meters, sourced from the kernel's export
 * tessellation at `segmentsPerTurn`. An instance pose with negative
 * determinant (mirrored placement) flips triangle winding so the right-hand
 * rule still yields outward normals.
 */
export function collectKernelTriangles(scene: StlExportScene, segmentsPerTurn: number): number[] {
  const out: number[] = []

  for (const id of scene.object_ids()) {
    // Top-level objects have their transforms baked: local == world.
    const soup = scene.object_export_triangles(id, segmentsPerTurn)
    for (let i = 0; i < soup.length; i++) out.push(soup[i])
  }

  for (const instanceId of scene.instance_ids()) {
    const def = scene.instance_def(instanceId)
    const pose = scene.instance_pose(instanceId)
    if (def === undefined || pose === undefined || pose.length !== 12) continue
    const [m00, m01, m02, tx, m10, m11, m12, ty, m20, m21, m22, tz] = pose
    const det =
      m00 * (m11 * m22 - m12 * m21) -
      m01 * (m10 * m22 - m12 * m20) +
      m02 * (m10 * m21 - m11 * m20)
    const flip = det < 0
    for (const memberId of scene.component_member_objects(def)) {
      const soup = scene.object_export_triangles(memberId, segmentsPerTurn)
      for (let t = 0; t + 9 <= soup.length; t += 9) {
        // Vertex order 0,1,2 — or 0,2,1 under a mirrored pose.
        const order = flip ? [0, 2, 1] : [0, 1, 2]
        for (const v of order) {
          const x = soup[t + v * 3]
          const y = soup[t + v * 3 + 1]
          const z = soup[t + v * 3 + 2]
          out.push(
            m00 * x + m01 * y + m02 * z + tx,
            m10 * x + m11 * y + m12 * z + ty,
            m20 * x + m21 * y + m22 * z + tz,
          )
        }
      }
    }
  }

  return out
}

/**
 * Serialize the current solid geometry (objects + instances, faces only) to
 * a binary STL buffer at millimeter scale, Z-up, with cylinder walls
 * re-faceted at `segmentsPerTurn` (0 = stored facets). Returns `null` when
 * there is nothing solid to export.
 */
export function exportSceneToStl(
  scene: StlExportScene,
  segmentsPerTurn: number,
): StlBuildResult | null {
  if (scene.object_ids().length === 0 && scene.instance_ids().length === 0) return null
  return writeBinaryStl(collectKernelTriangles(scene, segmentsPerTurn))
}

// ---------------------------------------------------------------------------
// Solid gating — the product point of STL export: warn before writing
// a file containing any non-watertight object. Query-only; never repairs.
// ---------------------------------------------------------------------------

/**
 * The minimal slice of the wasm `Scene` surface the gating query needs —
 * structural, so tests can pass a plain mock.
 */
export interface SolidQueryScene {
  object_ids(): BigUint64Array
  instance_ids(): BigUint64Array
  instance_def(instance: bigint): bigint | undefined
  component_member_objects(component: bigint): BigUint64Array
  object_solid(id: bigint): boolean
  object_name(object: bigint): string | undefined
}

/** One non-watertight object that would be included in the STL. */
export interface NonSolidObject {
  id: bigint
  name: string
}

/**
 * Every object the STL export would include that is NOT a watertight solid:
 * the top-level objects plus each placed instance's definition members
 * (exactly the set `buildExportScene` renders), deduplicated.
 */
export function collectNonSolidObjects(scene: SolidQueryScene): NonSolidObject[] {
  const seen = new Set<bigint>()
  const out: NonSolidObject[] = []

  const check = (id: bigint) => {
    if (seen.has(id)) return
    seen.add(id)
    if (scene.object_solid(id)) return
    out.push({ id, name: scene.object_name(id) ?? `Object ${id}` })
  }

  for (const id of scene.object_ids()) check(id)
  for (const instanceId of scene.instance_ids()) {
    const def = scene.instance_def(instanceId)
    if (def === undefined) continue
    for (const memberId of scene.component_member_objects(def)) check(memberId)
  }

  return out
}
