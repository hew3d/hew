// Builds the translucent placement ghost for a library item from
// `item_ghost_mesh` — a FREE wasm function that tessellates the item's
// bytes directly (no scratch `Scene`, which would leak `Load` calls into
// the global session recording). One flattened mesh in item coordinates,
// origin at the item's own origin, plus a hard-edge stroke.

import * as THREE from 'three'
import { loadKernel } from '../wasm/loader'
import type { V3 } from '../viewport/geoHelpers'

export interface ItemGhost {
  /** Accent-tinted fill + stroke, in ITEM coordinates. Caller owns
   *  disposal (the placement tool routes it through `clearPreview`). */
  group: THREE.Group
  bboxMin: V3
  bboxMax: V3
}

const GHOST_FILL = 0x4a8df0
const GHOST_STROKE = 0x6aa8f5

/** Tessellate item bytes into a ready-to-mount ghost. Throws on bytes that
 *  don't load as a document (same typed errors as opening the item). */
export async function buildItemGhost(bytes: Uint8Array): Promise<ItemGhost> {
  await loadKernel()
  const { item_ghost_mesh } = await import('../wasm/pkg/wasm_api.js')
  const mesh = item_ghost_mesh(bytes)
  try {
    const group = new THREE.Group()

    const geom = new THREE.BufferGeometry()
    geom.setAttribute('position', new THREE.Float32BufferAttribute(mesh.positions(), 3))
    geom.setAttribute('normal', new THREE.Float32BufferAttribute(mesh.normals(), 3))
    geom.setIndex(new THREE.BufferAttribute(mesh.indices(), 1))
    const fill = new THREE.Mesh(
      geom,
      new THREE.MeshBasicMaterial({
        color: GHOST_FILL,
        transparent: true,
        opacity: 0.28,
        depthWrite: false,
        side: THREE.DoubleSide,
      }),
    )
    fill.renderOrder = 2
    group.add(fill)

    const edges = mesh.edge_positions()
    if (edges.length >= 6) {
      const edgeGeom = new THREE.BufferGeometry()
      edgeGeom.setAttribute('position', new THREE.Float32BufferAttribute(edges, 3))
      const stroke = new THREE.LineSegments(
        edgeGeom,
        new THREE.LineBasicMaterial({
          color: GHOST_STROKE,
          transparent: true,
          opacity: 0.9,
        }),
      )
      stroke.renderOrder = 2
      group.add(stroke)
    }

    const b = mesh.bbox()
    return {
      group,
      bboxMin: [b[0], b[1], b[2]],
      bboxMax: [b[3], b[4], b[5]],
    }
  } finally {
    mesh.free()
  }
}
