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
})
