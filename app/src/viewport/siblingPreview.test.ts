import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import {
  SiblingPreviewMirror,
  affineToMatrix4,
  collectSessionGhosts,
  snapshotGhosts,
  SIBLING_PREVIEW_CAP,
  SIBLING_PREVIEW_NODE_BUDGET,
} from './siblingPreview'
import { buildSweptPrismPreview, clearPreview, markSessionGhost } from '../tools/transformPreview'

/** Row-major 3×4 translation by (x, y, z). */
const shift = (x: number, y: number, z: number): number[] => [1, 0, 0, x, 0, 1, 0, y, 0, 0, 1, z]

/** A unit square on the ground swept up `distance` — the Push/Pull ghost shape. */
function prism(distance = 1): THREE.Mesh {
  const p = buildSweptPrismPreview([0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0], [0, 0, 1], distance)
  if (p === null) throw new Error('degenerate prism')
  return p as THREE.Mesh
}

/** A marked ghost root holding `n` children, each a Group over a Mesh and a
 *  LineSegments — the shape of a multi-select Move ghost (3 nodes per object). */
function bigGhost(n: number): THREE.Group {
  const root = markSessionGhost(new THREE.Group())
  for (let i = 0; i < n; i++) {
    const g = new THREE.Group()
    g.add(new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1)), new THREE.LineSegments(new THREE.BufferGeometry()))
    g.position.set(i, 0, 0)
    root.add(g)
  }
  return root
}

function countNodes(obj: THREE.Object3D): number {
  let n = 0
  obj.traverse(() => n++)
  return n
}

function setup(opts: { frames?: number[]; cap?: number; nodeBudget?: number; fingerprint?: () => string } = {}) {
  const source = new THREE.Group()
  const root = new THREE.Group()
  const read = vi.fn(() => opts.frames ?? [...shift(4, 0, 0), ...shift(0, 7, 0)])
  const mirror = new SiblingPreviewMirror({
    source,
    root,
    frames: read,
    cap: opts.cap,
    nodeBudget: opts.nodeBudget,
    fingerprint: opts.fingerprint,
  })
  return { source, root, read, mirror }
}

const firstClone = (root: THREE.Group, frame = 0): THREE.Object3D => (root.children[frame] as THREE.Group).children[0]

describe('affineToMatrix4', () => {
  it('reads a row-major 3×4 affine at an offset into a three.js matrix', () => {
    const m = affineToMatrix4([...shift(0, 0, 0), 2, 0, 0, 5, 0, 3, 0, 6, 0, 0, 4, 7], 12)
    const p = new THREE.Vector3(1, 1, 1).applyMatrix4(m)
    expect([p.x, p.y, p.z]).toEqual([7, 9, 11])
  })
})

describe('collectSessionGhosts', () => {
  it('lists marked roots whole and skips unmarked objects', () => {
    const group = new THREE.Group()
    const ghost = markSessionGhost(new THREE.Group())
    ghost.add(markSessionGhost(new THREE.Mesh())) // a marked descendant is part of its root
    const cue = new THREE.LineSegments()
    const nested = new THREE.Group()
    const inner = markSessionGhost(new THREE.Mesh())
    nested.add(inner)
    group.add(ghost, cue, nested)
    expect(collectSessionGhosts(group)).toEqual([ghost, inner])
  })
})

describe('snapshotGhosts', () => {
  it('counts every node and changes with identity, geometry, material, transform and visibility', () => {
    const ghost = bigGhost(2)
    const a = snapshotGhosts([ghost])
    expect(a.count).toBe(1 + 2 * 3)
    expect(snapshotGhosts([ghost]).stamp).toEqual(a.stamp)
    ghost.position.x = 1
    const b = snapshotGhosts([ghost]).stamp
    expect(b).not.toEqual(a.stamp)
    const mesh = ghost.children[0].children[0] as THREE.Mesh
    mesh.geometry = new THREE.BoxGeometry(2, 2, 2)
    const c = snapshotGhosts([ghost]).stamp
    expect(c).not.toEqual(b)
    mesh.visible = false
    expect(snapshotGhosts([ghost]).stamp).not.toEqual(c)
  })
})

describe('SiblingPreviewMirror', () => {
  it('draws nothing while the preview is empty and never asks for frames', () => {
    const { root, read, mirror } = setup()
    mirror.sync()
    mirror.sync()
    expect(root.children).toHaveLength(0)
    expect(mirror.mirrorCount).toBe(0)
    expect(mirror.lastWork).toBe('idle')
    expect(read).not.toHaveBeenCalled()
  })

  it('mirrors a ghost once per frame, through that frame, sharing its geometry and material', () => {
    const { source, root, mirror } = setup()
    const ghost = prism()
    source.add(ghost)
    mirror.sync()

    expect(mirror.lastWork).toBe('rebuild')
    expect(mirror.mirrorCount).toBe(2)
    expect(root.children).toHaveLength(2)
    const [a, b] = root.children as THREE.Group[]
    expect(a.matrixAutoUpdate).toBe(false)
    expect(new THREE.Vector3().setFromMatrixPosition(a.matrix).toArray()).toEqual([4, 0, 0])
    expect(new THREE.Vector3().setFromMatrixPosition(b.matrix).toArray()).toEqual([0, 7, 0])
    for (const m of [a, b]) {
      expect(m.children).toHaveLength(1)
      const clone = m.children[0] as THREE.Mesh
      expect(clone).toBeInstanceOf(THREE.Mesh)
      expect(clone.geometry).toBe(ghost.geometry)
      expect(clone.material).toBe(ghost.material)
      expect(clone.renderOrder).toBe(ghost.renderOrder)
    }
    // The clone really lands where the sibling is: the prism's top corner
    // (1, 1, 1) shows at (5, 1, 1) on the first sibling.
    root.updateMatrixWorld(true)
    const top = new THREE.Vector3(1, 1, 1).applyMatrix4(a.children[0].matrixWorld)
    expect(top.toArray()).toEqual([5, 1, 1])
  })

  it('ignores preview objects that are not marked as session geometry', () => {
    const { source, root, read, mirror } = setup()
    source.add(new THREE.LineSegments()) // a cue, a leader, a marker
    mirror.sync()
    expect(root.children).toHaveLength(0)
    expect(read).not.toHaveBeenCalled()
    source.add(prism())
    mirror.sync()
    expect(mirror.mirrorCount).toBe(2)
    expect((root.children[0] as THREE.Group).children).toHaveLength(1) // the prism only
  })

  it('a sync with nothing changed does no work', () => {
    const { source, mirror } = setup()
    source.add(prism())
    mirror.sync()
    expect(mirror.lastWork).toBe('rebuild')
    mirror.sync()
    expect(mirror.lastWork).toBe('skip')
    mirror.sync()
    expect(mirror.lastWork).toBe('skip')
  })

  it('Push/Pull’s fresh prism per move RETARGETS the existing clones (same objects, new geometry) and reads frames once', () => {
    const { source, root, read, mirror } = setup()
    source.add(prism(1))
    mirror.sync()
    expect(read).toHaveBeenCalledTimes(1)
    const clone = firstClone(root) as THREE.Mesh

    clearPreview(source)
    const next = prism(2)
    source.add(next)
    mirror.sync()
    expect(mirror.lastWork).toBe('retarget')
    expect(read).toHaveBeenCalledTimes(1)
    expect(mirror.mirrorCount).toBe(2)
    expect(firstClone(root)).toBe(clone) // the same clone object…
    expect(clone.geometry).toBe(next.geometry) // …now drawing the new prism
    expect(clone.material).toBe(next.material)
    expect((firstClone(root, 1) as THREE.Mesh).geometry).toBe(next.geometry)
  })

  it('follows a driven ghost: a Move/Rotate/Scale ghost moved by its transform moves every clone, in place', () => {
    const { source, root, mirror } = setup()
    const ghost = markSessionGhost(new THREE.Group())
    const posed = new THREE.Mesh()
    posed.matrixAutoUpdate = false
    posed.matrix.makeTranslation(0, 0, 2)
    ghost.add(posed)
    source.add(ghost)
    mirror.sync()
    const clone = firstClone(root, 1)

    ghost.position.set(1, 2, 3)
    ghost.quaternion.setFromAxisAngle(new THREE.Vector3(0, 0, 1), Math.PI / 2)
    ghost.visible = false
    posed.matrix.makeTranslation(0, 0, 5)
    mirror.sync()
    expect(mirror.lastWork).toBe('retarget')
    expect(firstClone(root, 1)).toBe(clone) // kept, not rebuilt
    expect(clone.position.toArray()).toEqual([1, 2, 3])
    expect(clone.quaternion.equals(ghost.quaternion)).toBe(true)
    expect(clone.visible).toBe(false)
    expect(clone.children[0].matrixAutoUpdate).toBe(false)
    expect(new THREE.Vector3().setFromMatrixPosition(clone.children[0].matrix).toArray()).toEqual([0, 0, 5])
    // Its world position composes the sibling frame (0, 7, 0) with the drive.
    ghost.visible = true
    mirror.sync()
    root.updateMatrixWorld(true)
    expect(new THREE.Vector3().setFromMatrixPosition(clone.matrixWorld).toArray()).toEqual([1, 9, 3])
  })

  it('a child swapped in place under the same root (same count) is picked up: the clone draws the new geometry', () => {
    const { source, root, mirror } = setup()
    const ghost = markSessionGhost(new THREE.Group())
    const geoA = new THREE.BoxGeometry(1, 1, 1)
    ghost.add(new THREE.Mesh(geoA))
    source.add(ghost)
    mirror.sync()
    expect((firstClone(root).children[0] as THREE.Mesh).geometry).toBe(geoA)

    ghost.remove(ghost.children[0])
    const geoB = new THREE.BoxGeometry(2, 2, 2)
    ghost.add(new THREE.Mesh(geoB))
    mirror.sync()
    expect(mirror.lastWork).toBe('retarget')
    expect((firstClone(root).children[0] as THREE.Mesh).geometry).toBe(geoB)
  })

  it('rebuilds when a ghost subtree changes shape under the same root', () => {
    const { source, root, mirror } = setup()
    const ghost = markSessionGhost(new THREE.Group())
    ghost.add(new THREE.Mesh())
    source.add(ghost)
    mirror.sync()
    const before = firstClone(root)
    expect(before.children).toHaveLength(1)
    ghost.add(new THREE.Mesh()) // an array copy joined the ghost
    mirror.sync()
    expect(mirror.lastWork).toBe('rebuild')
    const after = firstClone(root)
    expect(after).not.toBe(before)
    expect(after.children).toHaveLength(2)
    // A node of another type in the same slot rebuilds too.
    ghost.remove(ghost.children[0])
    ghost.add(new THREE.LineSegments())
    mirror.sync()
    expect(mirror.lastWork).toBe('rebuild')
    expect(firstClone(root).children[1]).toBeInstanceOf(THREE.LineSegments)
  })

  it('ends with the gesture: an emptied preview drops every mirror and the next gesture reads frames afresh', () => {
    const { source, root, read, mirror } = setup()
    source.add(prism())
    mirror.sync()
    expect(mirror.mirrorCount).toBe(2)

    clearPreview(source) // commit, cancel, Escape, tool switch
    mirror.sync()
    expect(root.children).toHaveLength(0)
    expect(mirror.mirrorCount).toBe(0)
    expect(mirror.lastWork).toBe('idle')

    // The next gesture may run in another session (or none): frames are
    // asked for again.
    source.add(prism())
    mirror.sync()
    expect(read).toHaveBeenCalledTimes(2)
    expect(mirror.mirrorCount).toBe(2)
  })

  it('draws nothing outside a session (no frames) and does not keep asking during the gesture', () => {
    const { source, root, read, mirror } = setup({ frames: [] })
    source.add(prism())
    mirror.sync()
    mirror.sync()
    expect(root.children).toHaveLength(0)
    expect(mirror.mirrorCount).toBe(0)
    expect(mirror.lastWork).toBe('none')
    expect(read).toHaveBeenCalledTimes(1)
  })

  it('re-reads the frames mid-gesture when the session fingerprint changes, and drops the mirrors when the session is gone', () => {
    let fp = 'inst-1|a,b'
    let frames: number[] = [...shift(4, 0, 0), ...shift(0, 7, 0)]
    const source = new THREE.Group()
    const root = new THREE.Group()
    const read = vi.fn(() => frames)
    const mirror = new SiblingPreviewMirror({ source, root, frames: read, fingerprint: () => fp })
    source.add(prism())
    mirror.sync()
    expect(mirror.mirrorCount).toBe(2)
    mirror.sync()
    expect(read).toHaveBeenCalledTimes(1)

    // A placement was deleted under the gesture (undo): one frame left.
    fp = 'inst-1|a'
    frames = [...shift(4, 0, 0)]
    mirror.sync()
    expect(read).toHaveBeenCalledTimes(2)
    expect(mirror.lastWork).toBe('rebuild')
    expect(mirror.mirrorCount).toBe(1)

    // The session closed under the gesture: nothing to mirror.
    fp = '|'
    frames = []
    mirror.sync()
    expect(read).toHaveBeenCalledTimes(3)
    expect(mirror.mirrorCount).toBe(0)
    expect(root.children).toHaveLength(0)
    expect(mirror.lastWork).toBe('none')
  })

  it('invalidate() re-reads the frames on the next sync (a visibility or isolation change) and rebuilds on them', () => {
    let frames: number[] = [...shift(4, 0, 0), ...shift(0, 7, 0)]
    const source = new THREE.Group()
    const root = new THREE.Group()
    const read = vi.fn(() => frames)
    const mirror = new SiblingPreviewMirror({ source, root, frames: read })
    source.add(prism())
    mirror.sync()
    expect(mirror.mirrorCount).toBe(2)

    frames = [...shift(0, 7, 0)] // the first sibling was hidden
    mirror.sync()
    expect(mirror.mirrorCount).toBe(2) // not seen yet…
    mirror.invalidate()
    mirror.sync()
    expect(read).toHaveBeenCalledTimes(2)
    expect(mirror.mirrorCount).toBe(1)
    expect(new THREE.Vector3().setFromMatrixPosition((root.children[0] as THREE.Group).matrix).toArray()).toEqual([0, 7, 0])
  })

  it('caps the mirrored frames at the first N in kernel order', () => {
    const frames: number[] = []
    for (let k = 0; k < SIBLING_PREVIEW_CAP + 10; k++) frames.push(...shift(k, 0, 0))
    const { source, mirror } = setup({ frames })
    source.add(prism())
    mirror.sync()
    expect(mirror.mirrorCount).toBe(SIBLING_PREVIEW_CAP)

    const small = setup({ frames, cap: 3 })
    small.source.add(prism())
    small.mirror.sync()
    expect(small.mirror.mirrorCount).toBe(3)
    const positions = (small.root.children as THREE.Group[]).map((m) => new THREE.Vector3().setFromMatrixPosition(m.matrix).x)
    expect(positions).toEqual([0, 1, 2])
  })

  it('the node budget bounds the total mirrored objects: a big ghost gets fewer full mirrors', () => {
    const frames: number[] = []
    for (let k = 0; k < 10; k++) frames.push(...shift(k, 0, 0))
    const ghost = bigGhost(20) // 61 nodes
    const nodes = countNodes(ghost)
    expect(nodes).toBe(61)
    const { source, root, mirror } = setup({ frames, nodeBudget: 200 })
    source.add(ghost)
    mirror.sync()
    expect(mirror.lastWork).toBe('rebuild')
    expect(mirror.isProxy).toBe(false)
    expect(mirror.mirrorCount).toBe(3) // floor(200 / 61)
    let total = 0
    for (const m of root.children) total += countNodes(m) - 1 // the frame group itself is not a clone
    expect(total).toBe(3 * nodes)
    expect(total).toBeLessThanOrEqual(200)
    // The default budget admits the full cap for a small ghost…
    expect(Math.floor(SIBLING_PREVIEW_NODE_BUDGET / countNodes(prism()))).toBeGreaterThanOrEqual(SIBLING_PREVIEW_CAP)
    // …and a budget-sized mirror set is retargeted in place on the next move.
    ghost.position.x = 5
    mirror.sync()
    expect(mirror.lastWork).toBe('retarget')
    expect(mirror.mirrorCount).toBe(3)
  })

  it('a ghost too big for even one mirror is shown as a bounds box per frame, owned and disposed by the mirror', () => {
    const frames: number[] = [...shift(4, 0, 0), ...shift(0, 7, 0)]
    const ghost = bigGhost(20) // 61 nodes > budget 50
    const { source, root, mirror } = setup({ frames, nodeBudget: 50 })
    source.add(ghost)
    mirror.sync()
    expect(mirror.lastWork).toBe('proxy')
    expect(mirror.isProxy).toBe(true)
    expect(mirror.mirrorCount).toBe(2)
    const boxes = root.children.map((m) => (m as THREE.Group).children[0] as THREE.LineSegments)
    for (const box of boxes) {
      expect(box).toBeInstanceOf(THREE.LineSegments)
      expect((box.parent as THREE.Group).children).toHaveLength(1)
    }
    // The box spans the ghost's bounds: unit cubes centered at x = 0…19.
    expect(boxes[0].position.x).toBeCloseTo(9.5)
    expect(boxes[0].scale.x).toBeCloseTo(20)
    expect(boxes[0].scale.y).toBeCloseTo(1)
    expect(boxes[0].scale.z).toBeCloseTo(1)
    // Both frames share the one box geometry; it is the mirror's own, and
    // end() disposes it.
    expect(boxes[1].geometry).toBe(boxes[0].geometry)
    const dispose = vi.spyOn(boxes[0].geometry, 'dispose')
    mirror.end()
    expect(dispose).toHaveBeenCalledTimes(1)
    expect(mirror.mirrorCount).toBe(0)
  })

  it('leaves the ghost’s geometry alive when it ends: clones share, they do not own', () => {
    const { source, mirror } = setup()
    const ghost = prism()
    const dispose = vi.spyOn(ghost.geometry, 'dispose')
    source.add(ghost)
    mirror.sync()
    mirror.end()
    expect(dispose).not.toHaveBeenCalled()
    expect(mirror.mirrorCount).toBe(0)
  })
})
