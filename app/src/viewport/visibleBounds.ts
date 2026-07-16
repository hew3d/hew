/**
 * visibleBounds.ts — visibility-aware world bounding boxes.
 *
 * `THREE.Box3.expandByObject` ignores `.visible`, but hidden geometry
 * (eye-hidden nodes, hidden-by-tag content) stays attached to the scene
 * graph with its wrapper group's `visible` flag flipped off
 * (`SceneRenderer._applyHidden`). Building a fit box with the naive expand
 * therefore frames geometry the user cannot see — Zoom Extents and the
 * standard views promise to frame "every visible thing"
 * (site/src/content/learn/viewing.md), so they traverse with this helper
 * instead and prune invisible subtrees exactly as the renderer does.
 */

import * as THREE from 'three'

// Scratch box reused across calls (this runs per camera command, never
// concurrently), so the traversal allocates nothing per node.
const _geomBox = new THREE.Box3()

/**
 * Expand `box` by every **visible** geometry-bearing descendant of `root`
 * (including `root` itself). A node with `visible === false` prunes its
 * whole subtree — matching what the renderer actually draws. Returns `box`.
 */
export function expandByVisibleObject(box: THREE.Box3, root: THREE.Object3D): THREE.Box3 {
  // One world-matrix refresh for the whole subtree up front; the traversal
  // below then reads each node's matrixWorld directly.
  root.updateWorldMatrix(true, true)
  visit(box, root)
  return box
}

/** The object-level bounds surface `THREE.InstancedMesh` (and SkinnedMesh)
 * carries; plain meshes/lines leave `boundingBox` undefined. */
type WithObjectBounds = THREE.Object3D & {
  boundingBox?: THREE.Box3 | null
  computeBoundingBox?: () => void
}

function visit(box: THREE.Box3, node: THREE.Object3D): void {
  if (!node.visible) return
  const geometry = (node as Partial<THREE.Mesh>).geometry
  if (geometry !== undefined) {
    // Mirror Box3.expandByObject's object-level bounds branch: an
    // InstancedMesh carries its placements in the instanceMatrix attribute,
    // not in matrixWorld, so its OBJECT-level boundingBox — three's stock
    // computeBoundingBox unions the geometry bounds across every instance
    // matrix, and SceneRenderer's batch override additionally skips
    // suppressed (hidden/materialized) slots — is the truth. The geometry
    // bbox alone would frame only the definition at the origin and miss
    // every placed component instance. (The batch edge LineSegments needs
    // no special case: its geometry.computeBoundingBox already delegates
    // to the sibling face batch's instance-aware bounds.)
    const withObjectBounds = node as WithObjectBounds
    if (withObjectBounds.boundingBox !== undefined) {
      if (withObjectBounds.boundingBox === null) withObjectBounds.computeBoundingBox?.()
      const objectBox = withObjectBounds.boundingBox
      if (objectBox != null && !objectBox.isEmpty()) {
        _geomBox.copy(objectBox).applyMatrix4(node.matrixWorld)
        box.union(_geomBox)
      }
    } else {
      if (geometry.boundingBox === null) geometry.computeBoundingBox()
      if (geometry.boundingBox !== null && !geometry.boundingBox.isEmpty()) {
        _geomBox.copy(geometry.boundingBox).applyMatrix4(node.matrixWorld)
        box.union(_geomBox)
      }
    }
  }
  for (const child of node.children) visit(box, child)
}
