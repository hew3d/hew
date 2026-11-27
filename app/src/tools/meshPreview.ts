/**
 * meshPreview — a translucent ghost of a kernel preview mesh.
 *
 * Revolve and Deform preview the exact solid the kernel would build
 * (`revolve_preview`, `deform_preview`), not an approximation drawn in the
 * app. The result is a plain three.js group of one Mesh (the faces) and one
 * LineSegments (the hard edges), so the shared `clearPreview` disposes it.
 */

import * as THREE from 'three'

/** The slice of the wasm `MeshJs` this needs (so tests can pass a plain object). */
export interface PreviewMeshData {
  positions(): Float32Array
  normals(): Float32Array
  indices(): Uint32Array
  edge_positions(): Float32Array
  free(): void
}

const GHOST_OPACITY = 0.55
const GHOST_RENDER_ORDER = 996

/**
 * Build the ghost for `mesh` in `color` and free the wasm object. Returns
 * null for an empty mesh.
 */
export function buildMeshPreview(mesh: PreviewMeshData, color: number): THREE.Group | null {
  try {
    const positions = mesh.positions()
    const indices = mesh.indices()
    if (positions.length === 0 || indices.length === 0) return null

    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(positions.slice(), 3))
    const normals = mesh.normals()
    if (normals.length === positions.length) {
      geo.setAttribute('normal', new THREE.BufferAttribute(normals.slice(), 3))
    } else {
      geo.computeVertexNormals()
    }
    geo.setIndex(new THREE.BufferAttribute(indices.slice(), 1))

    const faceMat = new THREE.MeshLambertMaterial({
      color,
      transparent: true,
      opacity: GHOST_OPACITY,
      depthWrite: false,
      side: THREE.DoubleSide,
    })
    const faces = new THREE.Mesh(geo, faceMat)
    faces.renderOrder = GHOST_RENDER_ORDER

    const group = new THREE.Group()
    group.name = 'MeshPreview'
    group.add(faces)

    const edges = mesh.edge_positions()
    if (edges.length > 0) {
      const lineGeo = new THREE.BufferGeometry()
      lineGeo.setAttribute('position', new THREE.BufferAttribute(edges.slice(), 3))
      const lineMat = new THREE.LineBasicMaterial({ color, transparent: true, opacity: 0.9, depthTest: false })
      const lines = new THREE.LineSegments(lineGeo, lineMat)
      lines.renderOrder = GHOST_RENDER_ORDER + 1
      group.add(lines)
    }
    return group
  } finally {
    mesh.free()
  }
}
