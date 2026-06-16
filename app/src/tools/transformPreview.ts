/**
 * Shared helpers for transform-tool preview cloning and cleanup.
 *
 * `buildPreviewClone` deep-clones the rendered mesh for a given objectId and
 * gives the clone its OWN BufferGeometry instances (via geometry.clone()) so
 * that `clearPreview`'s geometry.dispose() calls cannot corrupt the live
 * scene object's shared geometry.
 */

import * as THREE from 'three'

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
      const mat = (child.material as THREE.MeshPhongMaterial).clone()
      mat.opacity = 0.5
      mat.transparent = true
      mat.depthWrite = false
      child.material = mat
    }
    if (child instanceof THREE.LineSegments) {
      // Clone geometry so dispose() on the preview does not corrupt the live object
      child.geometry = child.geometry.clone()
      const mat = (child.material as THREE.LineBasicMaterial).clone()
      mat.opacity = 0.5
      mat.transparent = true
      child.material = mat
    }
  })
  return clone
}

/**
 * Dispose all geometries and materials owned by the preview group, then
 * clear it.  Safe to call even when the group is empty.
 */
export function clearPreview(previewGroup: THREE.Group): void {
  previewGroup.traverse((child) => {
    if (child instanceof THREE.Mesh) {
      child.geometry.dispose()
      if (child.material instanceof THREE.Material) child.material.dispose()
    }
    if (child instanceof THREE.LineSegments) {
      child.geometry.dispose()
      if (child.material instanceof THREE.Material) child.material.dispose()
    }
  })
  previewGroup.clear()
}
