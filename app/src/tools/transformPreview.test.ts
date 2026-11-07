import { describe, it, expect } from 'vitest'
import * as THREE from 'three'
import {
  buildInstanceMemberPreviewClone,
  buildInstancePreviewClone,
  buildMultiPreviewClone,
  buildPreviewClone,
  buildSketchPreviewClone,
  buildSweptPrismPreview,
  isSessionGhost,
  markSessionGhost,
} from './transformPreview'

/** Count vertices/triangles across all THREE.Mesh instances in an Object3D. */
function meshTriangleCount(obj: THREE.Object3D): number {
  let triangles = 0
  obj.traverse((child) => {
    if (child instanceof THREE.Mesh) {
      const pos = child.geometry.getAttribute('position')
      const indexCount = child.geometry.getIndex()?.count ?? pos.count
      triangles += indexCount / 3
    }
  })
  return triangles
}

describe('buildSweptPrismPreview', () => {
  it('builds a 12-triangle prism for a unit square swept by +1 along Z', () => {
    // Unit square in the XY plane, no duplicate closing vertex.
    const boundary = new Float32Array([
      0, 0, 0,
      1, 0, 0,
      1, 1, 0,
      0, 1, 0,
    ])
    const result = buildSweptPrismPreview(boundary, [0, 0, 1], 1)
    expect(result).not.toBeNull()
    // 2 caps x 2 tris + 4 walls x 2 tris = 12 tris
    expect(meshTriangleCount(result!)).toBe(12)
  })

  it('triangulates a non-convex L-shaped polygon without throwing', () => {
    // L-shape (6 vertices), in the XY plane, swept along +Z.
    const boundary = new Float32Array([
      0, 0, 0,
      2, 0, 0,
      2, 1, 0,
      1, 1, 0,
      1, 2, 0,
      0, 2, 0,
    ])
    let result: THREE.Object3D | null = null
    expect(() => {
      result = buildSweptPrismPreview(boundary, [0, 0, 1], 0.5)
    }).not.toThrow()
    expect(result).not.toBeNull()
    // 6-sided loop: cap triangulation yields 4 tris/cap (n-2), plus 6 walls x 2 tris.
    expect(meshTriangleCount(result!)).toBe(4 * 2 + 6 * 2)
  })

  it('returns null when distance is ~0', () => {
    const boundary = new Float32Array([
      0, 0, 0,
      1, 0, 0,
      1, 1, 0,
      0, 1, 0,
    ])
    expect(buildSweptPrismPreview(boundary, [0, 0, 1], 0)).toBeNull()
    expect(buildSweptPrismPreview(boundary, [0, 0, 1], 1e-9)).toBeNull()
  })

  it('returns null for a boundary with fewer than 3 vertices', () => {
    const boundary = new Float32Array([0, 0, 0, 1, 0, 0])
    expect(buildSweptPrismPreview(boundary, [0, 0, 1], 1)).toBeNull()
  })

  it('returns null for an empty boundary', () => {
    expect(buildSweptPrismPreview(new Float32Array([]), [0, 0, 1], 1)).toBeNull()
  })

  it('tints additive (distance > 0) and cut (distance < 0) sweeps differently', () => {
    const boundary = new Float32Array([
      0, 0, 0,
      1, 0, 0,
      1, 1, 0,
      0, 1, 0,
    ])
    const addPrism = buildSweptPrismPreview(boundary, [0, 0, 1], 1) as THREE.Mesh
    const cutPrism = buildSweptPrismPreview(boundary, [0, 0, 1], -1) as THREE.Mesh
    const addColor = (addPrism.material as THREE.MeshBasicMaterial).color.getHex()
    const cutColor = (cutPrism.material as THREE.MeshBasicMaterial).color.getHex()
    expect(addColor).not.toBe(cutColor)
  })
})

describe('buildPreviewClone — drag ghosts are never section-clipped', () => {
  it('strips clippingPlanes from the faded clone material so the ghost is not frozen at the grab-time cut', () => {
    // A live object whose material carries an active section clip.
    const objectsGroup = new THREE.Group()
    const sub = new THREE.Group()
    sub.name = 'Object_5'
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]), 3))
    const mat = new THREE.MeshBasicMaterial()
    mat.clippingPlanes = [new THREE.Plane(new THREE.Vector3(0, 0, 1), 0)]
    sub.add(new THREE.Mesh(geo, mat))
    objectsGroup.add(sub)

    const clone = buildPreviewClone(objectsGroup, 5n)
    expect(clone).not.toBeNull()
    let cloneMat: THREE.Material | undefined
    clone!.traverse((c) => {
      if (c instanceof THREE.Mesh) cloneMat = c.material as THREE.Material
    })
    expect(cloneMat).toBeDefined()
    // Overlay-like: the ghost must not clip (a clone would otherwise freeze
    // the grab-time plane snapshot).
    expect(cloneMat!.clippingPlanes).toBeNull()
    // The live source material is untouched.
    expect(mat.clippingPlanes).toHaveLength(1)
  })
})

describe('every preview builder marks its root as session geometry (the sibling preview mirror reads the mark)', () => {
  function objectsGroupWith(id: bigint): THREE.Group {
    const objectsGroup = new THREE.Group()
    const sub = new THREE.Group()
    sub.name = `Object_${id}`
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]), 3))
    sub.add(new THREE.Mesh(geo, new THREE.MeshBasicMaterial()))
    objectsGroup.add(sub)
    return objectsGroup
  }
  function instanceGroup(instanceId: bigint, memberId: bigint): THREE.Group {
    const g = new THREE.Group()
    g.matrixAutoUpdate = false
    g.matrix.makeTranslation(3, 0, 0)
    const face = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), new THREE.MeshBasicMaterial())
    face.name = `InstanceFace_${instanceId}_${memberId}_0`
    g.add(face)
    return g
  }

  it('markSessionGhost / isSessionGhost round-trip on the root only', () => {
    const root = new THREE.Group()
    const child = new THREE.Mesh()
    root.add(child)
    expect(isSessionGhost(root)).toBe(false)
    expect(markSessionGhost(root)).toBe(root)
    expect(isSessionGhost(root)).toBe(true)
    expect(isSessionGhost(child)).toBe(false)
  })

  it('buildPreviewClone (Move/Rotate/Scale on a world object)', () => {
    const clone = buildPreviewClone(objectsGroupWith(5n), 5n)
    expect(clone).not.toBeNull()
    expect(isSessionGhost(clone!)).toBe(true)
  })

  it('buildMultiPreviewClone (a group’s leaves)', () => {
    const clone = buildMultiPreviewClone(objectsGroupWith(5n), [5n], [instanceGroup(9n, 1n)])
    expect(clone).not.toBeNull()
    expect(isSessionGhost(clone!)).toBe(true)
  })

  it('buildInstancePreviewClone (a component instance)', () => {
    const clone = buildInstancePreviewClone(instanceGroup(9n, 1n))
    expect(clone).not.toBeNull()
    expect(isSessionGhost(clone!)).toBe(true)
  })

  it('buildInstanceMemberPreviewClone (one member through its instance)', () => {
    const clone = buildInstanceMemberPreviewClone(instanceGroup(9n, 1n), 9n, 1n)
    expect(clone).not.toBeNull()
    expect(isSessionGhost(clone!)).toBe(true)
  })

  it('buildSketchPreviewClone (a sketch’s line work)', () => {
    const clone = buildSketchPreviewClone([0, 0, 0, 1, 0, 0])
    expect(clone).not.toBeNull()
    expect(isSessionGhost(clone!)).toBe(true)
  })

  it('buildSweptPrismPreview (Push/Pull)', () => {
    const prism = buildSweptPrismPreview([0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0], [0, 0, 1], 1)
    expect(prism).not.toBeNull()
    expect(isSessionGhost(prism!)).toBe(true)
  })
})
