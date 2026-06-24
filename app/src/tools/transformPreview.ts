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
