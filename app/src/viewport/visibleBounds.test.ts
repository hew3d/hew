import { describe, it, expect } from 'vitest'
import * as THREE from 'three'
import { expandByVisibleObject } from './visibleBounds'

/**
 * Zoom Extents / standard views must frame only what the user can see.
 * `Box3.expandByObject` ignores `.visible`, and hidden-by-tag / eye-hidden
 * geometry stays attached to the scene graph with `group.visible = false`
 * (SceneRenderer._applyHidden) — so a naive expand frames invisible
 * geometry, contradicting the docs' promise ("frames every visible thing").
 * These tests pin the visibility-aware traversal both fixes now share.
 */

function meshAt(x: number, y: number, z: number): THREE.Mesh {
  const mesh = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1))
  mesh.position.set(x, y, z)
  return mesh
}

describe('expandByVisibleObject', () => {
  it('includes visible geometry (parity with expandByObject)', () => {
    const root = new THREE.Group()
    root.add(meshAt(0, 0, 0))
    root.add(meshAt(3, 0, 0))

    const box = expandByVisibleObject(new THREE.Box3(), root)
    expect(box.isEmpty()).toBe(false)
    expect(box.min.x).toBeCloseTo(-0.5)
    expect(box.max.x).toBeCloseTo(3.5)
  })

  it('skips a hidden mesh — a far-away hidden object must not blow out the framing', () => {
    const root = new THREE.Group()
    root.add(meshAt(0, 0, 0))
    const hidden = meshAt(1000, 0, 0) // hidden-by-tag solid, far away
    hidden.visible = false
    root.add(hidden)

    const box = expandByVisibleObject(new THREE.Box3(), root)
    expect(box.isEmpty()).toBe(false)
    expect(box.max.x).toBeCloseTo(0.5) // ONLY the visible cube is framed
  })

  it('prunes a whole invisible subtree (the renderer hides per-object groups, not meshes)', () => {
    const root = new THREE.Group()
    // SceneRenderer wraps each object in its own group and flips
    // group.visible; the meshes inside stay individually visible.
    const objectGroup = new THREE.Group()
    objectGroup.add(meshAt(500, 500, 0))
    objectGroup.visible = false
    root.add(objectGroup)
    root.add(meshAt(1, 1, 1))

    const box = expandByVisibleObject(new THREE.Box3(), root)
    expect(box.max.x).toBeCloseTo(1.5)
    expect(box.max.y).toBeCloseTo(1.5)
  })

  it('respects world transforms of visible ancestors', () => {
    const root = new THREE.Group()
    const child = new THREE.Group()
    child.position.set(10, 0, 0)
    child.add(meshAt(0, 0, 0))
    root.add(child)

    const box = expandByVisibleObject(new THREE.Box3(), root)
    expect(box.min.x).toBeCloseTo(9.5)
    expect(box.max.x).toBeCloseTo(10.5)
  })

  it('an invisible root contributes nothing; an empty box stays empty', () => {
    const root = new THREE.Group()
    root.add(meshAt(0, 0, 0))
    root.visible = false

    const box = expandByVisibleObject(new THREE.Box3(), root)
    expect(box.isEmpty()).toBe(true)
  })

  // ------------------------------------------------------------------
  // Instanced batches: component placements live in the instanceMatrix
  // attribute, NOT in matrixWorld — the geometry bbox alone covers only
  // the definition at the origin. The traversal must use the OBJECT-level
  // InstancedMesh bounds (three's stock computeBoundingBox unions the
  // geometry bounds across every instance matrix; SceneRenderer's batch
  // override additionally skips suppressed slots).
  // ------------------------------------------------------------------

  function instancedAt(positions: [number, number, number][]): THREE.InstancedMesh {
    const im = new THREE.InstancedMesh(
      new THREE.BoxGeometry(1, 1, 1),
      new THREE.MeshBasicMaterial(),
      positions.length,
    )
    const m = new THREE.Matrix4()
    positions.forEach((p, i) => im.setMatrixAt(i, m.makeTranslation(p[0], p[1], p[2])))
    return im
  }

  it('frames every placement of an InstancedMesh, not just the definition at origin', () => {
    const root = new THREE.Group()
    // One placement at the origin, one far away — a placed component
    // instance must be framed by Zoom Extents.
    root.add(instancedAt([[0, 0, 0], [100, 0, 0]]))

    const box = expandByVisibleObject(new THREE.Box3(), root)
    expect(box.min.x).toBeCloseTo(-0.5)
    expect(box.max.x).toBeCloseTo(100.5) // NOT 0.5: the distant instance counts
  })

  it('the mixed case: a plain object plus a distant instance frames both', () => {
    const root = new THREE.Group()
    root.add(meshAt(0, 0, 0))
    root.add(instancedAt([[50, 20, 0]]))

    const box = expandByVisibleObject(new THREE.Box3(), root)
    expect(box.min.x).toBeCloseTo(-0.5)
    expect(box.max.x).toBeCloseTo(50.5)
    expect(box.max.y).toBeCloseTo(20.5)
  })

  it('a hidden InstancedMesh batch contributes nothing', () => {
    const root = new THREE.Group()
    root.add(meshAt(0, 0, 0))
    const im = instancedAt([[100, 0, 0]])
    im.visible = false
    root.add(im)

    const box = expandByVisibleObject(new THREE.Box3(), root)
    expect(box.max.x).toBeCloseTo(0.5)
  })

  it('honors an object-level computeBoundingBox override (the renderer skips suppressed slots)', () => {
    const root = new THREE.Group()
    const im = instancedAt([[0, 0, 0], [500, 0, 0]])
    // SceneRenderer._buildBatch overrides computeBoundingBox to skip
    // suppressed (hidden/materialized) slots; the traversal must consume
    // the override's result rather than recompute from raw attributes.
    im.computeBoundingBox = () => {
      im.boundingBox ??= new THREE.Box3()
      im.boundingBox.set(new THREE.Vector3(-0.5, -0.5, -0.5), new THREE.Vector3(0.5, 0.5, 0.5))
    }
    root.add(im)

    const box = expandByVisibleObject(new THREE.Box3(), root)
    expect(box.max.x).toBeCloseTo(0.5) // the suppressed far slot stays out
  })
})
