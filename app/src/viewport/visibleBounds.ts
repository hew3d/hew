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

function visit(box: THREE.Box3, node: THREE.Object3D): void {
  if (!node.visible) return
  const geometry = (node as Partial<THREE.Mesh>).geometry
  if (geometry !== undefined) {
    if (geometry.boundingBox === null) geometry.computeBoundingBox()
    if (geometry.boundingBox !== null && !geometry.boundingBox.isEmpty()) {
      _geomBox.copy(geometry.boundingBox).applyMatrix4(node.matrixWorld)
      box.union(_geomBox)
    }
  }
  for (const child of node.children) visit(box, child)
}
