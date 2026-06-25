/**
 * Shared helpers for transform-tool preview cloning and cleanup.
 *
 * `buildPreviewClone` deep-clones the rendered mesh for a given objectId and
 * gives the clone its OWN BufferGeometry instances (via geometry.clone()) so
 * that `clearPreview`'s geometry.dispose() calls cannot corrupt the live
 * scene object's shared geometry.
 *
 * `buildMultiPreviewClone` does the same for a set of leaf-object ids (used
 * when a group is being transformed — all leaf meshes move together).
 */

import * as THREE from 'three'

/**
 * Clone a mesh's material(s) and make the clone(s) semi-transparent. Face
 * meshes are multi-material (one entry per material group), so the
 * material may be an array; edge meshes are single-material. Returns the same
 * shape (array or single) so it can be assigned straight back to `.material`.
 */
function fadePreviewMaterial(
  material: THREE.Material | THREE.Material[],
): THREE.Material | THREE.Material[] {
  const fade = (m: THREE.Material): THREE.Material => {
    const c = m.clone()
    c.opacity = 0.5
    c.transparent = true
    c.depthWrite = false
    return c
  }
  return Array.isArray(material) ? material.map(fade) : fade(material)
}

/**
 * Clone the mesh sub-group for one object and make it semi-transparent.
 * Returns null if the source group is not found.
 */
function cloneObjectMesh(
  objectsGroup: THREE.Group,
  objectId: bigint,
): THREE.Object3D | null {
  const name = `Object_${objectId}`
  let sourceGroup: THREE.Object3D | undefined
  objectsGroup.traverse((child) => {
    if (child.name === name) sourceGroup = child
  })
  if (sourceGroup === undefined) return null

  const clone = sourceGroup.clone(true)
  clone.traverse((child) => {
    if (child instanceof THREE.Mesh) {
      // Clone geometry so dispose() on the preview does not corrupt the live object
      child.geometry = child.geometry.clone()
      child.material = fadePreviewMaterial(child.material)
    }
    if (child instanceof THREE.LineSegments) {
      // Clone geometry so dispose() on the preview does not corrupt the live object
      child.geometry = child.geometry.clone()
      child.material = fadePreviewMaterial(child.material)
    }
  })
  return clone
}

/**
 * Build a semi-transparent THREE.js clone of the object's rendered mesh for
 * use as a drag preview.  Returns null if the source group is not found.
 *
 * The clone owns its own BufferGeometry (cloned, not shared) so that
 * clearPreview() can safely dispose it without affecting the live object.
 */
export function buildPreviewClone(
  objectsGroup: THREE.Group | null,
  objectId: bigint,
): THREE.Object3D | null {
  if (objectsGroup === null) return null
  return cloneObjectMesh(objectsGroup, objectId)
}

/**
 * Build a semi-transparent clone of an instance's THREE.Group for use as a
 * drag preview. Returns null if the source group is not found.
 */
export function buildInstancePreviewClone(
  instanceGroup: THREE.Group | null,
): THREE.Object3D | null {
  if (instanceGroup === null) return null
  const clone = instanceGroup.clone(true)
  // Reset the clone's matrix so the preview is in world-space identity —
  // the tool will translate it directly.
  clone.matrixAutoUpdate = true
  clone.matrix.identity()
  clone.matrixWorldNeedsUpdate = true
  clone.traverse((child) => {
    if (child instanceof THREE.Mesh) {
      child.geometry = child.geometry.clone()
      child.material = fadePreviewMaterial(child.material)
    }
    if (child instanceof THREE.LineSegments) {
      child.geometry = child.geometry.clone()
      child.material = fadePreviewMaterial(child.material)
    }
  })
  return clone
}

/** Line color for a sketch drag preview — matches the live sketch line color
 * (`SKETCH_LINE_COLOR` in SceneRenderer) so the ghost reads as "this sketch". */
const SKETCH_PREVIEW_LINE_COLOR = 0x2266cc

/**
 * Build a semi-transparent THREE.LineSegments ghost from a sketch's
 * world-space line positions (as returned by `wasmScene.sketch_lines`).
 * Returns null if the sketch currently has no lines.
 */
export function buildSketchPreviewClone(
  linePositions: Float32Array | number[],
): THREE.Object3D | null {
  if (linePositions.length === 0) return null
  const positions = linePositions instanceof Float32Array
    ? linePositions
    : new Float32Array(linePositions)
  const geo = new THREE.BufferGeometry()
  geo.setAttribute('position', new THREE.BufferAttribute(positions, 3))
  const mat = fadePreviewMaterial(
    new THREE.LineBasicMaterial({ color: SKETCH_PREVIEW_LINE_COLOR, linewidth: 2 }),
  ) as THREE.LineBasicMaterial
  return new THREE.LineSegments(geo, mat)
}

/**
 * Build a combined semi-transparent preview for a set of leaf object ids
 * (used when transforming a group — all leaves must move together as one unit).
 * Returns a THREE.Group containing all found clones, or null if none found.
 */
export function buildMultiPreviewClone(
  objectsGroup: THREE.Group | null,
  leafIds: bigint[],
): THREE.Group | null {
  if (objectsGroup === null || leafIds.length === 0) return null

  const container = new THREE.Group()
  container.name = 'MultiPreview'
  let found = 0
  for (const id of leafIds) {
    const clone = cloneObjectMesh(objectsGroup, id)
    if (clone !== null) {
      container.add(clone)
      found++
    }
  }
  if (found === 0) return null
  return container
}

/** Ghost tint for an additive push/pull drag (material being added) — matches
 * the push/pull arrow's existing accent color. */
const SWEPT_PRISM_ADD_COLOR = 0xee8800
/** Ghost tint for a subtractive drag (material being removed / cut through) —
 * a reddish warning tone so the direction of the operation reads at a glance. */
const SWEPT_PRISM_CUT_COLOR = 0xdd4422

/**
 * Triangulate a closed 3D polygon loop (no duplicate closing vertex) by
 * projecting onto the plane perpendicular to `normal` and delegating to
 * `THREE.ShapeUtils.triangulateShape`. The axis with the largest |normal|
 * component is dropped, which keeps the projection non-degenerate for any
 * mostly-planar input. Returns vertex-index triangles into `points`, or an
 * empty array if triangulation fails or the loop is degenerate.
 */
function triangulatePolygon3D(
  points: THREE.Vector3[],
  normal: THREE.Vector3,
): number[][] {
  if (points.length < 3) return []

  // Drop the axis with the largest |normal| component so the remaining two
  // axes give a non-degenerate 2D projection of the (mostly-planar) loop.
  const ax = Math.abs(normal.x)
  const ay = Math.abs(normal.y)
  const az = Math.abs(normal.z)
  const project: (p: THREE.Vector3) => THREE.Vector2 =
    ax >= ay && ax >= az
      ? (p) => new THREE.Vector2(p.y, p.z)
      : ay >= az
        ? (p) => new THREE.Vector2(p.x, p.z)
        : (p) => new THREE.Vector2(p.x, p.y)

  const contour2D = points.map(project)
  try {
    return THREE.ShapeUtils.triangulateShape(contour2D, [])
  } catch {
    return []
  }
}

/**
 * Build a translucent swept-prism ghost: a base cap at `boundary`, a top cap
 * at `boundary` offset by `normal * distance`, and quad side walls connecting
 * the two loops edge-by-edge. Used by PushPullTool to preview the solid that
 * push/pull will actually produce, instead of a bare arrow.
 *
 * `boundary` is an ordered closed loop of world-space points (no duplicate
 * closing vertex); it may be non-convex or many-sided. Caps are triangulated
 * via `triangulatePolygon3D` so non-convex loops (L-shapes) and facet circles
 * (N-gons) both work. Returns null for degenerate input (fewer than 3 boundary
 * vertices, or a near-zero distance) rather than throwing.
 *
 * The returned Object3D is always built from THREE.Mesh instances so the
 * shared `clearPreview` (which disposes Mesh/LineSegments geometries and
 * materials) fully cleans it up between drag frames.
 */
export function buildSweptPrismPreview(
  boundary: Float32Array | number[],
  normal: [number, number, number],
  distance: number,
): THREE.Object3D | null {
  if (Math.abs(distance) < 1e-6) return null

  const vertexCount = Math.floor(boundary.length / 3)
  if (vertexCount < 3) return null

  const n = new THREE.Vector3(normal[0], normal[1], normal[2])
  const base: THREE.Vector3[] = []
  for (let i = 0; i < vertexCount; i++) {
    base.push(new THREE.Vector3(boundary[i * 3], boundary[i * 3 + 1], boundary[i * 3 + 2]))
  }
  const offset = n.clone().multiplyScalar(distance)
  const top = base.map((p) => p.clone().add(offset))

  const capTriangles = triangulatePolygon3D(base, n)
  if (capTriangles.length === 0) return null

  const positions: number[] = []
  const pushTri = (a: THREE.Vector3, b: THREE.Vector3, c: THREE.Vector3) => {
    positions.push(a.x, a.y, a.z, b.x, b.y, b.z, c.x, c.y, c.z)
  }

  // Base cap — winding flipped so its outward normal faces away from the
  // sweep direction (matches the top cap's natural winding facing +normal).
  for (const [i0, i1, i2] of capTriangles) {
    pushTri(base[i0], base[i2], base[i1])
  }
  // Top cap.
  for (const [i0, i1, i2] of capTriangles) {
    pushTri(top[i0], top[i1], top[i2])
  }
  // Side walls — one quad (two triangles) per boundary edge.
  for (let i = 0; i < vertexCount; i++) {
    const j = (i + 1) % vertexCount
    pushTri(base[i], base[j], top[j])
    pushTri(base[i], top[j], top[i])
  }

  const geo = new THREE.BufferGeometry()
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(positions), 3))
  geo.computeVertexNormals()

  const mat = new THREE.MeshBasicMaterial({
    color: distance > 0 ? SWEPT_PRISM_ADD_COLOR : SWEPT_PRISM_CUT_COLOR,
    transparent: true,
    opacity: 0.45,
    depthWrite: false,
    depthTest: false,
    side: THREE.DoubleSide,
  })

  const mesh = new THREE.Mesh(geo, mat)
  mesh.renderOrder = 996
  return mesh
}

/**
 * Dispose all geometries and materials owned by the preview group, then
 * clear it.  Safe to call even when the group is empty.
 */
export function clearPreview(previewGroup: THREE.Group): void {
  const disposeMaterial = (m: THREE.Material | THREE.Material[]) => {
    if (Array.isArray(m)) m.forEach((mm) => mm.dispose())
    else if (m instanceof THREE.Material) m.dispose()
  }
  previewGroup.traverse((child) => {
    if (child instanceof THREE.Mesh) {
      child.geometry.dispose()
      disposeMaterial(child.material)
    }
    if (child instanceof THREE.LineSegments) {
      child.geometry.dispose()
      disposeMaterial(child.material)
    }
  })
  previewGroup.clear()
}
